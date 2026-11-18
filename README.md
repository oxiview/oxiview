# oxiview

**English** | [简体中文](README_ZH.md)

A self-drawn Rust UI framework exclusively for native macOS, Windows, and Linux platforms.

M1 delivers the first native macOS window, a deterministic headless platform,
on-demand scheduling on a dedicated window thread, and GPU surface presentation
with an SDF rounded-rectangle path. M2 now has an interactive native preview
with a retained UI tree, Flex/Grid/Block layout, stacking, clipping, pointer
routing, and capture. The [detailed plan](docs/plans/m2-node-tree-and-layout.md)
records the approved design and remaining full-milestone work.
The latest integration target is a playable [Snake game](examples/snake/README.md)
using M3 Signals/components/view! and M4 real font shaping, DPI-aware glyph
images, and GPU compositing. Framework implementation and full milestone
acceptance are tracked separately in the plans.
M1 is complete: native visual acceptance, fresh-clone verification and local
performance baseline comparison passed. See the [execution record](docs/plans/m1-first-window.md)
for measured results and remaining latency work.
The toolchain is pinned to Rust stable 1.97.0 with edition 2024.

Run Snake on macOS 14+ with a visible display session and Metal GPU:

```sh
cargo run --release -p snake
```

Add `-- --debug-overlay` to show FPS, display cadence, CPU timings, scene/layout
counts, and viewport scale in the top-right corner. It is off by default and
stops refreshing after one final idle update.

To measure continuous motion, run `cargo run --release -p animation-fps`.
The [animation demo](examples/animation-fps/README.md) shows live submitted and
displayed FPS, the latter measured from Metal presentation timestamps. Use
`-- --measure 5 --json` for a five-second native sample.

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

Run the M2 Layout Lab on macOS 14+ with a visible display session and Metal GPU:

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

On macOS 14+ with a visible display session and Metal GPU, run the pointer-following demo:

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

Run all local checks with:

```sh
cargo xtask verify
```

See the [M2 plan](docs/plans/m2-node-tree-and-layout.md) for the next implementation
steps, the [M1 plan](docs/plans/m1-first-window.md) for completed scope and remaining items,
[the roadmap](docs/roadmap.md) for later milestones, and
[GLOSSARY.md](GLOSSARY.md) for terminology.

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
