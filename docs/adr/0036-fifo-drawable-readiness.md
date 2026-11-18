---
status: accepted
---

# Pace native frames by drawable availability

The macOS fullscreen implementation previously deferred each ordinary frame
after two acquisitions still awaited presentation callbacks. On the 120 Hz
Retina display, the 128-object animation at 3456×2168 presented about 54 FPS.
With identical pixels and FIFO, raising the limit to three reached about 102
FPS. Letting Metal pace acquisition reached about 117 FPS, but input dispatch
then waited behind the blocking acquisition on the window thread.

[Metal's drawable pool](https://developer.apple.com/documentation/quartzcore/cametallayer/nextdrawable%28%29)
supplies the next available texture. The
[presented handler](https://developer.apple.com/documentation/metal/mtldrawable)
observes when a drawable was displayed. Counting frames waiting for that
observation underfilled the pipeline on this system.

## Decision

For a dirty ordinary frame, the surface starts one safe wgpu texture
acquisition on a lazily created worker. While the pool is busy the window
thread services input, timers and application wakes. The acquisition publishes
its result before notifying the window loop. Layout, painting and submission
use the latest application state on the window thread. The native platform and
headless backend have a separate readiness notification. Repeated or obsolete
notifications are harmless.

The scheduler keeps one outstanding frame demand and preserves its oldest input
timestamp across waits and skipped submissions. If a deferred prediction is
already a whole refresh interval late, the acquired texture can serve the next
display slot. The worker sleeps when there is no request. Presentations still
use FIFO in fullscreen, with the same three-drawable Metal pool.

A zero-sized suspension abandons the pending result immediately. Before a
nonzero resize or teardown the window releases old textures and joins the old
worker. This satisfies [wgpu's surface reconfiguration contract](https://docs.rs/wgpu/30.0.2/wgpu/struct.Surface.html#method.configure).
The bounded native acquisition can delay this transition. Resize painting and
acknowledgement stay synchronous with the AppKit transaction. Windowed
Immediate presentation uses the same worker. Its drawable pool can also
block even though presentation does not wait for vertical sync.

This supersedes the fixed FIFO admission rule in the M1 execution note of
[ADR-0004](0004-frame-scheduling.md). Presentation callbacks still calibrate
the predicted display clock and provide actual scanout evidence. The latency,
idle and baseline contracts in [ADR-0026](0026-m1-native-latency-gates.md)
still apply.

## Evidence and trade-offs

The worker adds one cross-thread handoff and can hold one drawable during
painting. It avoids both a guessed callback budget and an acquisition wait
inside input dispatch. Increasing the old budget improved animation cadence
on one machine but retained its dependency on compositor latency. Immediate
presentation in fullscreen changed synchronization as well as pacing.

On 2026-11-18 the release build ran 128 objects on the M2 Pro's built-in
120 Hz display at 2× scale. Each run warmed up for one second. FPS counts
drawables with a positive Metal presentation timestamp.

| Mode and physical pixels | Original, 5 s | After fix, three 5 s runs | Median FPS | After fix, 20 s |
| --- | ---: | --- | ---: | ---: |
| Windowed, 1920×1200 | 119.6 | 118.2, 119.6, 119.6 | 119.6 | 119.35 |
| Fullscreen, 3456×2168 | 53.6 | 117.8, 120.0, 119.6 | 119.6 | 119.95 |

Fullscreen P95 display gap fell from 33.33 ms to 8.34 ms (median across the
three runs). The first 20-second windowed attempt timed out waiting for a
stable visible window. The table reports the completed retry.

The renderer also skips per-pixel opacity-stack allocation for flat scenes
and encodes solid colors directly. In two fullscreen pointer probes, all
111 markers reached the display. Capture-to-display P95 was 40.0 and 42.4 ms,
compared with 50.2 and 50.4 ms with the original gate. Capture starts before
posting a synthetic AppKit event and uses the first displayed drawable that
contains its marker. In a separate sustained-input `window --fullscreen-check`
run, fullscreen input-to-submit P95 was 10.67 ms against an 8.33 ms refresh
interval. The cadence and paint-to-submit checks passed.

Use the [continuous animation measurement](../../examples/animation-fps/README.md)
with fixed load, backing scale and sample duration. Compare actual displayed
FPS and the P95 display gap. Also exercise input during acquisition, stale
notifications after resize, zero-size suspension, resource release and idle
windows. Paint-to-submit starts at application paint. End-to-end input timing
includes time spent waiting for a drawable.
