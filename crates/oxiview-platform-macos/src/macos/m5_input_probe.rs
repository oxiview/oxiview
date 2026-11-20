use super::{MacOsPlatform, Native, boundary, failure, find, native_time, seconds};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSResponder, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSString, NSTimer};
use oxiview_base::Size;
use oxiview_platform::{
    Error, KeyCode, KeyModifiers, Platform, SystemAppearance, SystemPreferences, WindowAttributes,
    WindowEndpoint, WindowEvent, WindowId,
};
use std::{
    ptr::{self, NonNull},
    sync::{Mutex, mpsc},
    thread,
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(5);

/// Runs the opt-in M5 keyboard and preference smoke check on the process main
/// thread. Opens one window and returns its initial preferences after it closes.
/// The probe has a five-second deadline. Use the `m5_input_probe` example for
/// the additional process watchdog.
pub fn diagnostic_m5_input_probe() -> Result<SystemPreferences, Error> {
    let mtm = MainThreadMarker::new().ok_or(Error::MainThreadRequired)?;
    let deadline = native_time()
        .checked_add(TIMEOUT)
        .ok_or_else(|| failure("M5 input probe deadline overflow"))?;
    let mut worker = None;
    let mut timer = None;
    let result = MacOsPlatform::default().run(|context| {
        let window = context.create_window(WindowAttributes {
            title: "oxiview M5 input probe".into(),
            logical_size: Size::new(360.0, 160.0),
            // The probe acquires no rendering surface. Let resize tokens expire.
            resize_timeout: Duration::from_millis(100),
        })?;
        let id = window.main.id();
        let (send, receive) = mpsc::channel();
        let mut endpoint = window.endpoint;
        worker = Some(
            thread::Builder::new()
                .name("m5-input-probe-window".into())
                .spawn(move || check_window(&mut *endpoint, receive, deadline))
                .map_err(|error| Error::OperationFailed(error.to_string()))?,
        );
        timer = Some(schedule_playback(mtm, id, send));
        Ok(())
    });
    if let Some(timer) = timer {
        timer.invalidate();
    }
    let outcome = worker.map(thread::JoinHandle::join);
    result?;
    outcome
        .ok_or_else(|| failure("M5 input probe did not start"))?
        .map_err(|_| failure("M5 input probe thread panicked"))?
}

struct Expectations {
    keys: Vec<WindowEvent>,
    appearances: [SystemAppearance; 2],
}

enum Playback {
    WaitingForKeyWindow,
    WaitingForFlip(SystemAppearance),
    Finished,
}

fn schedule_playback(
    _mtm: MainThreadMarker,
    window_id: WindowId,
    send: mpsc::Sender<Result<Expectations, Error>>,
) -> Retained<NSTimer> {
    let playback = Mutex::new(Playback::WaitingForKeyWindow);
    let tick = RcBlock::new(move |_: NonNull<NSTimer>| {
        boundary(|| {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let Ok(mut playback) = playback.try_lock() else {
                return;
            };
            if let Some(window) = find(window_id).filter(|window| !window.closed.get())
                && let Err(error) = playback.advance(&window, mtm, &send)
            {
                *playback = Playback::Finished;
                let _ = send.send(Err(error));
            }
        });
    });
    // SAFETY: The marker requires the main thread. The timer copies a block
    // capturing only a window ID, a mutex over owned state and a channel.
    // Scheduling in the default mode keeps playback outside live resize.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.02, true, &tick) }
}

impl Playback {
    fn advance(
        &mut self,
        window: &Native,
        mtm: MainThreadMarker,
        send: &mpsc::Sender<Result<Expectations, Error>>,
    ) -> Result<(), Error> {
        match *self {
            Self::WaitingForKeyWindow => {
                let app = NSApplication::sharedApplication(mtm);
                if !app.isActive() || !window.window.isKeyWindow() {
                    return Ok(());
                }
                let content: &NSResponder = &window.view;
                if !window
                    .window
                    .firstResponder()
                    .as_deref()
                    .is_some_and(|first| ptr::eq(first, content))
                {
                    return Err(failure(
                        "probe expected the content view as first responder",
                    ));
                }
                if window.view.appearance().is_some() {
                    return Err(failure(
                        "probe content view must start with inherited appearance",
                    ));
                }
                let snapshot = window.preferences.get();
                let original = check_preferences(snapshot)?;
                let workspace = NSWorkspace::sharedWorkspace();
                if snapshot.increased_contrast
                    != Some(workspace.accessibilityDisplayShouldIncreaseContrast())
                    || snapshot.reduced_motion
                        != Some(workspace.accessibilityDisplayShouldReduceMotion())
                {
                    return Err(failure(
                        "probe contrast or motion snapshot differs from AppKit",
                    ));
                }
                let (flipped, appearance) = opposite_appearance(original)?;
                let mut keys = Vec::new();
                for (event, expected) in prepare_keys(window.window.windowNumber())? {
                    // Let NSApplication perform key-equivalent and responder
                    // dispatch through its queue, including the unclaimed Command key.
                    app.postEvent_atStart(&event, false);
                    keys.push(expected);
                }
                window.view.setAppearance(Some(&appearance));
                *self = Self::WaitingForFlip(flipped);
                if send
                    .send(Ok(Expectations {
                        keys,
                        appearances: [flipped, original],
                    }))
                    .is_err()
                {
                    window.view.setAppearance(None);
                    *self = Self::Finished;
                }
            }
            Self::WaitingForFlip(flipped)
                if window.preferences.get().appearance == Some(flipped) =>
            {
                // The real view callback has published the flipped snapshot.
                // Restore inheritance and let AppKit deliver the callback again.
                window.view.setAppearance(None);
                *self = Self::Finished;
            }
            _ => {}
        }
        Ok(())
    }
}

fn opposite_appearance(
    original: SystemAppearance,
) -> Result<(SystemAppearance, Retained<NSAppearance>), Error> {
    // SAFETY: The AppKit names are process-lifetime NSString constants.
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let (target, name) = match original {
        SystemAppearance::Light => (SystemAppearance::Dark, dark),
        SystemAppearance::Dark => (SystemAppearance::Light, light),
        _ => return Err(failure("probe requires light or dark appearance")),
    };
    let appearance = NSAppearance::appearanceNamed(name)
        .ok_or_else(|| failure("AppKit appearance unavailable"))?;
    Ok((target, appearance))
}

fn prepare_keys(window_number: isize) -> Result<Vec<(Retained<NSEvent>, WindowEvent)>, Error> {
    let plain = KeyModifiers::default();
    let command = KeyModifiers {
        meta: true,
        ..plain
    };
    let shift = KeyModifiers {
        shift: true,
        ..plain
    };
    let samples = [
        (0x0d, KeyCode::KeyW, "w", true, plain),
        (0x0d, KeyCode::KeyW, "w", false, plain),
        (0x28, KeyCode::KeyK, "k", true, command),
        (0x30, KeyCode::Tab, "\t", true, plain),
        (0x30, KeyCode::Tab, "\u{19}", true, shift),
    ];
    samples
        .into_iter()
        .map(|(native_code, code, characters, pressed, modifiers)| {
            let kind = if pressed {
                NSEventType::KeyDown
            } else {
                NSEventType::KeyUp
            };
            let mut flags = NSEventModifierFlags::empty();
            flags.set(NSEventModifierFlags::Command, modifiers.meta);
            flags.set(NSEventModifierFlags::Shift, modifiers.shift);
            let characters = NSString::from_str(characters);
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0.0, 0.0),
                flags,
                native_time().as_secs_f64(),
                window_number,
                None,
                &characters,
                &characters,
                false,
                native_code,
            )
            .ok_or_else(|| failure("AppKit could not create a probe key event"))?;
            let expected = WindowEvent::Key {
                code,
                logical: code.us_logical_key(),
                modifiers,
                pressed,
                repeat: false,
                timestamp: seconds(event.timestamp()),
            };
            Ok((event, expected))
        })
        .collect()
}

