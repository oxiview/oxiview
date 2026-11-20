//! Opt-in Cocoa protocol and responder smoke test. No system pasteboard access.

use super::{
    MacOsPlatform, Native, boundary, desktop_services::clipboard_on, failure, find, native_time,
    text_input::owned_string, text_state::MISSING,
};
use block2::RcBlock;
use objc2::{AnyThread, MainThreadOnly, rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSPasteboard, NSScreen,
    NSTextInputClient,
};
use objc2_foundation::{MainThreadMarker, NSAttributedString, NSPoint, NSRange, NSString, NSTimer};
use oxiview_base::{Rect, Size};
use oxiview_platform::*;
use std::{cell::RefCell, ops::Range, ptr::NonNull, thread, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(5);
const FIRST: TextInputSession = TextInputSession(1);
const SECOND: TextInputSession = TextInputSession(2);
const BOARD: RequestId = RequestId(7);
const SAMPLE: &str = "oxiview 中文 العربية 😀\0end";

/// Runs the opt-in M6 probe on the process main thread, with one AppKit window.
/// It exercises the input-client ABI, selection and surrounding text, candidate
/// geometry across window movement, ordered focus, and a private pasteboard.
/// The probe closes on completion or after five seconds. The executable adds
/// a process watchdog. A human Pinyin session also exercises candidate UI.
pub fn diagnostic_m6_input_probe() -> Result<(), Error> {
    let mtm = MainThreadMarker::new().ok_or(Error::MainThreadRequired)?;
    let deadline = native_time()
        .checked_add(TIMEOUT)
        .ok_or_else(|| failure("M6 input probe deadline overflow"))?;
    let mut worker = None;
    let mut timer = None;
    let result = MacOsPlatform::default().run(|context| {
        let window = context.create_window(WindowAttributes {
            title: "oxiview M6 text input probe".into(),
            logical_size: Size::new(360.0, 160.0),
            resize_timeout: Duration::from_millis(500),
        })?;
        let id = window.main.id();
        let mut endpoint = window.endpoint;
        worker = Some(
            thread::Builder::new()
                .name("m6-input-probe-window".into())
                .spawn(move || check_window(&mut *endpoint, deadline))
                .map_err(|error| Error::OperationFailed(error.to_string()))?,
        );
        timer = Some(schedule(mtm, id));
        Ok(())
    });
    if let Some(timer) = timer {
        timer.invalidate();
    }
    let outcome = worker.map(thread::JoinHandle::join);
    result?;
    outcome
        .ok_or_else(|| failure("M6 input probe did not start"))?
        .map_err(|_| failure("M6 input probe thread panicked"))?
}

type Client = ProtocolObject<dyn NSTextInputClient>;

enum Playback {
    First,
    Second(Retained<Client>),
    Finished,
}

fn schedule(_mtm: MainThreadMarker, id: WindowId) -> Retained<NSTimer> {
    let playback = RefCell::new(Playback::First);
    let tick = RcBlock::new(move |_: NonNull<NSTimer>| {
        boundary(|| {
            let Some(window) = find(id).filter(|window| !window.closed.get()) else {
                return;
            };
            let Ok(mut playback) = playback.try_borrow_mut() else {
                return;
            };
            if let Err(error) = playback.advance(&window) {
                *playback = Playback::Finished;
                window.route_input(WindowEvent::Clipboard {
                    id: BOARD,
                    result: Err(error.to_string()),
                });
            }
        });
    });
    // SAFETY: The marker requires the main thread, and the timer copies the
    // block and calls it in the main run loop's default mode.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.02, true, &tick) }
}

impl Playback {
    fn advance(&mut self, window: &Native) -> Result<(), Error> {
        let Some(session) = window
            .input
            .session()
            .filter(|_| window.window.isKeyWindow())
        else {
            return Ok(());
        };
        let Some(context) = window.view.inputContext() else {
            return Err(failure(
                "the content view did not publish its input context",
            ));
        };
        match self {
            Self::First if session.id == FIRST => {
                let client = context.client();
                check_client(window, &client)?;
                send_click_and_keys(window, &client)?;
                *self = Self::Second(client);
            }
            Self::Second(old) if session.id == SECOND => {
                ensure(
                    old.selectedRange() == MISSING,
                    "retired client still exposes a selection",
                )?;
                ensure(!old.hasMarkedText(), "retired client still reports preedit")?;
                let obsolete = NSString::from_str("retired");
                // SAFETY: The NSString and NSRange satisfy the Cocoa callback contract.
                unsafe {
                    old.insertText_replacementRange(&obsolete, MISSING);
                }

                let result = private_clipboard();
                window.route_input(WindowEvent::Clipboard { id: BOARD, result });
                *self = Self::Finished;
            }
            _ => {}
        }
        Ok(())
    }
}

fn fixture(stage: u64) -> TextInputSnapshot {
    let (text, selection, marked) = match stage {
        1 => ("😀ni\nsecond", 6..6, Some(4..6)),
        2 => ("😀nihao\nsecond", 9..9, Some(4..9)),
        3 | 5 => ("😀你好\nsecond", 10..10, None),
        4 => ("😀你好t\nsecond", 11..11, Some(10..11)),
        6 => ("😀你好z\nsecond", 11..11, Some(10..11)),
        _ => ("😀旧\nsecond", 4..7, None),
    };
    let caret = Rect::new(58.0, 16.0, 59.0, 34.0);
    let geometry = if stage == 0 {
        vec![
            TextInputGeometry {
                range: 0..4,
                rect: Rect::new(18.0, 16.0, 42.0, 34.0),
            },
            TextInputGeometry {
                range: 4..7,
                rect: Rect::new(42.0, 16.0, 58.0, 34.0),
            },
            TextInputGeometry {
                range: 8..14,
                rect: Rect::new(18.0, 40.0, 80.0, 58.0),
            },
        ]
    } else {
        Vec::new()
    };
    TextInputSnapshot {
        session: FIRST,
        revision: 1,
        acknowledged: stage,
        text: text.into(),
        selection,
        marked,
        caret,
        geometry,
    }
}

fn ensure(condition: bool, problem: &str) -> Result<(), Error> {
    if condition {
        Ok(())
    } else {
        Err(failure(problem))
    }
}

fn check_client(window: &Native, client: &Client) -> Result<(), Error> {
    ensure(
        client.selectedRange() == NSRange::new(2, 1),
        "initial selection UTF-16 range",
    )?;
    let mut actual = MISSING;
    let proposed = NSRange::new(0, 10);
    // SAFETY: The actualRange pointer points at a live writable NSRange.
    let first = unsafe { client.firstRectForCharacterRange_actualRange(proposed, &mut actual) };
    ensure(
        actual == NSRange::new(0, 3),
        "multi-line query did not return the first visual line",
    )?;
    ensure(
        first.size.width == 40.0 && first.size.height == 18.0,
        "incorrect candidate rectangle dimensions",
    )?;
    let index =
        client.characterIndexForPoint(NSPoint::new(first.origin.x + 25.0, first.origin.y + 5.0));
    ensure(
        index == 2,
        "screen point did not resolve to the expected UTF-16 index",
    )?;

    let from = window.window.frame().origin;
    let current = window.window.screen();
    let target = NSScreen::screens(window.view.mtm())
        .to_vec()
        .into_iter()
        .find(|screen| {
            current
                .as_deref()
                .is_some_and(|current| current != &**screen)
        })
        .map_or_else(
            || NSPoint::new(from.x + 24.0, from.y + 15.0),
            |screen| {
                let visible = screen.visibleFrame();
                NSPoint::new(visible.origin.x + 40.0, visible.origin.y + 40.0)
            },
        );
    window.window.setFrameOrigin(target);
    let to = window.window.frame().origin;
    // SAFETY: The NSRange out-parameter remains valid through this call.
    let moved = unsafe { client.firstRectForCharacterRange_actualRange(proposed, &mut actual) };
    ensure(
        (to.x != from.x || to.y != from.y)
            && (moved.origin.x - first.origin.x - (to.x - from.x)).abs() < 0.01
            && (moved.origin.y - first.origin.y - (to.y - from.y)).abs() < 0.01,
        "candidate rectangle did not follow the window in screen coordinates",
    )?;

    let marked =
        NSAttributedString::initWithString(NSAttributedString::alloc(), &NSString::from_str("ni"));
    // SAFETY: All inputs are Objective-C strings with scalar-aligned selections.
    unsafe {
        client.setMarkedText_selectedRange_replacementRange(&marked, NSRange::new(2, 0), MISSING);
    }
    ensure(
        client.hasMarkedText()
            && client.markedRange() == NSRange::new(2, 2)
            && client.selectedRange() == NSRange::new(4, 0),
        "synchronous queries did not see the pending mark",
    )?;
    // SAFETY: Writable NSRange output parameter as required by the Cocoa ABI.
    let substring = unsafe {
        client.attributedSubstringForProposedRange_actualRange(NSRange::new(2, 2), &mut actual)
    };
    ensure(
        actual == NSRange::new(2, 2)
            && substring
                .as_deref()
                .and_then(|text| owned_string(&text.string()))
                .as_deref()
                == Some("ni"),
        "pending surrounding text query did not return the marked substring",
    )?;
    // SAFETY: This is a writable NSRange. A legitimate Cocoa client must
    // reject a string range that splits a surrogate pair.
    let invalid = unsafe {
        client.attributedSubstringForProposedRange_actualRange(NSRange::new(1, 1), &mut actual)
    };
    ensure(
        invalid.is_none() && actual == MISSING,
        "substring query accepted a split surrogate",
    )?;
    let bad = NSString::from_str("BAD");
    let update = NSString::from_str("ihao");
    let commit = NSString::from_str("你好");
    // SAFETY: NSString arguments and scalar NSRange values satisfy the ABI.
    // The first attempted replacement is intentionally invalid in UTF-16.
    unsafe {
        client.insertText_replacementRange(&bad, NSRange::new(1, 1));
        client.setMarkedText_selectedRange_replacementRange(
            &update,
            NSRange::new(4, 0),
            NSRange::new(1, 1),
        );
        client.insertText_replacementRange(&commit, MISSING);
    }
    ensure(
        !client.hasMarkedText() && client.selectedRange() == NSRange::new(4, 0),
        "synchronous selection did not reflect the pending commit",
    )?;
    // SAFETY: These NSStrings are valid and the selections are lengths of the
    // supplied strings. An empty marked replacement cancels the pending mark.
    unsafe {
        client.setMarkedText_selectedRange_replacementRange(
            &NSString::from_str("t"),
            NSRange::new(1, 0),
            MISSING,
        );
        client.setMarkedText_selectedRange_replacementRange(
            &NSString::from_str(""),
            NSRange::new(0, 0),
            MISSING,
        );
    }
    ensure(
        !client.hasMarkedText() && client.selectedRange() == NSRange::new(4, 0),
        "clear-mark callback did not restore the outgoing selection",
    )?;
    // SAFETY: As above, this NSString's length is one UTF-16 unit. The next
    // mouse press transfers focus with a mark in flight.
    unsafe {
        client.setMarkedText_selectedRange_replacementRange(
            &NSString::from_str("z"),
            NSRange::new(1, 0),
            MISSING,
        );
    }
    Ok(())
}

fn send_click_and_keys(window: &Native, client: &Client) -> Result<(), Error> {
    let number = window.window.windowNumber();
    let app = NSApplication::sharedApplication(window.view.mtm());
    let click = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDown, NSPoint::new(100.0, 32.0), NSEventModifierFlags::empty(),
        native_time().as_secs_f64(), number, None, -6, 1, 0.0,
    ).ok_or_else(|| failure("could not construct the focus-transfer mouse event"))?;
    // Send down and the very next key before the window thread can publish the
    // incoming field. NSApplication still performs native responder dispatch.
    app.sendEvent(&click);
    let late = NSString::from_str("late");
    // SAFETY: A valid NSString and the Cocoa sentinel are supplied.
    unsafe {
        client.insertText_replacementRange(&late, MISSING);
    }
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let characters = NSString::from_str("x");
        let key = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind, NSPoint::ZERO, NSEventModifierFlags::empty(),
            native_time().as_secs_f64(), number, None,
            &characters, &characters, false, 0x07,
        ).ok_or_else(|| failure("could not construct the first key for the incoming focus"))?;
        app.sendEvent(&key);
    }
    Ok(())
}

