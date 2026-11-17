# Benchmarks and performance baselines

`cargo xtask bench` is the workspace performance verification entry point. Ordinary `cargo xtask verify` skips benchmarks; `cargo xtask verify --full` opens real macOS windows for latency and idle measurements and adds bench after ordinary checks and continues aggregating even if earlier checks fail (doctor environment failures still return early). Use Rust stable 1.97 and edition 2024 only; nightly is unnecessary.

## Writing benchmarks

The default is [Divan 0.1.21](https://github.com/nvzqz/divan), a maintained project licensed MIT OR Apache-2.0 with MSRV 1.80. This repository pins the exact version; upgrades must revalidate output parsing. Added transitive dependencies mainly include divan-macros, condtype, regex-lite, terminal_size, and their system dependencies. `cargo xtask deps` reviews licenses, advisories, and duplicate versions.

Define versions only in root `[workspace.dependencies]`; crates inherit `divan.workspace = true` in dev-dependencies. Under ADR-0021, workspace example source lives in root `benches/compare.rs`, referenced by the `oxiview-testing` `[[bench]]` target with `harness = false`. Follow this convention for new targets; do not create another framework crate.

`benches/compare.rs` measures S4 `compare` on identical 256×256 and 1024×1024 gradient RGBA8 images, including per-pixel comparison and diff-image allocation. Image construction is outside the timed region. `benches/render.rs`, owned by `oxiview-render-wgpu`, renders 1, 128 and 1,000 sparse rounded rectangles on a 1280×840 target. Its 20 single-iteration samples include scene encoding, GPU completion and RGBA readback; scene construction and device/pipeline warm-up are outside measurement. This guards against evaluating every primitive at every pixel. It requires a native GPU and is not an input-to-pixel latency measurement. Use black_box to preserve real computation and let Divan consume the result. Benchmarks may use expect; library code still prohibits unwrap/expect/panic.

## Measurement and parsing

xtask runs `cargo bench --workspace --benches --locked` sequentially three times using the optimized bench profile. The example uses 100 samples per run and 10 iterations per sample. Read Divan's median from each run, then take the median of those three values as the final result in ns. This is not a confidence interval or statistical significance test. Keep power, cooling, build configuration, and background load stable; do not run other performance tests concurrently.

Divan currently emits terminal statistics tables. The parser validates headers and column counts, reconstructs tree names as `target/function/argument`, and supports ps/ns/µs/μs/us/ms/s. It requires finite positive values and nonzero sample counts, rejecting duplicate metrics, empty output, and inconsistent metric sets across runs. Clear inherited `DIVAN_*` overrides so filtering or test mode cannot alter measurement. New benchmarks use whitespace-free names without counter rows or custom output. Format changes require parser and test updates; parsing failure must never count as success. Standard libtest output with zero benchmarks is ignored.

Reports go to `target/xtask/report.json`. `performance.metrics` includes each current value, baseline, status, and threshold; the three M1 metrics include `hard_check_pass` and measurement details (including latency percentiles and frames); the M7 list metric shows `pending`. `bench-results.json` is a standalone artifact with the same details. `bench.log` records the decision summary; `bench-1.log` through `bench-3.log` preserve raw stdout/stderr. Native stdout and stderr are preserved in `bench-latency-1.json` through `bench-latency-3.json` / matching `.log` files and `bench-idle.json` / `bench-idle.log`. Every bench run first removes these old artifacts to avoid referencing stale measurements after failure. `--json` prints only the report to stdout.

## Recording and updating

1. Close heavy workloads and confirm the current code and benchmark inputs are as intended.
2. Run `cargo xtask bench --record` **only after human confirmation**. The command explicitly reports creation or replacement of `perf/baselines/<machine-id>.json`, which must be reviewed and committed. Default bench and verify never write baselines.
3. Immediately run `cargo xtask bench` to confirm reproducibility. Investigate code and environment on regression; do not repeatedly record to hide failures.
4. Updating a baseline preserves each metric's `regression_percent` (default 10, individually configurable from 0–100), except the fixed 15% pixel margin, while updating values, machine description, and recording time. Missing recorded metrics fail even with record. Deleting or renaming metrics requires human review of baseline changes.

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

Production macOS drawable tracking maintains a per-window compositor-offset estimate without idle polling. It uses presentedTime minus the associated slot vsync, an exponential moving average (1/8 new sample), defaults to one interval, and bounds the estimate to four intervals. Metal presented handlers capture only shared feedback and the acquisition slot; tracking never retains drawables. Handlers may run on arbitrary threads, so feedback uses atomics and a mutex-protected diagnostics queue. Zero timestamps leave the estimate unchanged and produce distinct not-displayed diagnostics records so latency measurement can fail immediately. The `diagnostics` Cargo feature is off by default and enabled only by the window example. `MacOsPlatform::with_diagnostics` opts into measurement record collection. Diagnostics expose backend clock time, timestamped synthetic pointer posting, drawable IDs/presentation records, and process CPU time without exposing native objects. They work in release builds and are not product APIs.

Ordinary verify opens no measurement window. Its headless tests use deterministic virtual input phases, assert capture-to-submission latency ≤ one interval, and assert zero frames across 1,000 subsequent idle intervals. xtask self-tests cover strict native JSON, inclusive hard boundaries, fabricated regressions, environment exit codes, and the pending M7 protocol.


## Native presentation behavior

Idle requests use the next unused display slot immediately from the main command source. Further requests in the same slot arm the display link; it remains paused with no callbacks while idle. Startup calibrates phase with a display-link tick. Frame and resize predictions add the production compositor estimate to slot vsync. Windowed surfaces prefer Immediate, fullscreen uses Fifo, and unsupported Immediate falls back to Fifo. Both modes request maximum frame latency 2, which wgpu's Metal backend maps to three drawables. Reducing the pool to two after fullscreen can make Immediate acquisition block for a refresh interval.

Before issuing a FIFO frame opportunity, the macOS backend checks outstanding drawable presentations. At two outstanding drawables it leaves the request pending and pauses the display link. A presentation callback frees capacity and wakes the main command source only if a frame was deferred. The window thread continues processing input while waiting, then paints once capacity is available. Waiting inside `nextDrawable` after painting instead froze that frame's input state for another refresh interval. Skipped presentations also free capacity; concurrent callbacks retire their own acquisitions without assuming callback order. The callback stores a weak wake target and never retains a drawable. Resize acknowledgements still bypass ordinary frame opportunities. The surface uses wgpu's default alpha configuration.

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
