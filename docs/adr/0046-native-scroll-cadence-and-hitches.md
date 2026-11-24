---
status: accepted
---

# Measure native scrolling by visible updates and hitch time

On 2026-11-23 the owner selected two targets for the 100,000-row log viewer on the built-in 120 Hz Mac display. The first is stable 120 FPS, following the scrolling experience reported for Zed's GPUI. The second is at most 1.2 milliseconds of hitch time per second of active scrolling, following Apple's example of an XCTest regression baseline. These are oxiview's targets for this workload. [GPUI measurement](https://zed.dev/blog/120fps) · [Apple XCTest example](https://developer.apple.com/videos/play/wwdc2020/10077/)

## Decision

Measure the release viewer at 1120×720 points and 2× scale. After two seconds of warm-up, scroll by 12 points on each opportunity for ten seconds. The application must produce new list content through its ordinary input, layout, paint and presentation path.

- **Cadence.** Count the refresh slots occupied by advancing content using the actual Metal presentation timestamps. Stable 120 FPS supplies a new image in all 1,200 slots. Check the calibrated display interval, both sample edges, row identity, displacement and complete drawable feedback. Preserve the existing machine-scoped dropped-frame baseline and the requirement for an independent comparison. [Original target](0001-goals-and-non-goals.md) · [Native validator](../../xtask/src/bench/scroll.rs)
- **Hitch time.** Run the same release workload with Apple's Hitches and Display instruments. Attribute hitch intervals by process, display and swap. Sum the duration inside the active scroll interval, counting overlapping time once, and divide milliseconds by seconds of active scrolling. Save the measured value, trace, raw viewer output and observation coverage. Validate the result against the viewer's drawable evidence. Insufficient coverage makes a low reading inconclusive. [Apple observer](../../xtask/m7_hitches/observe.py) · [Validation](../../xtask/src/bench/apple.rs)

The untraced run measures cadence without the profiler. The traced run measures the Apple events. A separate diagnostic build deliberately stalled the window thread three times for about 100 ms. The Metal records showed the three gaps, while Apple reported one unrelated hitch. Both measurements are needed to establish the scrolling experience of this self-drawn window. The [measurement protocol](../conventions/testing-performance.md#m7-virtualization-and-native-scrolling) gives the commands and artifact locations.
