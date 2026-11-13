---
status: accepted
---

# Platform layer: own backends for all three platforms from day one, without winit

Windows, event loops, input, and system services connect directly to each platform: AppKit on macOS, Win32 on Windows, and Wayland/X11 on Linux. Do not depend on winit or other generic window libraries. The framework requires input-to-pixel latency ≤ 1 frame, flicker-free resizing, fully integrated custom title bars (Snap Layouts and traffic-light buttons), and multiple windows on multiple threads (ADR-0003). These require direct control of event loops and presentation timing; a generic window abstraction would limit that control.

On Linux, **Wayland first, X11 compatibility**: implement new capabilities on Wayland first. Keep X11 usable without requiring feature parity.

Expose system services (menus, tray, dialogs, clipboard, drag and drop, notifications, cursors, window materials, and others) through **capability traits**. Applications query support at runtime through `capabilities()`; the framework does not silently degrade missing capabilities.

## Considered Options

- **Start with winit and leave an escape hatch**: faster initially, but multi-threaded windows, IME candidate positioning, and resize-synchronized presentation require workarounds, concentrating migration costs later.
- **Use winit indefinitely**: limited by upstream capabilities; rejected.

## Consequences

- Maintain a test matrix for three platform backends and two Linux protocols.
- Integrate IME, accessibility bridges (AccessKit adapters), and DPI changes separately into each owned backend.
