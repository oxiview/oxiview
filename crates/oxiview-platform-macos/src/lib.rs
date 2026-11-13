#![allow(unsafe_code)]
//! Minimal AppKit adapter for the platform context.
//!
//! Unsafe is confined here to Objective-C initialization, native bitmap copying,
//! and window ownership. The borrowed frame is copied before AppKit retains it.
//! This is a one-image viewer, replaced when M1 designs the real platform trait.

use oxiview_platform::{Error, Frame, WindowPresenter};

/// AppKit implementation of the temporary one-image viewer.
#[derive(Default)]
pub struct MacOsPresenter;

impl WindowPresenter for MacOsPresenter {
    fn present(&self, frame: Frame<'_>) -> Result<(), Error> {
        validate(frame)?;
        #[cfg(target_os = "macos")]
        {
            macos::present(frame)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
}

fn validate(frame: Frame<'_>) -> Result<(), Error> {
    let length = usize::try_from(frame.width)
        .ok()
        .and_then(|w| w.checked_mul(frame.height as usize))
        .and_then(|area| area.checked_mul(4));
    if frame.width == 0
        || frame.height == 0
        || length != Some(frame.pixels.len())
        || length.is_none_or(|n| n > isize::MAX as usize)
    {
        return Err(Error::InvalidFrame);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2::runtime::ProtocolObject;
    use objc2::{AnyThread, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapFormat,
        NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage, NSImageView, NSWindow, NSWindowDelegate,
        NSWindowStyleMask,
    };
    use objc2_foundation::{
        MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
        ns_string,
    };

    define_class!(
        // SAFETY: NSObject permits subclassing, no Drop implementation is added,
        // and MainThreadOnly keeps every AppKit callback on the main thread.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct CloseDelegate;

        // SAFETY: NSObjectProtocol adds no implementation invariants.
        unsafe impl NSObjectProtocol for CloseDelegate {}
        // SAFETY: The callback has the protocol's exact signature and thread affinity.
        unsafe impl NSWindowDelegate for CloseDelegate {
            #[unsafe(method(windowWillClose:))]
            fn window_will_close(&self, _notification: &NSNotification) {
                // Never let an unexpected Rust unwind cross the Objective-C callback.
                if std::panic::catch_unwind(|| {
                    NSApplication::sharedApplication(self.mtm()).stopModal();
                })
                .is_err()
                {
                    // State cannot safely be resumed after an unexpected callback panic.
                    std::process::abort();
                }
            }
        }
    );

    pub(super) fn present(frame: Frame<'_>) -> Result<(), Error> {
        let mtm = MainThreadMarker::new().ok_or(Error::MainThreadRequired)?;
        autoreleasepool(|_| {
            let size = NSSize::new(f64::from(frame.width), f64::from(frame.height));
            // SAFETY: Null planes requests AppKit-owned storage. Validation ensures
            // nonzero dimensions and a representable tightly packed RGBA8 length.
            // Four 8-bit samples, alpha last and non-premultiplied match Frame.
            let bitmap = unsafe {
                NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                    NSBitmapImageRep::alloc(), std::ptr::null_mut(),
                    frame.width as isize, frame.height as isize, 8, 4, true, false,
                    NSDeviceRGBColorSpace, NSBitmapFormat::AlphaNonpremultiplied,
                    frame.width as isize * 4, 32,
                )
            }.ok_or(Error::BitmapAllocation)?;
            let destination = bitmap.bitmapData();
            if destination.is_null() {
                return Err(Error::BitmapAllocation);
            }
            // SAFETY: The new bitmap exclusively owns width*height*4 writable
            // bytes with our explicit row stride. Source has that exact validated
            // length and is disjoint from this fresh allocation. Neither is released
            // during the copy, and no AppKit view can access the bitmap yet.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    frame.pixels.as_ptr(),
                    destination,
                    frame.pixels.len(),
                );
            }
            let image = NSImage::initWithSize(NSImage::alloc(), size);
            image.addRepresentation(&bitmap);
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
            let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect);
            view.setImage(Some(&image));
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.finishLaunching();
            // SAFETY: Main-thread allocation, valid finite content rect, and
            // releasedWhenClosed is disabled immediately to retain Rust ownership.
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    rect,
                    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // SAFETY: Rust's Retained owns the window, rather than a window
            // controller; disabling close-time release prevents a double release.
            unsafe {
                window.setReleasedWhenClosed(false);
            }
            // SAFETY: NSObject init has this signature and initializes this fresh
            // allocation. The delegate is retained until after modal processing.
            let delegate: Retained<CloseDelegate> =
                unsafe { msg_send![CloseDelegate::alloc(mtm), init] };
            window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            window.setTitle(ns_string!("oxiview — M0"));
            window.setContentView(Some(&view));
            window.center();
            window.makeKeyAndOrderFront(None);
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            app.runModalForWindow(&window);
            window.setDelegate(None);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_frame_layouts_before_native_access() {
        for frame in [
            Frame {
                width: 0,
                height: 1,
                pixels: &[],
            },
            Frame {
                width: 1,
                height: 0,
                pixels: &[],
            },
            Frame {
                width: 1,
                height: 1,
                pixels: &[0; 3],
            },
            Frame {
                width: 1,
                height: 1,
                pixels: &[0; 5],
            },
            Frame {
                width: u32::MAX,
                height: u32::MAX,
                pixels: &[],
            },
        ] {
            assert!(matches!(
                MacOsPresenter.present(frame),
                Err(Error::InvalidFrame)
            ));
        }
        assert!(
            validate(Frame {
                width: 2,
                height: 1,
                pixels: &[0; 8]
            })
            .is_ok()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_worker_thread_before_appkit_access() {
        // Test workers are not the process main thread.
        assert!(matches!(
            MacOsPresenter.present(Frame {
                width: 1,
                height: 1,
                pixels: &[0; 4]
            }),
            Err(Error::MainThreadRequired)
        ));
    }
}
