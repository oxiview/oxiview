# Window diagnostics and Layout Lab

## M2 Layout Lab

```sh
cargo run --release -p window -- --layout-demo
cargo run --release -p window -- --layout-demo --headless
```

The native preview exercises Flex, Grid and Block layout in one retained tree.
Use ADD and REMOVE to change the tree, LAYER to change the stacking order, and
drag the orange tile across the clip or outside the window to exercise pointer
capture. The headless command runs the interaction checks and writes
`initial.png`, `interacted.png` and `resized.png` under `target/xtask/m2-preview/`.
The M1 measurement and overlay flags apply to the pointer-following scene below.
[M2 plan and acceptance status](../../docs/plans/m2-node-tree-and-layout.md)

## M1 pointer-following scene

From the repository root:

```sh
cargo run -p window
cargo run -p window -- --headless
cargo run --release -p window -- --measure-latency 120 --json
cargo run --release -p window -- --idle-check 30 --json
```

The native demo requires macOS 14+, a visible WindowServer display session and
a Metal GPU. It renders a pointer-following rounded rectangle through a wgpu
surface, on demand, on its window thread. Close the window to exit.
The headless mode renders the scene with the CPU adapter and writes
`target/xtask/window.png`, without starting AppKit or a GPU.

Measurement modes close the window automatically. Exit codes are 0 for a passed
hard check, 1 for a measured hard-check failure, and 2 for invalid arguments,
missing environment facilities or failure to measure. With `--json`, stdout
contains exactly one JSON object on successful measurement (including a failed
hard check); errors go to stderr and produce no measurement object. The modes
are mutually exclusive and require a positive duration or 1–100,000 samples.

Latency settles for one second and presents a warm-up pointer frame, then
sweeps twenty deterministic refresh phases, `((i × 7) mod 20) / 20`.
The paused display-link target is extrapolated by whole intervals to schedule
future posts; OS scheduling can delay the actual post. Each new pointer
position invalidates the scene. Only one input is outstanding, and its capture
timestamp is matched through core's `LatencyRecord` to the drawable rendered
for it. Metal's nonzero `presentedTime` supplies actual on-screen time.
Frame submission and presentation each have a five-second timeout. No predicted
timestamp substitutes for an actual presentation. The display interval must
remain stable for the run. Capture-to-submit nearest-rank p95 must be ≤ one refresh interval. Pixel p95 is compared by xtask to the human-recorded optimized baseline × 1.15 on the same display; standalone measurement cannot decide this gate.
Synthetic AppKit events bypass the HID/WindowServer input path: hardware input
latency is excluded.

Idle settles for one second after the first frame, then measures
`getrusage(RUSAGE_SELF)` user + system CPU divided by wall time × 100.
A pass requires CPU ≤ 0.1%, zero actual on-screen presentations, zero submitted
frames, and zero display-link frame events during the interval. The measurement
thread sleeps during the interval; it does not poll. Any pointer/button/key event during the interval returns exit 2: "user input during idle measurement; rerun without touching the mouse". The framework still receives and processes input.

The example opts into the platform's release-capable `diagnostics` feature.
Production drawable tracking estimates compositor delay; measurement record collection is activated only for measurement runs. This replaces the
debug-only `OXIVIEW_WINDOW_INPUT_CHECK` probe. Diagnostics are measurement
machinery, not a framework product API.

`cargo xtask bench` runs three latency rounds (median of their p95 values),
then the idle mode, sequentially after Divan;
`cargo xtask verify --full` therefore opens real windows. Ordinary verify uses
headless tests. See [performance conventions](../../docs/conventions/testing-performance.md)
for baseline comparison and human-confirmed recording rules.

Run with `--debug-overlay` for a top-right graph of the most recent 60 produced frames. Teal bars show paint-to-submit time, amber bars show input-to-submit time from the oldest input attributed to that frame, and the white line is one refresh interval. One terminal line per second of frame production reports frame count, averages/maxima, and interval. There is no timer or self-invalidation: idle produces no output or frames. The flag is off by default and rejected with `--measure-latency`, `--idle-check`, or `--headless`; baseline measurement runs do not construct or draw the overlay. It can be combined with `--fullscreen-check` to exercise its real rendering cost under sustained input. The GPU renderer bins these primitives by physical region so a pixel evaluates only its tile's commands.

Run `cargo run --release -p window -- --fullscreen-check --debug-overlay --json`
for an automated sustained-input check, or omit `--debug-overlay` to test the
minimal scene. It measures windowed, fullscreen and restored-window stages.
Each stage calibrates the current refresh interval, posts pointer events at
twice that rate, warms up for one second, then measures two seconds. Submissions
and actual presentations must each reach 80% of refresh rate, with p95 frame
gaps no greater than 1.5 refresh intervals. These tolerances catch half-rate
rendering while allowing OS jitter; they do not change the latency hard target.

Entering fullscreen also requires a confirmed on-screen transition frame without
pointer input. Missing presentation feedback or a changing refresh interval
returns exit 2. A failed cadence check or paint-to-submit p95 above one refresh
interval returns exit 1. JSON separates `cadence_pass`,
`paint_submit_within_refresh` and `input_submit_within_refresh`; 60 FPS alone
does not establish low latency. Each stage also reports submitted/displayed FPS,
frame gaps, paint-to-submit p95, input-to-submit p50/p95 and
`input_submit_over_refresh_percent`, corresponding to the amber bars above the
white line. Continuous input can coalesce several events into one frame; its
oldest capture timestamp is preserved. Input overruns remain explicit in this
report, while `--measure-latency` enforces the separate phase-swept M1 gate. This
check does not establish a fullscreen input-to-pixel baseline (deferred to M10)
or inspect pixels. A human must still confirm full-window coverage, pointer
tracking, and flicker-free transitions and live resize.