fn private_clipboard() -> Result<ClipboardResponse, String> {
    let board = NSPasteboard::pasteboardWithUniqueName();
    let read = ClipboardRequest::Read { id: BOARD };
    let initial = clipboard_on(&board, &read)?;
    if initial != ClipboardResponse::Read(None) {
        return Err("The new private pasteboard was not empty".into());
    }
    let write = clipboard_on(
        &board,
        &ClipboardRequest::Write {
            id: BOARD,
            text: SAMPLE.into(),
        },
    )?;
    if write != ClipboardResponse::Written {
        return Err("The private pasteboard did not acknowledge the write".into());
    }
    let reply = clipboard_on(&board, &read);
    board.clearContents();
    reply
}

fn check_window(endpoint: &mut dyn WindowEndpoint, deadline: Duration) -> Result<(), Error> {
    let mut active = FIRST;
    let mut expected = 1;
    let mut first_new_key = false;
    let mut stale = false;
    let mut board = false;

    while endpoint.now() < deadline {
        let event = endpoint.wait_event_until(deadline)?;
        match event {
            Some(WindowEvent::ActivationChanged { active: true, .. }) if expected == 1 => {
                endpoint.flush_platform(vec![PlatformCommand::TextInput(Some(fixture(0)))])?;
            }
            Some(WindowEvent::Resized { token, .. }) => {
                endpoint.acknowledge_resize(token)?;
                if expected == 1 {
                    endpoint.flush_platform(vec![PlatformCommand::TextInput(Some(fixture(0)))])?;
                }
            }
            Some(WindowEvent::TextInput(input)) if input.session == FIRST && active == FIRST => {
                ensure(
                    input.revision == 1 && input.sequence == expected,
                    "first session callback order",
                )?;
                check_edit(expected, &input.edit)?;
                endpoint
                    .flush_platform(vec![PlatformCommand::TextInput(Some(fixture(expected)))])?;
                expected += 1;
            }
            Some(WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                ..
            }) => {
                ensure(
                    expected == 7 && active == FIRST,
                    "pointer reached the app before the earlier text callbacks",
                )?;
                active = SECOND;
                let mut next = fixture(0);
                next.session = SECOND;
                next.text.clear();
                next.selection = 0..0;
                next.geometry.clear();
                endpoint.flush_platform(vec![PlatformCommand::TextInput(Some(next))])?;
            }
            Some(WindowEvent::TextInput(input)) if input.session == FIRST && active == SECOND => {
                ensure(
                    input.sequence == 7
                        && matches!(input.edit, TextInputEdit::Commit { ref text, .. } if text == "late"),
                    "unexpected callback from the outgoing session",
                )?;
                stale = true;
            }
            Some(WindowEvent::TextInput(input)) if input.session == SECOND && active == SECOND => {
                ensure(
                    input.sequence == 1
                        && matches!(input.edit,
                    TextInputEdit::Commit { ref range, ref text, .. } |
                    TextInputEdit::SetMarked { ref range, ref text, .. }
                    if *range == (0..0) && !text.is_empty()),
                    "the first key did not target the incoming session",
                )?;
                first_new_key = true;
            }
            Some(WindowEvent::Key {
                code: KeyCode::KeyX,
                ..
            }) => {
                return Err(failure(
                    "the consumed text stroke also arrived as a raw key",
                ));
            }
            Some(WindowEvent::Clipboard { id: BOARD, result }) => {
                let result = result.map_err(Error::OperationFailed)?;
                ensure(
                    result == ClipboardResponse::Read(Some(SAMPLE.into())),
                    "private pasteboard round trip changed the text",
                )?;
                board = true;
            }
            Some(WindowEvent::Destroyed | WindowEvent::CloseRequested) => {
                return Err(failure("the M6 probe closed before completing the checks"));
            }
            None => break,
            _ => {}
        }
        endpoint.flush_platform(Vec::new())?;
        if first_new_key && stale && board {
            return Ok(());
        }
    }
    Err(Error::OperationFailed(format!(
        "M6 input probe timed out. Next first-session sequence {expected}, incoming key {first_new_key}, stale callback {stale}, clipboard {board}"
    )))
}

fn check_edit(sequence: u64, edit: &TextInputEdit) -> Result<(), Error> {
    let marked = |range: Range<usize>, text: &str, selected: usize| TextInputEdit::SetMarked {
        range,
        text: text.into(),
        selection: selected..selected,
    };
    let expected = match sequence {
        1 => marked(4..7, "ni", 2),
        2 => marked(5..6, "ihao", 4),
        3 => TextInputEdit::Commit {
            range: 4..9,
            text: "你好".into(),
            kind: TextCommitKind::Composition,
        },
        4 => marked(10..10, "t", 1),
        5 => TextInputEdit::Cancel,
        6 => marked(10..10, "z", 1),
        _ => return Err(failure("extra callback in the first session")),
    };
    if edit == &expected {
        Ok(())
    } else {
        Err(Error::OperationFailed(format!(
            "expected {expected:?}, got {edit:?}"
        )))
    }
}
