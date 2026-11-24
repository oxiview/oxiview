# Performance history

All archived measurement campaigns appear below, newest first. Each heading identifies the Git HEAD at measurement time. An asterisk indicates local changes. Each run links to its original benchmark or animation samples. [Measurement protocol](../conventions/testing-performance.md)

## M7 log viewer · 2026-11-23 `32f6c6d`\*

Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

Test HEAD `32f6c6d84f47f28c3e8e3f1674e661fd17f20039`.

| 100,000-row native scroll | Displayed / expected slots | Measurement |
| --- | ---: | ---: |
| Untraced 1 | 1192 / 1200 | 119.2 FPS |
| Untraced 2 | 1160 / 1200 | 116.0 FPS |
| Untraced 3 | 1186 / 1200 | 118.6 FPS |
| Apple trace 1 | 751 / 1200 | 0.83 ms/s (lower bound) |
| Apple trace 2 | 1162 / 1200 | 0.83 ms/s (lower bound) |

[m7-scroll.json](runs/1795426783-32f6c6d/m7-scroll.json) · [run.json](runs/1795426783-32f6c6d/run.json)

## Full retest after pacing fix `82a8325`\*

Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

Test HEAD `82a83254254c6eb4910604c303343c5dc14c02be`. Related implementation subsequently committed as `4f78e14`.

| Metric | Full retest after pacing fix `82a8325`\* |
| --- | ---: |
| Windowed input to submission, P95 | 4.3136 ms |
| Windowed input to display, P95 | 27.1255 ms |
| Idle process CPU (observation length) | 0.00820% (30 s) |
| Idle frames submitted / presented / display-link events | 0 / 0 / 0 |
| Offscreen GPU, 1 sparse rounded rectangle | 2.267 ms |
| Offscreen GPU, 128 sparse rounded rectangles | 2.487 ms |
| Offscreen GPU, 1,000 sparse rounded rectangles | 4.272 ms |

The release build rendered 128 moving objects and the metrics panel at 2× scale after a 1 s warm-up. Displayed FPS counts Metal-confirmed presentations. Repeated runs are summarized by their median.

| Run | Mode and physical pixels | Samples | Displayed FPS | Display gap P95 | Longer observation |
| --- | --- | ---: | ---: | ---: | ---: |
| Full retest after pacing fix `82a8325`\* | Windowed · 1920 × 1200 | 3 × 5 s | 119.6 | 8.34 ms | 119.35 FPS (1 × 20 s) |
| Full retest after pacing fix `82a8325`\* | Fullscreen FIFO · 3456 × 2168 | 3 × 5 s | 119.6 | 8.34 ms | 119.95 FPS (1 × 20 s) |

Fullscreen pointer probes for Full retest after pacing fix `82a8325`\* measured capture-to-display P95 at 40.0 ms and 42.4 ms. Metal presented 111 of 111 input markers. A separate sustained-input run measured input-to-submit P95 at 10.67 ms.

[animation-fullscreen-1.json](runs/1794988411-82a8325/animation-fullscreen-1.json) · [animation-fullscreen-2.json](runs/1794988411-82a8325/animation-fullscreen-2.json) · [animation-fullscreen-3.json](runs/1794988411-82a8325/animation-fullscreen-3.json) · [animation-fullscreen-4.json](runs/1794988411-82a8325/animation-fullscreen-4.json) · [animation-windowed-1.json](runs/1794988411-82a8325/animation-windowed-1.json) · [animation-windowed-2.json](runs/1794988411-82a8325/animation-windowed-2.json) · [animation-windowed-3.json](runs/1794988411-82a8325/animation-windowed-3.json) · [animation-windowed-4.json](runs/1794988411-82a8325/animation-windowed-4.json) · [bench.json](runs/1794988411-82a8325/bench.json) · [run.json](runs/1794988411-82a8325/run.json)

## Before pacing fix `82a8325`\*

Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

Test HEAD `82a83254254c6eb4910604c303343c5dc14c02be`. Related implementation subsequently committed as `6d36dab`.

The release build rendered 128 moving objects and the metrics panel at 2× scale after a 1 s warm-up. Displayed FPS counts Metal-confirmed presentations. Repeated runs are summarized by their median.

| Run | Mode and physical pixels | Samples | Displayed FPS | Display gap P95 | Longer observation |
| --- | --- | ---: | ---: | ---: | ---: |
| Before pacing fix `82a8325`\* | Windowed · 1920 × 1200 | 1 × 5 s | 119.6 | 8.34 ms | — |
| Before pacing fix `82a8325`\* | Fullscreen FIFO · 3456 × 2168 | 1 × 5 s | 53.6 | 33.33 ms | — |

Fullscreen pointer probes for Before pacing fix `82a8325`\* measured capture-to-display P95 at 50.2 ms and 50.4 ms. Metal presented 112 of 112 input markers.

[animation-fullscreen-1.json](runs/1794982863-82a8325/animation-fullscreen-1.json) · [animation-windowed-1.json](runs/1794982863-82a8325/animation-windowed-1.json) · [run.json](runs/1794982863-82a8325/run.json)

## M1 acceptance `4c324e1`\*

Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

Test HEAD `4c324e1bafd5a6a00c64f089cb81f5ab7fecddcb`. Related implementation subsequently committed as `197da5e`.

| Metric | M1 acceptance `4c324e1`\* |
| --- | ---: |
| Windowed input to submission, P95 | 4.620 ms |
| Windowed input to display, P95 | 27.304 ms |
| Idle process CPU (observation length) | 0.00937% (30 s) |
| Idle frames submitted / presented / display-link events | 0 / 0 / 0 |
| Offscreen GPU, 1 sparse rounded rectangle | 2.511 ms |
| Offscreen GPU, 128 sparse rounded rectangles | 2.742 ms |
| Offscreen GPU, 1,000 sparse rounded rectangles | 4.394 ms |

[bench.json](runs/1794878728-4c324e1/bench.json) · [run.json](runs/1794878728-4c324e1/run.json)

## Recorded baseline `9078375`\*

Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

Test HEAD `907837576282e331b14cf1f99dc97dae5099b2c0`. Related implementation subsequently committed as `857e3d7`.

| Metric | Recorded baseline `9078375`\* |
| --- | ---: |
| Windowed input to submission, P95 | 4.573 ms |
| Windowed input to display, P95 | 30.262 ms |
| Idle process CPU (observation length) | 0.02458% (5 s) |
| Idle frames submitted / presented / display-link events | 0 / 0 / 0 |
| Offscreen GPU, 1 sparse rounded rectangle | 4.130 ms |
| Offscreen GPU, 128 sparse rounded rectangles | 2.842 ms |
| Offscreen GPU, 1,000 sparse rounded rectangles | 7.465 ms |

[bench.json](runs/1794835346-9078375/bench.json) · [run.json](runs/1794835346-9078375/run.json)
