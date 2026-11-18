# oxiview

**English** | [简体中文](README_ZH.md)

A self-drawn Rust UI framework exclusively for native macOS, Windows, and Linux platforms.

M1 delivers the first native macOS window, a deterministic headless platform,
on-demand scheduling on a dedicated window thread, and GPU surface presentation
with an SDF rounded-rectangle path. M2 now has an interactive native preview
with a retained UI tree, Flex/Grid/Block layout, stacking, clipping, pointer
routing, and capture. The [detailed plan](docs/plans/m2-node-tree-and-layout.md)
records the approved design and remaining full-milestone work.
The playable [Snake game](examples/snake/README.md) uses M3 Signals, function
components and optional JSX-like `view!` expressions in Rust source files.
M4 shapes its text, rasterizes glyphs at the display scale and composites them
on the GPU. The milestone plans track the remaining acceptance work.
M1 is complete: native visual acceptance, fresh-clone verification and local
performance baseline comparison passed. See the [execution record](docs/plans/m1-first-window.md)
for measured results and remaining latency work.
The toolchain is pinned to Rust stable 1.97.0 with edition 2024.

## Snake

Run Snake on macOS 14+ with a visible display session and Metal GPU.

```sh
cargo run --release -p snake
```

Add `-- --debug-overlay` to show application FPS, display cadence, CPU timings,
scene/layout counts and viewport scale in the top-right corner. At Classic
pace the snake initially advances every 125 ms, so about 8 application FPS
is expected during steady play. The overlay is off by default and stops
refreshing after one final idle update.

Enter starts a new game, Space pauses/resumes, and arrows or WASD steer. Mouse
controls, selectable pace, score, session best, and recent runs are included.
Ready/paused/finished games schedule no periodic ticks. Losing activation pauses
the game; returning waits for explicit resume.

For real-font images and deterministic acceptance without a visible window:

```sh
cargo run --release -p snake -- --headless --json
```

Artifacts and the pass/fail manifest are written to `target/xtask/snake/`.
`./examples/snake/package-macos.sh` builds a directly openable local application
bundle there. See the [M3 plan](docs/plans/m3-reactivity-and-components.md) and
[M4 text/Snake plan](docs/plans/m4-text-and-snake.md) for scope and pending gates.

## Continuous animation FPS

Run `cargo run --release -p animation-fps` for the live panel. For a repeatable
comparison, run these commands separately on the same display.

```sh
cargo run --release -p animation-fps -- --objects 128 --measure 5 --json
cargo run --release -p animation-fps -- --objects 128 --measure 5 --fullscreen --json
```

The demo reports submissions and actual displayed frames separately. Compare
`displayed_fps` and `display_gap_p95_ms` alongside the physical pixel
dimensions. See the [measurement guide](examples/animation-fps/README.md) and
the [native frame pacing results](docs/adr/0036-fifo-drawable-readiness.md).

## Layout Lab

Run the M2 Layout Lab on macOS 14+ with a visible display session and Metal GPU.

```sh
cargo run --release -p window -- --layout-demo
```

Use **ADD / REMOVE** to change the live tree, **WRAP / RESIZE** to change Flex
layout, **LAYER** to change stacking, and **RESET** to restore positions and
dimensions. Hover over the tiles; click a flow tile to change its width or a
Grid tile to change its opacity. Drag the orange tile across its clip or outside
the window, then release. Resize the window to inspect responsive layout.
This older example retains its bitmap diagnostic captions. Snake exercises the
new real-font text path.

For deterministic interaction checks and CPU-rendered preview images:

```sh
cargo run --release -p window -- --layout-demo --headless
```

This checks creation/removal, stale IDs, reflow, Grid placement, capture dragging,
stacking, and unchanged-input redraw suppression. Images are written to
`target/xtask/m2-preview/{initial,interacted,resized}.png`. They are preview
artifacts, not accepted snapshot baselines. M2 milestone acceptance and its
new performance baselines remain pending.

## M1 window diagnostics

On macOS 14+ with a visible display session and Metal GPU, run the pointer-following demo.

```sh
cargo run --release -p window -- --debug-overlay
```

Move the pointer, resize the window, and use Ctrl+Cmd+F to toggle fullscreen.
The overlay shows paint-to-submit and oldest-input-to-submit latency against one
refresh interval. Continuous-input bars can exceed that line; the M1 baseline
uses a separate phase-swept workload. Actual pixel latency is checked against a
human-recorded baseline, not an absolute one-frame bound. See
[the accepted latency gates](docs/adr/0026-m1-native-latency-gates.md).

For a CPU-rendered demo image without a native window:

```sh
cargo run --release -p window -- --headless
```

The image is written to `target/xtask/window.png`.

## Verification

Run the standard local checks from the repository root.

```sh
cargo xtask verify
```

The [roadmap](docs/roadmap.md) links the milestone plans and their acceptance
status. The [glossary](GLOSSARY.md) defines the framework terminology.

License: MIT OR Apache-2.0. Copyright The oxiview Authors.

`verify` checks for rustc 1.97 stable and cargo-deny, then runs formatting,
Clippy, tests, dependency checks, and documentation checks. It aggregates failures
from all checks after the environment check passes. `verify --full` also runs benchmarks.

Other commands include `cargo xtask fmt [--check]`, `lint`, `test`, `deps`, and `doctor`.
All subcommands support `--json`, which restricts stdout to the JSON report.
Each run overwrites `target/xtask/report.json`; subprocess logs are saved in the same directory.
Each check reports `name`, `status` (pass/fail/error), `duration_ms`,
`failure_summary`, and `artifacts` (relative to the repository root).
Exit codes: 0 for success, 1 for failed checks, and 2 for environment or xtask errors.
`doctor` suggests installation commands for missing tools; it never installs them automatically.

`cargo xtask verify --full` also measures native windowed latency and idle CPU
and compares this machine's performance baselines. It requires a visible macOS
session; keep the mouse still during the idle check. Missing baselines fail and
may be recorded only after human confirmation. See
[the performance protocol](docs/conventions/testing-performance.md).

Windows and Linux remain planned native ports. Focus/Action mapping, editable
text/IME, and the widget library remain later milestones. The original M1 demo and measurement modes
retain their rendering/scheduling workload; use the separate M2 preview mode
for node-tree and layout interaction.
