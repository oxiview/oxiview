//! Main-thread AppKit services for the editor. The application receives
//! paths and plain text, and owns all source file I/O.

use super::{Native, boundary, find, text_input::owned_string};
use block2::RcBlock;
use objc2::{MainThreadOnly, msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSModalResponse, NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSPasteboard,
    NSPasteboardItem, NSPasteboardTypeString, NSSavePanel,
};
use objc2_foundation::{NSArray, NSString, NSURL};
use oxiview_platform::{
    ClipboardRequest, ClipboardResponse, FilePanelKind, FilePanelRequest, RequestId, WindowEvent,
};
use std::{
    cell::RefCell,
    ffi::{CStr, OsStr, c_char},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
};

pub(super) fn clipboard(request: ClipboardRequest) -> WindowEvent {
    let result = clipboard_on(&NSPasteboard::generalPasteboard(), &request);
    WindowEvent::Clipboard {
        id: request.id(),
        result,
    }
}

pub(super) fn clipboard_on(
    board: &NSPasteboard,
    request: &ClipboardRequest,
) -> Result<ClipboardResponse, String> {
    // SAFETY: This is AppKit's static NSString for plain text pasteboard items.
    let plain_text = unsafe { NSPasteboardTypeString };
    match request {
        ClipboardRequest::Read { .. } => {
            if let Some(text) = board.stringForType(plain_text) {
                let text = owned_string(&text).ok_or("Clipboard text contains invalid Unicode")?;
                return Ok(ClipboardResponse::Read(Some(text)));
            }
            if board
                .availableTypeFromArray(&NSArray::from_slice(&[plain_text]))
                .is_some()
            {
                return Err("Could not read plain text from the clipboard".into());
            }
            Ok(ClipboardResponse::Read(None))
        }
        ClipboardRequest::Write { text, .. } => {
            let item = NSPasteboardItem::new();
            if !item.setString_forType(&NSString::from_str(text), plain_text) {
                return Err("Could not create a plain-text clipboard item".into());
            }
            board.clearContents();
            if !board.writeObjects(&NSArray::from_slice(&[ProtocolObject::from_ref(&*item)])) {
                return Err("The clipboard rejected the plain-text item".into());
            }
            Ok(ClipboardResponse::Written)
        }
        _ => Err("Unsupported clipboard request".into()),
    }
}

struct ActivePanel {
    id: RequestId,
    panel: Retained<NSSavePanel>,
}

#[derive(Default)]
pub(super) struct Panels {
    active: RefCell<Option<ActivePanel>>,
}

impl Panels {
    pub(super) fn show(&self, window: &Native, request: FilePanelRequest) {
        let id = request.id;
        if self.active.borrow().is_some() || window.window.attachedSheet().is_some() {
            window.route_input(WindowEvent::FilePanel {
                id,
                result: Err("A panel is already open for this window".into()),
            });
            return;
        }
        let panel = match request.kind {
            FilePanelKind::Open => {
                let panel = NSOpenPanel::openPanel(window.view.mtm());
                panel.setCanChooseFiles(true);
                panel.setCanChooseDirectories(false);
                panel.setAllowsMultipleSelection(false);
                panel.into_super()
            }
            FilePanelKind::Save { suggested_name } => {
                let panel = NSSavePanel::savePanel(window.view.mtm());
                panel.setCanCreateDirectories(true);
                if let Some(name) = suggested_name {
                    panel.setNameFieldStringValue(&NSString::from_str(&name));
                }
                panel
            }
            _ => {
                window.route_input(WindowEvent::FilePanel {
                    id,
                    result: Err("Unsupported file panel kind".into()),
                });
                return;
            }
        };
        let window_id = window.shared.id;
        self.active.replace(Some(ActivePanel {
            id,
            panel: panel.clone(),
        }));
        // The AppKit sheet retains the block until its completion on the main
        // thread. A window closing during the sheet retires the request first.
        let handler = RcBlock::new(move |response: NSModalResponse| {
            boundary(|| {
                if let Some(window) = find(window_id) {
                    window.panels.complete(&window, id, response);
                }
            });
        });
        panel.beginSheetModalForWindow_completionHandler(&window.window, &handler);
    }

    fn complete(&self, window: &Native, id: RequestId, response: NSModalResponse) {
        let panel = {
            let mut active = self.active.borrow_mut();
            if !active.as_ref().is_some_and(|panel| panel.id == id) {
                return;
            }
            active.take()
        };
        let Some(active) = panel else { return };
        let result = if response == NSModalResponseOK {
            panel_path(&active.panel).map(Some)
        } else if response == NSModalResponseCancel {
            Ok(None)
        } else {
            Err("AppKit could not complete the file panel".into())
        };
        active.panel.orderOut(None);
        window.route_input(WindowEvent::FilePanel { id, result });
    }

    pub(super) fn close(&self) {
        let active = self.active.borrow_mut().take();
        if let Some(active) = active {
            // SAFETY: AppKit defines nil as an accepted sender for this action.
            // The AppKit-owned sheet and its parent still exist on the main thread.
            unsafe {
                active.panel.cancel(None);
            }
        }
    }
}

fn panel_path(panel: &NSSavePanel) -> Result<PathBuf, String> {
    let url = panel.URL().ok_or("The panel did not return a file URL")?;
    file_path(&url).ok_or_else(|| "Could not get the selected file path".into())
}

fn file_path(url: &NSURL) -> Option<PathBuf> {
    if !url.isFileURL() {
        return None;
    }
    // SAFETY: This is NSURL's exact selector and C-string return ABI. Its
    // representation is borrowed until the active autorelease pool drains.
    let path: *const c_char = unsafe { msg_send![url, fileSystemRepresentation] };
    if path.is_null() {
        return None;
    }
    // SAFETY: The live file NSURL returned a non-null, NUL-terminated path for
    // this pool lifetime; the PathBuf copies the bytes before releasing the URL.
    let path = unsafe { CStr::from_ptr(path) };
    Some(PathBuf::from(OsStr::from_bytes(path.to_bytes())))
}

#[cfg(test)]
#[allow(clippy::expect_used)] // Fixture URLs have known valid representations.
mod tests {
    use super::*;

    #[test]
    fn file_url_conversion_preserves_unicode_and_literal_percent_characters() {
        let path = "/notes/数学 😀 %2F.md";
        let file = NSURL::fileURLWithPath(&NSString::from_str(path));
        assert_eq!(file_path(&file), Some(PathBuf::from(path)));
        let remote = NSURL::URLWithString(&NSString::from_str("https://example.org/notes.md"))
            .expect("well-formed HTTP URL");
        assert_eq!(file_path(&remote), None);
    }
}
