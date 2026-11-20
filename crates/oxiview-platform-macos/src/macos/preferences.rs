use block2::{RcBlock, StackBlock};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSColor, NSColorSpace, NSSystemColorsDidChangeNotification, NSView, NSWorkspace,
    NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSNotificationCenter, NSNotificationName,
    NSObjectProtocol, NSOperationQueue,
};
use oxiview_base::Color;
use oxiview_platform::{SystemAppearance, SystemPreferences};
use std::{cell::Cell, ptr::NonNull};

pub(super) fn snapshot(view: &NSView) -> SystemPreferences {
    let appearance = view.effectiveAppearance();
    let workspace = NSWorkspace::sharedWorkspace();
    let accent = Cell::new(None);
    // Resolve the dynamic system color using the view's drawing appearance.
    appearance.performAsCurrentDrawingAppearance(&StackBlock::new(|| {
        accent.set(to_srgb(&NSColor::controlAccentColor()));
    }));
    SystemPreferences {
        appearance: appearance_kind(&appearance),
        accent_color: accent.get(),
        increased_contrast: Some(workspace.accessibilityDisplayShouldIncreaseContrast()),
        reduced_motion: Some(workspace.accessibilityDisplayShouldReduceMotion()),
        text_scale: None,
    }
}

fn appearance_kind(appearance: &NSAppearance) -> Option<SystemAppearance> {
    // SAFETY: Both AppKit names are process-lifetime NSString constants.
    let (light, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
    let name =
        appearance.bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[light, dark]))?;
    if &*name == dark {
        Some(SystemAppearance::Dark)
    } else if &*name == light {
        Some(SystemAppearance::Light)
    } else {
        None
    }
}

fn to_srgb(color: &NSColor) -> Option<Color> {
    // Convert the profile before reading components. Extended sRGB preserves
    // any out-of-gamut channels in the framework's float color representation.
    let rgb = color.colorUsingColorSpace(&NSColorSpace::extendedSRGBColorSpace())?;
    let channels = [
        rgb.redComponent(),
        rgb.greenComponent(),
        rgb.blueComponent(),
        rgb.alphaComponent(),
    ]
    .map(|channel| channel as f32);
    channels
        .iter()
        .all(|channel| channel.is_finite())
        .then(|| Color::from_srgb(channels))
}

pub(super) struct Observer {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Observer {
    fn new(center: Retained<NSNotificationCenter>, name: &NSNotificationName) -> Self {
        let main_queue = NSOperationQueue::mainQueue();
        let resample = RcBlock::new(|_: NonNull<NSNotification>| {
            super::boundary(|| {
                if MainThreadMarker::new().is_some() {
                    for window in super::natives() {
                        window.refresh_preferences();
                    }
                }
            });
        });
        // SAFETY: The block captures no data and the center retains it until
        // removal. Delivery runs on the main queue and no sender is required.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&main_queue),
                &resample,
            )
        };
        Self { center, token }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        // SAFETY: This is the observer token returned by this same center.
        unsafe { self.center.removeObserver(self.token.as_ref()) };
    }
}

pub(super) fn observe(_mtm: MainThreadMarker) -> [Observer; 2] {
    // SAFETY: Both AppKit names are process-lifetime NSString constants.
    let (colors_changed, display_options_changed) = unsafe {
        (
            NSSystemColorsDidChangeNotification,
            NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
        )
    };
    [
        Observer::new(NSNotificationCenter::defaultCenter(), colors_changed),
        Observer::new(
            NSWorkspace::sharedWorkspace().notificationCenter(),
            display_options_changed,
        ),
    ]
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use objc2_app_kit::{
        NSAppearanceNameAccessibilityHighContrastAqua,
        NSAppearanceNameAccessibilityHighContrastDarkAqua,
    };

    #[test]
    fn high_contrast_appearance_keeps_its_light_or_dark_identity() {
        // SAFETY: Both AppKit names are process-lifetime NSString constants.
        let (light, dark) = unsafe {
            (
                NSAppearanceNameAccessibilityHighContrastAqua,
                NSAppearanceNameAccessibilityHighContrastDarkAqua,
            )
        };
        for (name, expected) in [
            (light, SystemAppearance::Light),
            (dark, SystemAppearance::Dark),
        ] {
            let appearance = NSAppearance::appearanceNamed(name).expect("built-in appearance");
            assert_eq!(appearance_kind(&appearance), Some(expected));
        }
    }

    #[test]
    fn accent_conversion_reads_owned_srgb_components_and_preserves_alpha() {
        let native = NSColor::colorWithDisplayP3Red_green_blue_alpha(0.75, 0.4, 0.2, 0.375);
        let color = to_srgb(&native).expect("Display P3 can be converted to sRGB");
        drop(native);
        let [red, green, blue, alpha] = color.to_srgb();
        assert!(red > 0.75 && green < 0.4 && blue < 0.2);
        assert_eq!(alpha, 0.375);
    }
}
