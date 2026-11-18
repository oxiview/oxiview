# Continuous animation FPS

**English**

Run the demo on macOS 14+ with a visible window and a Metal GPU.

```sh
cargo run --release -p animation-fps
```

The window starts with 32 moving objects on three tracks. Each displayed-frame
opportunity samples the predicted presentation time to position the objects.
The workload uses the normal retained UI tree, text nodes, Scene and native
GPU presenter. Space pauses or resumes. Up doubles the object count and Down
halves it, within 1 to 128. An inactive, hidden or zero-sized window suspends
the animation. Returning to the window resumes from the held position if
playback was enabled.

The live panel shows successful submissions per second, actual displayed
drawables per second and the display-link refresh rate. Metal reports each
drawable ID with its presentation timestamp. A zero timestamp marks a drawable
that was never displayed. The P95 gap uses consecutive positive presentation
timestamps in sorted order. The panel updates four times per second and shows a
trailing second with a 100 ms delay for arriving feedback.

## Timed measurement

```sh
cargo run --release -p animation-fps -- --objects 32 --measure 5 --json
cargo run --release -p animation-fps -- --objects 32 --measure 5 --fullscreen --json
```

Each command opens its own window, warms up for one second, samples the
specified 1–60 seconds, collects trailing presentation feedback and closes the
window. Both rates use the same half-open interval on the backend's monotonic
clock. A warm-up submission counts towards displayed FPS if Metal timestamps
its display inside the measurement interval. The report includes physical
dimensions and backing scale, observed refresh range, submitted and displayed
frame counts, P95 display gap, P95 paint-to-submit time, and submissions which
received a zero presentation timestamp.

Keep the window visible, active and at a stable size for the timed run.
Interrupted runs or missing display feedback return an error. Run measurements
sequentially on the same display with the same object count and window size for
comparison. Results describe this animation and include its metrics panel.

To build a local application bundle, run `./examples/animation-fps/package-macos.sh`.
The result is `target/xtask/animation-fps/Oxiview Motion.app`.

For a visual check without a display, `cargo run -p animation-fps -- --headless`
exports two CPU-rendered PNGs 125 ms apart to `target/xtask/animation-fps/`.
Their FPS readouts stay empty.
