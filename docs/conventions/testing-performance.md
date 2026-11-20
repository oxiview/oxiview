# Benchmarks and performance baselines

`cargo xtask bench` is the workspace performance verification entry point. Ordinary `cargo xtask verify` skips benchmarks; `cargo xtask verify --full` opens real macOS windows for latency and idle measurements and adds bench after ordinary checks and continues aggregating even if earlier checks fail (doctor environment failures still return early). Use Rust stable 1.97 and edition 2024 only; nightly is unnecessary.

## Writing benchmarks

The default is [Divan 0.1.21](https://github.com/nvzqz/divan), a maintained project licensed MIT OR Apache-2.0 with MSRV 1.80. This repository pins the exact version; upgrades must revalidate output parsing. Added transitive dependencies mainly include divan-macros, condtype, regex-lite, terminal_size, and their system dependencies. `cargo xtask deps` reviews licenses, advisories, and duplicate versions.

Define versions only in root `[workspace.dependencies]`; crates inherit `divan.workspace = true` in dev-dependencies. Under ADR-0021, workspace example source lives in root `benches/compare.rs`, referenced by the `oxiview-testing` `[[bench]]` target with `harness = false`. Follow this convention for new targets; do not create another framework crate.

`benches/compare.rs` measures S4 `compare` on identical 256×256 and 1024×1024 gradient RGBA8 images, including per-pixel comparison and diff-image allocation. Image construction is outside the timed region. `benches/render.rs`, owned by `oxiview-render-wgpu`, renders 1, 128 and 1,000 sparse rounded rectangles on a 1280×840 target. Its 20 single-iteration samples include scene encoding, GPU completion and RGBA readback; scene construction and device/pipeline warm-up are outside measurement. This guards against evaluating every primitive at every pixel. It requires a native GPU and is not an input-to-pixel latency measurement. Use black_box to preserve real computation and let Divan consume the result. Benchmarks may use expect; library code still prohibits unwrap/expect/panic.

## Measurement and parsing

Full bench runs `cargo bench --workspace --benches --locked` sequentially three times using the optimized bench profile. The image comparison uses 100 samples per run and 10 iterations per sample. Each benchmark scope reads Divan's median from each run and takes the median of those three values as the final result in ns. Keep power, cooling, build configuration, and background load stable and run performance tests one at a time.

Divan currently emits terminal statistics tables. The parser validates headers and column counts, reconstructs tree names as `target/function/argument`, and supports ps/ns/µs/μs/us/ms/s. It requires finite positive values and nonzero sample counts, rejecting duplicate metrics, empty output, and inconsistent metric sets across runs. Clear inherited `DIVAN_*` overrides so filtering or test mode cannot alter measurement. New benchmarks use whitespace-free names without counter rows or custom output. Format changes require parser and test updates; parsing failure must never count as success. Standard libtest output with zero benchmarks is ignored.

Reports go to `target/xtask/report.json`. `performance.metrics` includes each current value, baseline, status, and threshold. In a full report, the three M1 metrics include `hard_check_pass` and measurement details (including latency percentiles and frames), and the M7 list metric shows `pending`. `bench-results.json` is a standalone artifact with the same details. `bench.log` records the decision summary, and `bench-1.log` through `bench-3.log` preserve raw stdout/stderr. Native stdout and stderr are preserved in `bench-latency-1.json` through `bench-latency-3.json` and their matching `.log` files, as well as `bench-idle.json` and `bench-idle.log`. Each run clears the old artifacts for its scope before measurement. `--json` prints only the report to stdout.

## M2 retained tree and layout benchmarks

The layout target computes from a fresh Taffy adapter and caller-owned Flex
column tree with up to ten children per parent and a fixed intrinsic size
fallback. Node counts include the root, and the logical viewport is 800×600.
The UI tree target starts with a prepared 985-node tree (root, 24 fixed-size
absolute rows, and 40 tiles per row) at 720×480 logical points and scale 1.
Fixture construction, warm-up, and preparation of replacement styles happen
outside the timed work. [Layout benchmark](../../benches/layout.rs) · [UI tree benchmark](../../benches/ui_tree.rs)

Run the M2 comparison once the CPU is idle and the owner is ready to measure.

```sh
cargo xtask bench --m2
```

This command selects the eight workloads below by exact Divan name and runs
three rounds of the `layout` and `ui_tree` targets without opening a native
window or running GPU, pointer latency, or idle diagnostics. The report
includes `performance.scope = "m2"` and only those metrics. Standalone results
go to `target/xtask/bench-m2-results.json`, with the decision in `bench-m2.log`
and Divan output in `bench-m2-1.log` through `bench-m2-3.log` in the same
directory.

| Report metric | Timed workload | Samples per run | Iterations per sample |
| --- | --- | ---: | ---: |
| `layout/initial_layout/100` | Compute initial layout for 100 nodes | 40 | 1 |
| `layout/initial_layout/1000` | Compute initial layout for 1,000 nodes | 40 | 1 |
| `layout/initial_layout/10000` | Compute initial layout for 10,000 nodes | 40 | 1 |
| `ui_tree/arena_edits` | Append a tile using a warmed arena slot, move it from the first row to index 0 in the last row, and remove it | 100 | 10 |
| `ui_tree/clean_preparation` | Call `paint` on the unchanged tree at the same viewport and reuse its cached Scene | 100 | 1,000 |
| `ui_tree/contained_layout_change` | Change the first tile's height from 16 to 18 logical points and run layout within the fixed-size row fixture | 40 | 1 |
| `ui_tree/paint_only_update` | Alternate the first tile's background between two colors, prepare its frame and release the frame before the next edit | 100 | 100 |
| `ui_tree/hit_testing` | Hit test logical point (8, 8) against prepared geometry, reaching the first tile deep in reverse paint order | 100 | 10 |

Each metric reports the median of the three Divan medians in ns per
iteration. Lower is better, with the default 10% relative regression
threshold and no absolute M2 time budget. The timings cover CPU-side work
on fixed fixtures. Tests use counters to check that paint-only edits skip
layout and contained layout changes skip clean branches.
[Layout tests](../../crates/oxiview-core/src/ui/layout_tests.rs) · [Diagnostics tests](../../crates/oxiview-core/tests/ui_diagnostics.rs)

A new metric without a machine baseline reports `missing_baseline`. A metric
missing from all three parsed rounds reports `missing_measurement`. Both
fail the comparison. Differing metric sets between rounds or an unexpected
metric are output errors. Snapshot acceptance and native visual and input
review are separate acceptance steps. [Benchmark comparison](../../xtask/src/bench.rs) · [M2 acceptance gates](../plans/m2-node-tree-and-layout.md#verification-and-approval-gates)

## M3 reactivity and component benchmarks

`cargo xtask bench --m3` measures four CPU workloads on the thread-local
reactive runtime and a retained UI tree. The benchmark process creates
its own scopes, signals and views without opening a window. Graph
construction, initial mounting and preparation of row-order inputs happen
outside the timed loop. Component workloads apply queued updates through
`UiTree::flush_updates` and check the command outcomes. [Benchmark source](../../benches/components.rs)

| Report metric | One timed iteration | Samples per run | Iterations per sample |
| --- | --- | ---: | ---: |
| `components/batched_memo_diamond` | Write one signal twice in a batch, settle two derived branches and their sum, and read the observer's result | 100 | 100 |
| `components/bound_style_update` | Toggle opacity through a signal bound to one mounted component | 100 | 100 |
| `components/show_mount_and_dispose` | Show a branch with a private signal, apply its mount, then hide it and apply disposal | 100 | 50 |
| `components/keyed_reorder_32` | Reverse 32 mounted rows with bound styles and retain their node identities | 100 | 10 |

The report identifies `scope = "m3"` and writes
`target/xtask/bench-m3-results.json`. Three `bench-m3-*.log` files retain
the raw Divan output. Recording with `--m3 --record` requires human approval.
It updates these four metrics and preserves the existing M1 and M2 values.
[M3 acceptance record](../plans/m3-reactivity-and-components.md#acceptance-record)

## M4 text benchmarks

`cargo xtask bench --m4` uses the pinned Noto Sans and Noto Sans Arabic
fixtures with system font discovery disabled. Font registration happens before
timing. Both paragraph workloads shape and wrap the same mixed-script text at
a width of 188 logical pixels. The rich paragraph defines three spans before
measurement. [Benchmark source](../../benches/text_pipeline.rs)

| Metric under `text_pipeline/` | One timed iteration | Samples × iterations |
| --- | --- | ---: |
| `shape_wrapped_paragraph` | Shape Latin and Arabic text at 16 logical pixels per em | 80 × 64 |
| `shape_rich_paragraph` | Shape the paragraph with weight, size and foreground overrides | 80 × 64 |
| `cold_label_glyphs_2x` | Rasterize the pre-shaped score label at 2× with the glyph image cache budget set to zero | 80 × 32 |
| `warm_label_glyphs_2x` | Prepare the same label at the same subpixel origin with all glyph images resident | 100 × 512 |

The scoped comparison writes `target/xtask/bench-m4-results.json` and three
Divan logs. Each metric uses the median of three run medians and the workspace
default 10% regression threshold. Cache counters verify that the cold path
rasterized glyphs and the warm path reused them. These times measure work on
the CPU. GPU residency is checked separately by rendering repeated frames.

## M5 focus, Actions and themes

`cargo xtask bench --m5` runs three CPU workloads in a retained window tree. Fixture construction, keyboard event construction and theme parsing take place before timing. The report uses the median of three Divan medians and the workspace 10% comparison threshold. [Benchmark source](../../benches/interaction.rs)

| Metric under `interaction/` | One timed iteration |
| --- | --- |
| `tab_256` | Deliver Tab down and up through ordinary dispatch over 256 eligible stops and read the focused identity. |
| `action_eight_contexts` | Resolve F6 from the innermost of eight nested Keymaps, deliver the outer handler and complete key release. |
| `color_token_16_of_256` | Switch light/dark palettes with 16 consumers of the changed color and 240 consumers of an unchanged color. |

The scoped report is `target/xtask/bench-m5-results.json`. Recording updates these three metrics on the current machine after owner approval. [M5 plan and acceptance](../plans/m5-focus-keyboard-and-themes.md)

## M6 editing workloads

`cargo xtask bench --m6` runs the editor workloads in `oxiview-text` without opening a window. The scoped result is `target/xtask/bench-m6-results.json`. It uses the median of three Divan medians and the workspace 10% comparison threshold. [Workload source](../../benches/text_editing.rs)

| Metric under `text_editing/` | Timed operation |
| --- | --- |
| `replace_and_undo_4k` | Replace a selection in a 4 KiB mixed-script document and undo the replacement. |
| `cached_wrapped_geometry_32_lines` | Hit-test and resolve selection rectangles on a cached 32-line layout. |
| `preedit_16_updates_then_commit` | Apply sixteen marked-text updates and commit one candidate. |

## Recording and updating

### Publishing measurements in the READMEs

The English and Chinese READMEs show the latest **five measurement
campaigns in total**, ordered by measurement time. A campaign can include
native benchmarks, animation samples, or both. Multiple samples at one
resolution and duration produce one median. Keep independent before/after
code states in separate campaigns, even when they share a Git HEAD.
The [project history](../performance/history.md) retains every archived run,
its full revision and its measurement artifacts.

Capture the source revision **before** building and running the workload.
Use an empty directory on each run and describe the actual machine, OS,
toolchain and display. For example, from the repository root:

```sh
cargo xtask perf-report --start target/perf-run --machine "Apple M2 Pro (Mac14,10) · 32 GiB · macOS 27.2 · Rust 1.97.0 · 120 Hz Retina (2×)"
cargo xtask bench
cp target/xtask/bench-results.json target/perf-run/bench.json
cargo run --release -p animation-fps -- --objects 128 --measure 5 --json > target/perf-run/animation-windowed-1.json
cargo run --release -p animation-fps -- --objects 128 --measure 5 --fullscreen --json > target/perf-run/animation-fullscreen-1.json
cargo xtask perf-report --add target/perf-run
cargo xtask perf-report --check
```

Include the workloads you measured. Save each animation repeat or longer
observation under its own `animation-*.json` filename in that directory.
`run.json` captures the full HEAD, its clean/dirty state and the measurement
start time. Its `label` holds an English and a Chinese caption. The optional
`implementation` field identifies the later commit that landed a measured
working-tree change. The archive keeps distinct campaigns at the same HEAD.

`--add` copies the reviewed inputs into `docs/performance/runs/` and refreshes
the Markdown. Repeating the import with identical files is safe. To regenerate
from existing records, run `cargo xtask perf-report`. Ordinary verification
checks that both README excerpts and the archive match those records. Performance
baseline updates still use the separate human-confirmed `bench --record` workflow.

### Updating the regression baseline

1. Close heavy workloads and confirm the current code and benchmark inputs are as intended.
2. After human confirmation, use `cargo xtask bench --record` for the full suite or add `--m2`, `--m3` or `--m4` to record only that milestone. Review and commit the approved changes to `perf/baselines/<machine-id>.json`. Default bench and verify never write baselines.
3. Immediately repeat the comparison for the same scope to confirm reproducibility. Investigate code and environment on regression. Do not repeatedly record to hide failures.
4. Full recording preserves each metric's `regression_percent` (default 10, individually configurable from 0–100), except the fixed 15% pixel margin, while updating values, machine description, and recording time. Scoped recording updates the selected milestone's values and preserves the existing machine description, recording time, thresholds, and other entries. A first scoped recording creates the machine baseline with the selected values and pending hard-target entries. Full comparison requires baselines for all full-suite measurements. Missing measurements within the selected scope also fail in recording mode. Deleting or renaming metrics requires human review of baseline changes.

The machine identifier uses a fixed FNV-1a 64-bit hash of architecture, model, CPU name, and memory bytes, prefixed with the operating system. macOS reads `hw.model`, `machdep.cpu.brand_string`, and `hw.memsize`; if sandboxing denies access, it uses corresponding fields from `system_profiler SPHardwareDataType -detailLevel mini -json` (mini excludes serial numbers and UUIDs). Windows uses CIM model/CPU/physical memory; Linux uses the DMI model, `/proc/cpuinfo`, and `/proc/meminfo`. It reads no serial numbers, usernames, or hostnames. OS and rustc versions are recorded in the description but excluded from the hash, so repeated runs yield the same ID.

Machines with identical hardware configurations share an identifier; distinguishing them cannot be guaranteed without unique private device identifiers. Memory upgrades create a new ID. Linux MemTotal is kernel-available memory, so kernel/boot configuration changes may also change the ID. Unsupported probing environments return explicit errors instead of falling back to an unknown ID. Windows/Linux implementations still need testing on their native platforms.

Files use schema_version=1 and UTC Unix seconds. Metrics record value, unit, direction, regression_percent, and target. Native latency metrics additionally record refresh_interval_ms. Older schema-1 files remain readable: a missing input_to_submit_ms or pixel refresh interval is a missing baseline (exit 1), requiring human-confirmed recording. Default bench never changes values. The M7 metric remains null.

input_to_submit_ms stores capture-to-submission p95, with a hard target of one measured refresh interval. input_to_pixel_ms stores capture-to-actual-presentation p95 and has a fixed 15% floating regression margin against the human-recorded optimized implementation. There is no absolute one-frame pixel gate. A missing pixel baseline yields a null hard_check_pass, not a passing gate. Recording stores the display interval; a subsequent measurement differing by more than 0.1% (to allow display-clock rounding/jitter) is an environment error (exit 2): use the same display or re-record after human confirmation. Recording is the explicit operation that can change that display. The idle hard target remains 0.1% with zero submitted/presented frames and display-link events.

## Decision rules

- lower: fail only if current minus baseline exceeds `baseline × threshold / 100`; higher uses baseline minus current. Exactly meeting the threshold passes, as do improvements. A zero baseline allows no change in the worse direction.
- No baseline for this machine, a new metric without a baseline, or an unmeasured recorded metric: fail with exit code 1. Missing baselines prompt use of `--record` after human confirmation, avoiding claims of verification without comparison.
- Hard-target failures fail regardless of baseline or recording mode. A failed hard target prevents recording. Regressions and nonzero Divan subprocess exits: fail with exit code 1. Environment, file, or parsing errors use exit code 2, matching xtask conventions.
- Other machines' baselines are shown as raw log content only, without validation or comparison. Current system and toolchain descriptions also appear in the log; a human should assess whether to record again after system/toolchain changes.

## Hard-target integration plan

| Metric | Hard target | Current status | Measurement |
| --- | --- | --- | --- |
| `input_to_submit_ms` | p95 ≤ one refresh interval | measured (M1) | capture to endpoint-clock timestamp immediately after presenter returns Presented |
| `input_to_pixel_ms` | p95 ≤ human-recorded optimized baseline × 1.15 | measured (M1) | 120 sequential synthetic AppKit pointer moves; capture to actual `MTLDrawable.presentedTime`; stored value is p95 milliseconds |
| `idle_cpu_percent` | zero presented frames, zero display-link frame events, and process CPU ≤ 0.1% | measured (M1) | thirty-second idle interval after one-second settling; `getrusage(RUSAGE_SELF)` user + system CPU divided by elapsed wall time × 100 |
| `list_100k_scroll_dropped_frames` | 0 frames | pending until M7 | 100,000-row list scrolling |

Measured means a real source and enforced check exist; it does not assert that the current implementation meets the target.

After all three Divan rounds, bench runs `cargo run --release -p window -- --measure-latency 120 --json` three times, then `cargo run --release -p window -- --idle-check 30 --json`. Submit and pixel metrics independently use the median of the three p95 values; each round must still pass the submit hard target, and all rounds must use the same display interval. Reports preserve all three rounds. Recording uses exactly the same aggregation. These measurements never overlap each other or Divan. Both native modes require macOS 14+, a visible WindowServer display session and a Metal GPU. Missing native facilities, malformed/missing measurements, timeout, and unsupported non-macOS hosts are environment errors (exit 2), never skips or passes. The subprocess guard bounds each cargo invocation to 180 seconds.

Diagnostic windows are centered explicitly on the primary display (the first
`NSScreen`), independently of keyboard focus. Native logs identify that display
and its maximum refresh rate; measured display-link intervals still determine
compatibility with the baseline. Ordinary interactive windows keep AppKit's
default placement. Configure the primary display to match the recorded interval
before benchmarking; neither moving focus nor attaching a second monitor should
silently select a different measurement display.

Latency uses one warm-up pointer frame after one-second settling to obtain the display-link interval. For sample i, the phase is `((i × 7) mod 20) / 20` of a refresh interval. Posting is scheduled two whole intervals plus that fraction beyond the latest elapsed cycle extrapolated from the preceding acquired frame's slot vsync (excluding its compositor-offset prediction). This sweeps twenty phases six times for 120 samples, without entropy; OS scheduling can delay an intended posting moment. Every position differs from the preceding one. Only one synthetic event is outstanding: its capture timestamp must match the consuming frame's `LatencyRecord`, and that frame's drawable identity must report a nonzero actual presentation time before the next input. A changed display interval invalidates the run.

Capture timestamps and Metal presentation times share the backend host-uptime clock. The endpoint is actual on-screen presentation, not GPU completion, submission, or a display-link prediction. These synthetic events enter AppKit's queue inside the process, bypassing HID and WindowServer input delivery. Hardware input latency is not included. Injection captures time before queuing the command, then acknowledges delivery from the view callback using a diagnostic event number. The returned timestamp is exactly the value delivered to core: AppKit can quantize the floating-point timestamp by a nanosecond when the event crosses its queue. The worker waits at most five seconds without holding the window queue lock; calling injection on the main thread is rejected. This preserves strict frame attribution without moving the capture endpoint or accepting approximate matches. Percentiles use nearest rank; JSON reports samples, pixel and submit p50/p95/max milliseconds, refresh interval milliseconds, pixel p95 frames, compositor-offset estimate, and the inclusive submit hard-check result. The pixel gate is evaluated only by xtask against a compatible recorded baseline. [ADR-0026](../adr/0026-m1-native-latency-gates.md) records the accepted M1 gate adjustment.

Idle measurement sleeps without polling after settling. The example counts all pointer/button/key events delivered by the framework; any event during the interval returns environment error 2, "user input during idle measurement; rerun without touching the mouse". Events still reach the framework and application normally. No synthetic inputs are posted in idle mode. Presentation records count actual on-screen frames within the interval; the existing submitted-frame counter is also required to stay unchanged, preventing unpresented work from passing. Display-link frame events must remain zero. CPU includes all process threads and is not divided by CPU core count. JSON reports presented and submitted frames, display-link events, CPU percentage, actual wall interval, and the combined hard-check result. xtask rejects intervals shorter than thirty seconds. The longer interval reduces the influence of small startup housekeeping bursts on near-zero CPU percentages; it does not establish an application CPU optimization. The earlier five-second candidate remains a historical baseline with the same CPU unit, 0.1% hard ceiling and 10% relative tolerance; it is not rewritten to obtain a pass.

Production macOS drawable tracking maintains a per-window compositor-offset estimate without idle polling. It uses presentedTime minus the associated slot vsync, an exponential moving average (1/8 new sample), defaults to one interval, and bounds the estimate to four intervals. Metal presented handlers capture only shared feedback and the acquisition slot; tracking never retains drawables. Handlers may run on arbitrary threads, so feedback uses atomics and a mutex-protected diagnostics queue. Zero timestamps leave the estimate unchanged and produce distinct not-displayed diagnostics records so latency measurement can fail immediately. The `diagnostics` Cargo feature is off by default. The window example and the [continuous animation demo](../../examples/animation-fps/README.md) enable it for native measurements. `MacOsPlatform::with_diagnostics` opts into measurement record collection. Diagnostics expose backend clock time, timestamped synthetic pointer posting, drawable IDs/presentation records, and process CPU time without exposing native objects. They work in release builds and are not product APIs.

Ordinary verify opens no measurement window. Its headless tests use deterministic virtual input phases, assert capture-to-submission latency ≤ one interval, and assert zero frames across 1,000 subsequent idle intervals. xtask self-tests cover strict native JSON, inclusive hard boundaries, fabricated regressions, environment exit codes, and the pending M7 protocol.


## Native presentation behavior

Idle requests use the next unused display slot immediately from the main command source. Further requests in the same slot arm the display link; it remains paused with no callbacks while idle. Startup calibrates phase with a display-link tick. Frame and resize predictions add the production compositor estimate to slot vsync. Windowed surfaces prefer Immediate, fullscreen uses Fifo, and unsupported Immediate falls back to Fifo. Both modes request maximum frame latency 2, which wgpu's Metal backend maps to three drawables. Reducing the pool to two after fullscreen can make Immediate acquisition block for a refresh interval.

For ordinary frames the renderer acquires one actual surface texture asynchronously in both presentation modes. During the wait the window thread continues handling input. The ready signal resumes painting with current application state. Submission and display rates remain separate measurements. A positive Metal presentation timestamp counts as on-screen feedback, and a zero timestamp identifies a drawable that was never displayed. Resize transactions still configure, present and acknowledge synchronously. See [ADR-0036](../adr/0036-fifo-drawable-readiness.md) for ownership, cancellation and the scope of paint-to-submit timing.

For a sustained animation workload, run the windowed and fullscreen commands
in the [FPS demo](../../examples/animation-fps/README.md). Its JSON pairs
actual displayed FPS and P95 display gap with physical size, scale and
sample duration. This is a standalone diagnostic. The `xtask bench` gates
above use the M1 pointer and idle workloads.

The hosting layer has a process-lifetime delegate returning nil for window. This bypasses wgpu-hal 30.0.2's metal/surface.rs hosting_window/acquire_texture occlusion check (gfx-rs/wgpu#8309), which misclassifies visibly rendering fullscreen windows on the tested macOS 27.2 process. Truly occluded nextDrawable may block until its timeout; nil becomes wgpu Timeout, FrameOutcome::Skipped, and a retained pending frame retried by core. The existing skipped-frame tests exercise that retry path.

Fullscreen input-to-pixel latency baselines remain deferred to M10. The example's
`--fullscreen-check` now exercises sustained input in windowed, fullscreen and
restored-window stages. Each stage calibrates its refresh interval, warms up for
one second, then measures two seconds while posting input at twice refresh rate.
Both submissions and actual nonzero `presentedTime` feedback must reach at least
80% of refresh rate; each p95 frame gap must be at most 1.5 refresh intervals.
These local cadence tolerances catch drawable-pool serialization and dropped
presentations; they do not relax `input_to_submit_ms`'s separate hard gate.

The fullscreen transition must also report an on-screen transition frame without
pointer input. Missing presentation feedback returns an environment error rather
than silently passing. `--debug-overlay` is permitted with this check so the
actual overlay workload is covered, but remains forbidden with the latency and
idle baseline modes. JSON reports submitted/displayed rates, gap p95 and
paint-to-submit p95, capture-to-submit p50/p95 and the percentage of input samples
above one refresh interval for all three stages. `cadence_pass`,
`paint_submit_within_refresh` and `input_submit_within_refresh` are distinct
results; a stable display rate does not establish low input latency. The check
returns exit 1 for a cadence or paint-to-submit budget failure. Continuous-input
latency overruns remain visible in the report; the separate, phase-swept
`--measure-latency` command enforces M1's input-to-submit hard target. Pixel coverage and smooth transitions
still require human observation. Transaction-synchronized presentation is enabled
only during interactive live resize; fullscreen reparenting uses normal
presentation. After screen/fullscreen transition callbacks, the backend requests
one normal repaint because AppKit can discard transition drawables. This keeps
the final image from waiting for new input, without starting an idle frame loop.