fn check_preferences(preferences: SystemPreferences) -> Result<SystemAppearance, Error> {
    let appearance = preferences
        .appearance
        .ok_or_else(|| failure("AppKit did not supply an appearance"))?;
    let channels = preferences
        .accent_color
        .ok_or_else(|| failure("AppKit did not supply an owned accent color"))?
        .to_srgb();
    let [_, _, _, alpha] = channels;
    if !channels.iter().all(|channel| channel.is_finite()) || !(0.0..=1.0).contains(&alpha) {
        return Err(failure("AppKit accent color has invalid sRGB channels"));
    }
    if preferences.increased_contrast.is_none() || preferences.reduced_motion.is_none() {
        return Err(failure(
            "AppKit did not supply contrast and motion preferences",
        ));
    }
    Ok(appearance)
}

fn check_window(
    endpoint: &mut dyn WindowEndpoint,
    receive: mpsc::Receiver<Result<Expectations, Error>>,
    deadline: Duration,
) -> Result<SystemPreferences, Error> {
    let initial = endpoint.preferences();
    check_preferences(initial)?;
    loop {
        match next_event(endpoint, deadline)? {
            Some(WindowEvent::Resized { .. }) => break,
            Some(_) => {}
            None => return Err(failure("timed out waiting for the initial probe resize")),
        }
    }
    let expected = receive
        .recv_timeout(deadline.saturating_sub(endpoint.now()))
        .map_err(|_| {
            failure("timed out waiting for the probe window to become active and key")
        })??;
    let mut key_count = 0;
    let mut appearance_count = 0;
    while key_count < expected.keys.len() || appearance_count < expected.appearances.len() {
        let Some(event) = next_event(endpoint, deadline)? else {
            return Err(Error::OperationFailed(format!(
                "M5 input probe timed out after {key_count}/{} routed key transitions and {appearance_count}/{} appearance updates. Next key {:?}.",
                expected.keys.len(),
                expected.appearances.len(),
                expected.keys.get(key_count),
            )));
        };
        match event {
            WindowEvent::Key { timestamp, .. }
                if expected.keys.iter().any(|candidate| {
                    matches!(candidate, WindowEvent::Key { timestamp: posted, .. } if *posted == timestamp)
                }) =>
            {
                if expected.keys.get(key_count) != Some(&event) {
                    return Err(Error::OperationFailed(format!(
                        "unexpected routed probe key {event:?}. Expected {:?}.",
                        expected.keys.get(key_count),
                    )));
                }
                key_count += 1;
            }
            WindowEvent::PreferencesChanged { preferences } => {
                let appearance = check_preferences(preferences)?;
                if expected.appearances.get(appearance_count) == Some(&appearance) {
                    appearance_count += 1;
                }
            }
            _ => {}
        }
    }
    Ok(initial)
}

