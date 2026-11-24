# oxiview

**English** | [简体中文](README_ZH.md)

A self-drawn Rust UI framework exclusively for native macOS, Windows, and Linux platforms.

M1 delivers the first native macOS window, a deterministic headless platform,
on-demand scheduling on a dedicated window thread, and GPU surface presentation
with an SDF rounded-rectangle path. M2's Layout Lab exercises a retained UI
tree, incremental Flex/Grid/Block layout, stacking, rounded clipping and
pointer capture. M2 is complete. Layout Lab passed native manual review,
and all eight tree and layout benchmarks passed against their recorded
baselines. [M2 execution record](docs/plans/m2-node-tree-and-layout.md#execution-record)
The playable [Snake game](examples/snake/README.md) uses M3 Signals, function
components and optional JSX-like `view!` expressions in Rust source files.
M3 is complete. Snake passed native review and all four reactive and component
performance comparisons passed. [M3 acceptance record](docs/plans/m3-reactivity-and-components.md#acceptance-record)
M4 is complete. Small glyphs are cached at the display scale and large
outlines render through Vello GPU. The owner confirmed text clarity in the
native Snake window. [M4 acceptance record](docs/plans/m4-text-and-snake.md#acceptance-record)
M5 is complete. Snake's M5 fork passed native review for keyboard navigation, persistent rebinding and live appearance. Three focus-ring snapshots and three performance workloads have accepted baselines. [M5 acceptance record](docs/plans/m5-focus-keyboard-and-themes.md#acceptance-record)
M6 is complete. The Markdown editor passed native macOS review for Chinese input, candidate positioning, undo and file round trips. The release replay covers the editing and preview behavior. [M6 acceptance record](docs/plans/m6-text-editing-and-markdown.md#acceptance-record)
M1 is complete: native visual acceptance, fresh-clone verification and local
performance baseline comparison passed. See the [execution record](docs/plans/m1-first-window.md)
for measured results and remaining latency work.
The toolchain is pinned to Rust stable 1.97.0 with edition 2024.

## Log viewer

M7 delivered Oxiview Logs. Open a local log or explore 100,000 sample records. The list expands messages inline. The table supports column sorting and resizing, cell navigation and copying rectangular selections. Follow keeps the latest matching record visible until the reader scrolls back. [Log viewer and keyboard controls](examples/log-viewer-oxi/README.md) · [Delivery and scrolling follow-up](docs/plans/m7-widgets-and-scrolling.md#acceptance-record)

```sh
cargo run --release -p log-viewer-oxi
cargo run --release -p log-viewer-oxi -- --headless --json
```

## Markdown editor

Run the native two-pane editor on macOS, or exercise the same application through its release headless replay.

```sh
cargo run --release -p markdown-editor-oxi
cargo run --release -p markdown-editor-oxi -- --headless --json
```

Open a Markdown file, edit multilingual source with inline IME, and preview tables, local images, Mermaid diagrams and LaTeX. [Editor and keyboard controls](examples/markdown-editor-oxi/README.md) · [M6 plan](docs/plans/m6-text-editing-and-markdown.md)

## Snake

Run Snake on macOS 14+ with a visible display session and Metal GPU.

```sh
cargo run --release -p snake
```

Run `cargo run --release -p snake-oxi` for the same game with Rust logic and
precompiled `.oxi` views and styles. [Snake in .oxi](examples/snake-oxi/README.md)

Run `cargo run --release -p snake-m5-oxi` for the M5 fork. Use Tab to reach its inline Settings, switch appearance, or record a game shortcut that persists across launches. The board and settings resolve the arrow keys in their own focus contexts. [Snake M5 demo](examples/snake-m5-oxi/README.md)

The [Neovim and LazyVim plugin](editors/oxi.nvim/README.md) enables `.oxi`
highlighting, live syntax diagnostics and navigation between views and Rust.

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
bundle there. The [M3 acceptance record](docs/plans/m3-reactivity-and-components.md#acceptance-record)
covers reactive components. The [M4 acceptance record](docs/plans/m4-text-and-snake.md#acceptance-record) covers text and rendering.

## Performance

<!-- oxiview-perf:start -->

**Test system** · Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)

**M7 log viewer · 2026-11-23 `32f6c6d`\***

| 100,000-row native scroll | Displayed / expected slots | Measurement |
| --- | ---: | ---: |
| Untraced 1 | 1192 / 1200 | 119.2 FPS |
| Untraced 2 | 1160 / 1200 | 116.0 FPS |
| Untraced 3 | 1186 / 1200 | 118.6 FPS |
| Apple trace 1 | 751 / 1200 | 0.83 ms/s (lower bound) |
| Apple trace 2 | 1162 / 1200 | 0.83 ms/s (lower bound) |

### Window latency, idle CPU and offscreen rendering

The macOS probe starts timing just before each synthetic AppKit pointer event is posted. It measures 120 events per round and reports the median of three round P95s. Submission ends when the presenter returns. Display latency ends at Metal’s actual presentation timestamp. The 1280 × 840 offscreen test includes scene encoding, GPU completion and RGBA readback.

| Metric | Recorded baseline `9078375`\* | M1 acceptance `4c324e1`\* | Full retest after pacing fix `82a8325`\* |
| --- | ---: | ---: | ---: |
| Windowed input to submission, P95 | 4.573 ms | 4.620 ms | 4.3136 ms |
| Windowed input to display, P95 | 30.262 ms | 27.304 ms | 27.1255 ms |
| Idle process CPU (observation length) | 0.02458% (5 s) | 0.00937% (30 s) | 0.00820% (30 s) |
| Idle frames submitted / presented / display-link events | 0 / 0 / 0 | 0 / 0 / 0 | 0 / 0 / 0 |
| Offscreen GPU, 1 sparse rounded rectangle | 4.130 ms | 2.511 ms | 2.267 ms |
| Offscreen GPU, 128 sparse rounded rectangles | 2.842 ms | 2.742 ms | 2.487 ms |
| Offscreen GPU, 1,000 sparse rounded rectangles | 7.465 ms | 4.394 ms | 4.272 ms |

### Continuous animation and input latency

The release build rendered 128 moving objects and the metrics panel at 2× scale after a 1 s warm-up. Displayed FPS counts Metal-confirmed presentations. Repeated runs are summarized by their median.

| Run | Mode and physical pixels | Samples | Displayed FPS | Display gap P95 | Longer observation |
| --- | --- | ---: | ---: | ---: | ---: |
| Before pacing fix `82a8325`\* | Windowed · 1920 × 1200 | 1 × 5 s | 119.6 | 8.34 ms | — |
| Full retest after pacing fix `82a8325`\* | Windowed · 1920 × 1200 | 3 × 5 s | 119.6 | 8.34 ms | 119.35 FPS (1 × 20 s) |
| Before pacing fix `82a8325`\* | Fullscreen FIFO · 3456 × 2168 | 1 × 5 s | 53.6 | 33.33 ms | — |
| Full retest after pacing fix `82a8325`\* | Fullscreen FIFO · 3456 × 2168 | 3 × 5 s | 119.6 | 8.34 ms | 119.95 FPS (1 × 20 s) |

Fullscreen pointer probes for Before pacing fix `82a8325`\* measured capture-to-display P95 at 50.2 ms and 50.4 ms. Metal presented 112 of 112 input markers.

Fullscreen pointer probes for Full retest after pacing fix `82a8325`\* measured capture-to-display P95 at 40.0 ms and 42.4 ms. Metal presented 111 of 111 input markers. A separate sustained-input run measured input-to-submit P95 at 10.67 ms.

An asterisk marks a measurement taken with local changes on top of the displayed Git HEAD. The archive identifies the commits that subsequently landed the measured changes.

<!-- oxiview-perf:end -->

[Full performance history](docs/performance/history.md) · [Measurement and update procedure](docs/conventions/testing-performance.md)

### Reproduce the animation measurements

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
stacking, and unchanged-input redraw suppression. The three preview images are
written to `target/xtask/m2-preview/{initial,interacted,resized}.png`. The
application integration suite renders dedicated CPU snapshot fixtures at 1×,
1.5× and 2×. Layout Lab can also record pointer delivery during the
[native capture review](examples/window/README.md#native-capture-review).

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

Windows and Linux remain planned native ports. The Markdown editor exercises
M6 text input. The log viewer exercises M7 widgets and large collections. The original M1 demo and measurement modes
retain their rendering/scheduling workload; use the separate M2 preview mode
for node-tree and layout interaction.
