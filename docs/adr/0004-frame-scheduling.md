---
status: accepted
---

# Frame scheduling: on-demand by default, reserved continuous window/layer rendering, display-vsync clocks

Each window has a **frame policy**, defaulting to **on-demand**: request frames only when state is invalidated, animations run, or the system requests repainting. There is no idle frame loop, and only dirty regions are repainted. This is required for the approximately 0% idle CPU target (ADR-0001). For 3D viewports, video, and game-like scenes, the policy also supports **continuous window rendering**, producing a whole-window frame every vsync, and **continuous layer rendering**, refreshing one compositing layer each frame while the rest of the window remains on-demand. Initially implement only on-demand rendering and reserve the other two in the interface.

Align the frame clock to the window's display vsync (CVDisplayLink/CADisplayLink, DXGI, Wayland presentation-time), supporting different refresh rates across monitors and variable refresh rate (VRR). Sample animations at the **predicted presentation timestamp**, rather than the current time, to avoid jitter.

## Consequences

- The compositor needs a layer concept (see decision 12 on compositing and external content), even when only on-demand rendering is implemented initially.
- Switch frame-clock sources when a window moves between displays with different refresh rates.