fn next_event(
    endpoint: &mut dyn WindowEndpoint,
    deadline: Duration,
) -> Result<Option<WindowEvent>, Error> {
    if endpoint.now() >= deadline {
        return Ok(None);
    }
    match endpoint.wait_event_until(deadline)? {
        Some(WindowEvent::Destroyed | WindowEvent::CloseRequested) => {
            Err(failure("probe window closed before checks completed"))
        }
        event => Ok(event),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use oxiview_base::Color;

    #[test]
    fn probe_key_fixtures_preserve_backtab_and_command() {
        let keys = prepare_keys(0).expect("NSEvents without a window");
        assert_eq!(keys.len(), 5);
        for (event, expected) in keys {
            assert_eq!(
                super::super::keyboard::transition(&event, event.r#type() == NSEventType::KeyDown),
                expected
            );
        }
    }

    #[test]
    fn probe_requires_supported_preferences_and_a_finite_accent() {
        let ready = SystemPreferences {
            appearance: Some(SystemAppearance::Light),
            accent_color: Some(Color::from_rgba8(10, 80, 190, 255)),
            increased_contrast: Some(false),
            reduced_motion: Some(false),
            text_scale: None,
        };
        assert_eq!(check_preferences(ready).ok(), Some(SystemAppearance::Light));
        for incomplete in [
            SystemPreferences {
                appearance: None,
                ..ready
            },
            SystemPreferences {
                accent_color: None,
                ..ready
            },
            SystemPreferences {
                increased_contrast: None,
                ..ready
            },
            SystemPreferences {
                reduced_motion: None,
                ..ready
            },
            SystemPreferences {
                accent_color: Some(Color::from_srgb([f32::NAN, 0.0, 0.0, 1.0])),
                ..ready
            },
        ] {
            assert!(check_preferences(incomplete).is_err());
        }
    }
}
