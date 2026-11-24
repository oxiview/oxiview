use super::*;
use objc2_foundation::NSNumber;
use objc2_quartz_core::CAMetalDisplayLinkUpdate;

/// Display identity and host-clock observations for one Metal display-link callback.
#[derive(Clone, Copy, Debug)]
pub struct MetalDisplayUpdate {
    /// The screen's Core Graphics display identifier.
    pub display_id: u32,
    /// Whether the physical display is built into this Mac.
    pub built_in: bool,
    /// The display's advertised maximum refresh rate.
    pub max_hz: u32,
    /// Apple's estimated on-screen time for the update.
    pub target: Duration,
}

pub(super) struct Source {
    display_id: u32,
    built_in: bool,
    max_hz: u32,
}

impl Source {
    pub(super) fn new(window: &Native) -> Option<Self> {
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGDisplayIsBuiltin(display: u32) -> u32;
        }
        let screen = window.window.screen()?;
        let description = screen.deviceDescription();
        // SAFETY: NSScreenNumber is documented as the NSNumber containing this screen's CGDirectDisplayID.
        let number: Option<Retained<NSNumber>> =
            unsafe { msg_send![&*description, objectForKey: ns_string!("NSScreenNumber")] };
        let display_id = number?.unsignedIntValue();
        // SAFETY: This ID comes from the retained NSScreen. CGDisplayIsBuiltin takes an ID by value.
        let built_in = unsafe { CGDisplayIsBuiltin(display_id) } != 0;
        let max_hz = u32::try_from(screen.maximumFramesPerSecond()).ok()?;
        Some(Self {
            display_id,
            built_in,
            max_hz,
        })
    }

    pub(super) fn record(
        &self,
        feedback: &presentation_probe::Feedback,
        update: &CAMetalDisplayLinkUpdate,
    ) {
        if let Ok(mut entries) = feedback.display_updates.lock()
            && entries.len() < 8192
        {
            entries.push(MetalDisplayUpdate {
                display_id: self.display_id,
                built_in: self.built_in,
                max_hz: self.max_hz,
                target: seconds(update.targetPresentationTimestamp()),
            });
        }
    }
}
