---
status: accepted
---

# Threading model: the main thread pumps events; one window thread per window by default

The **main thread** only receives system events and calls APIs that require it, such as menus, IME cursor positioning, and window creation. Each window has its own **window thread** by default, running state, layout, painting, and GPU presentation. CAMetalLayer, DXGI swapchains, and Vulkan/Wayland surfaces allow presentation off the main thread. Applications may also place multiple windows on one window thread (**shared-thread mode**). A stalled window then need not stall others, and UI computation does not delay main-thread event handling.

State and signals on a window thread are `!Send`, used only on that thread without locks. Share data across window threads through application-level `Send` state with messages or subscriptions. Windows in shared-thread mode can directly share `!Send` state.

## Considered Options

- **One UI thread shared by all windows** (GPUI and most frameworks): simplest cross-window state sharing, but heavy computation in one window blocks all windows. Retained as an optional mode.
- **All state Send + Sync**: adds locks on hot paths and forces atomic operations into the reactive runtime; rejected.

## Consequences

- The reactive runtime (decision 4) must be thread-local, with one instance per window thread.
- Forwarding events from the main thread to a window thread adds a cross-thread hop. Include it in the input-to-pixel ≤ 1 frame budget and monitor it in CI.
- Platform backends must provide low-latency main/window-thread channels that are not starved by blocking operations such as macOS modal loops or Windows drag-resize loops.
