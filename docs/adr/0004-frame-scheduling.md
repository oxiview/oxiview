---
status: accepted
---

# Frame scheduling: on-demand by default, reserved continuous window/layer rendering, display-vsync clocks

Each window has a **frame policy**, defaulting to **on-demand**: request frames only when state is invalidated, animations run, or the system requests repainting. There is no idle frame loop, and only dirty regions are repainted. This is required for the approximately 0% idle CPU target (ADR-0001). For 3D viewports, video, and game-like scenes, the policy also supports **continuous window rendering**, producing a whole-window frame every vsync, and **continuous layer rendering**, refreshing one compositing layer each frame while the rest of the window remains on-demand. Initially implement only on-demand rendering and reserve the other two in the interface.

Align the frame clock to the window's display vsync (CVDisplayLink/CADisplayLink, DXGI, Wayland presentation-time), supporting different refresh rates across monitors and variable refresh rate (VRR). Sample animations at the **predicted presentation timestamp**, rather than the current time, to avoid jitter.

## Consequences

- The compositor needs a layer concept (see decision 12 on compositing and external content), even when only on-demand rendering is implemented initially.
- Switch frame-clock sources when a window moves between displays with different refresh rates.

## M1 execution note — 2026-11-16

M1 implements on-demand whole-window repainting. Invalidation, a timed request or
a system repaint request can schedule a frame; an idle window produces no frames
or display-link frame events. Dirty-region repainting is deferred. Both
continuous policies remain explicit unsupported capabilities until M10.

On macOS 14+, a view-associated `CADisplayLink` supplies display timing and pauses
when no work is pending. The headless backend supplies deterministic virtual
vsync. Predictions include a bounded compositor-offset estimate updated by
actual Metal presentation feedback; a prediction is never reported as measured
input-to-pixel latency. M1 has no animation API; that remains M8.

Windowed surfaces prefer Immediate presentation and fullscreen uses FIFO. Both
keep three Metal drawables. At M1 close-out the platform deferred ordinary FIFO
frame opportunities while two acquired drawables awaited presentation.
Presentation callbacks released capacity and woke deferred work.
After a display/fullscreen transition, one repaint restores content even if
AppKit discarded a transition drawable. These requests do not create an idle
rendering loop. See the [performance conventions](../conventions/testing-performance.md)
for the diagnostic workload and the remaining continuous-input latency overruns.

Since 2026-11-18 the renderer acquires ordinary drawables asynchronously in
both modes. The window thread continues processing input while it waits.
[ADR-0036](0036-fifo-drawable-readiness.md) replaces the M1 two-presentation rule.
