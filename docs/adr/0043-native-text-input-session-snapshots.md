---
status: accepted
---

# Answer native text-input queries from a session snapshot

AppKit can ask for the selected range, marked range, surrounding text or candidate rectangle from inside the same call that sets marked text. A preceding click can also move focus to another field before the next key arrives. The editing model and paragraph layout live on the window thread, while the input context and these synchronous callbacks live on the main thread. [Window threading](0003-window-threading-model.md) · [AppKit text input client](https://developer.apple.com/documentation/appkit/nstextinputclient)

Assign a new session to each focused editor lifetime. The window publishes an owned snapshot with displayed UTF-8 text, selection, marked range, visible geometry, input revision and the latest acknowledged native sequence. The macOS adapter applies outstanding native edits to its local copy, so a same-call query sees the preceding update immediately. A window-thread edit supersedes that projection by advancing the input revision. An acknowledged snapshot retires the covered updates. Old sessions cannot edit a newly focused field. The committed document and undo history have one owner on the window thread. [Port types](../../crates/oxiview-platform/src/editor.rs) · [AppKit projection](../../crates/oxiview-platform-macos/src/macos/text_state.rs)

The application flushes platform commands after an event and again after a paint that can change geometry. On macOS, later keys can queue behind a raw input turn until its focus or selection update has arrived. The main thread continues pumping events. On the headless backend the same session, revision and sequence values drive the event and reply tests. [Native input ordering](../../crates/oxiview-platform-macos/src/macos/input_dispatch.rs) · [Window loop](../../crates/oxiview-core/src/lib.rs)

Blocking the main thread for each query would simplify range ownership but would couple input-method progress to layout and surface work. A passive snapshot alone would briefly return old text during a native callback. Keeping a revisioned projection incurs copying when the editor changes and requires explicit handling of stale callbacks. Geometry and text stay in logical window coordinates and UTF-8 at the port. Each adapter converts to its platform's range and coordinate units. [Committed text boundary](0042-publish-committed-editor-text.md)
