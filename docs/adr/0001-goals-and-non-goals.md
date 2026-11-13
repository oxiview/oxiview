---
status: accepted
---

# Goals and non-goals: broad application coverage, milestone progression, house style, and hard performance targets in CI

This is a self-drawn Rust UI framework exclusively for native desktop platforms (macOS / Windows / Linux), using no Web technologies at any stage (WebView, HTML/CSS, WASM, or browser targets). It does not target a single application category: productivity tools, general desktop applications, visualization, and creative applications are all in scope. Capabilities progress from simple to complex by milestone: text, layout, borders/backgrounds, basic components, complex components, then 3D and external-content compositing. A consistent house style spans all three platforms, with full theming and customization; interactions follow platform conventions. Quantifiable hard performance targets become CI regression checks.

## Hard performance targets (initial version, enforced in CI)

- Input-to-pixel latency ≤ 1 frame (about 8 ms at 120 Hz)
- Zero repaints while idle, with CPU usage around 0%
- No dropped frames while scrolling a 100,000-row virtual list

## Non-goals

- **Wrapping native system widgets**: all widgets are self-drawn; no wrappers around NSButton, Win32 controls, or equivalents.
- **A GUI designer**: no drag-and-drop visual UI editor.
- **Multi-language bindings**: Rust API only, without C / C++ / Python bindings.

Embedding the framework as a UI layer inside a game engine is not a non-goal; that use case remains open.

## Consequences

- Broad coverage means the architecture cannot specialize in one application category, such as GPUI's editor focus. Every layer must be extensible to 3D and external-content compositing.
- Performance targets enter CI from day one, so benchmark and latency-measurement infrastructure must be built during M0–M1.
