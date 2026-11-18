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

## M1 execution note — 2026-11-16

AppKit window creation and event dispatch run on the main thread. A dedicated
window thread owns the application, scheduler, scene construction and surface
presentation. The application factory runs there, allowing application state to
remain `!Send`; cross-thread endpoints carry owned events and wake requests.
The headless backend implements the same interface and resize protocol.

Interactive live resize is a bounded exception to the nonblocking main-thread
rule: the main thread services a resize-only run-loop mode while waiting up to
the configured timeout (250 ms by default) for the window thread to commit the resized frame with the Core Animation
transaction. Timeout releases the main thread and is traced. Ordinary rendering
and fullscreen reparenting do not use transaction-synchronized presentation.

M1 implemented one window in the demo. Multiple windows and shared-thread mode
remain M9 work. Native latency
is checked locally under [ADR-0026](0026-m1-native-latency-gates.md), including the
main-to-window-thread hop.

M3 installs the [reactive runtime and local executor](../plans/m3-reactivity-and-components.md)
on the window thread. Ordinary native frames now acquire their surface texture
on a worker. Input dispatch, layout, painting and submission stay on the window
thread. [Native frame pacing](0036-fifo-drawable-readiness.md)
