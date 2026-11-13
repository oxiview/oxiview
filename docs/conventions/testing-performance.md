# Benchmarks and performance baselines

`cargo xtask bench` is the workspace performance verification entry point. Ordinary `cargo xtask verify` skips benchmarks; `cargo xtask verify --full` adds bench after ordinary checks and continues aggregating even if earlier checks fail (doctor environment failures still return early). Use Rust stable 1.97 and edition 2024 only; nightly is unnecessary.

## Writing benchmarks

The default is [Divan 0.1.21](https://github.com/nvzqz/divan), a maintained project licensed MIT OR Apache-2.0 with MSRV 1.80. This repository pins the exact version; upgrades must revalidate output parsing. Added transitive dependencies mainly include divan-macros, condtype, regex-lite, terminal_size, and their system dependencies. `cargo xtask deps` reviews licenses, advisories, and duplicate versions.

Define versions only in root `[workspace.dependencies]`; crates inherit `divan.workspace = true` in dev-dependencies. Under ADR-0021, workspace example source lives in root `benches/compare.rs`, referenced by the `oxiview-testing` `[[bench]]` target with `harness = false`. Follow this convention for new targets; do not create another framework crate.

The example measures S4 `compare` on identical 256×256 and 1024×1024 gradient RGBA8 images, including per-pixel comparison and diff-image allocation. Image construction is outside the timed region. Use black_box to preserve real computation and let Divan consume the result. Benchmarks may use expect; library code still prohibits unwrap/expect/panic.

## Measurement and parsing

xtask runs `cargo bench --workspace --benches --locked` sequentially three times using the optimized bench profile. The example uses 100 samples per run and 10 iterations per sample. Read Divan's median from each run, then take the median of those three values as the final result in ns. This is not a confidence interval or statistical significance test. Keep power, cooling, build configuration, and background load stable; do not run other performance tests concurrently.

Divan currently emits terminal statistics tables. The parser validates headers and column counts, reconstructs tree names as `target/function/argument`, and supports ps/ns/µs/μs/us/ms/s. It requires finite positive values and nonzero sample counts, rejecting duplicate metrics, empty output, and inconsistent metric sets across runs. Clear inherited `DIVAN_*` overrides so filtering or test mode cannot alter measurement. New benchmarks use whitespace-free names without counter rows or custom output. Format changes require parser and test updates; parsing failure must never count as success. Standard libtest output with zero benchmarks is ignored.

Reports go to `target/xtask/report.json`. `performance.metrics` includes each current value, baseline, status, and threshold; hard-target metrics show `pending`. `bench-results.json` is a standalone artifact with the same details. `bench.log` records the decision summary; `bench-1.log` through `bench-3.log` preserve raw stdout/stderr. Every bench run first removes these old artifacts to avoid referencing stale measurements after failure. `--json` prints only the report to stdout.

## Recording and updating

1. Close heavy workloads and confirm the current code and benchmark inputs are as intended.
2. Run `cargo xtask bench --record` **only after human confirmation**. The command explicitly reports creation or replacement of `perf/baselines/<machine-id>.json`, which must be reviewed and committed. Default bench and verify never write baselines.
3. Immediately run `cargo xtask bench` to confirm reproducibility. Investigate code and environment on regression; do not repeatedly record to hide failures.
4. Updating a baseline preserves each metric's `regression_percent` (default 10, individually configurable from 0–100) while updating values, machine description, and recording time. Missing recorded metrics fail even with record. Deleting or renaming metrics requires human review of baseline changes.

The machine identifier uses a fixed FNV-1a 64-bit hash of architecture, model, CPU name, and memory bytes, prefixed with the operating system. macOS reads `hw.model`, `machdep.cpu.brand_string`, and `hw.memsize`; if sandboxing denies access, it uses corresponding fields from `system_profiler SPHardwareDataType -detailLevel mini -json` (mini excludes serial numbers and UUIDs). Windows uses CIM model/CPU/physical memory; Linux uses the DMI model, `/proc/cpuinfo`, and `/proc/meminfo`. It reads no serial numbers, usernames, or hostnames. OS and rustc versions are recorded in the description but excluded from the hash, so repeated runs yield the same ID.

Machines with identical hardware configurations share an identifier; distinguishing them cannot be guaranteed without unique private device identifiers. Memory upgrades create a new ID. Linux MemTotal is kernel-available memory, so kernel/boot configuration changes may also change the ID. Unsupported probing environments return explicit errors instead of falling back to an unknown ID. Windows/Linux implementations still need testing on their native platforms.

Files use schema_version=1 and UTC Unix seconds. Metrics record value, unit, direction (lower is better or higher is better), regression_percent, and target. This step's microbenchmarks all use lower; hard-target value=null means unmeasured, not zero.

## Decision rules

- lower: fail only if current minus baseline exceeds `baseline × threshold / 100`; higher uses baseline minus current. Exactly meeting the threshold passes, as do improvements. A zero baseline allows no change in the worse direction.
- No baseline for this machine, a new metric without a baseline, or an unmeasured recorded metric: fail with exit code 1. Missing baselines prompt use of `--record` after human confirmation, avoiding claims of verification without comparison.
- Regressions and nonzero benchmark subprocess exits: fail with exit code 1. Environment, file, or parsing errors use exit code 2, matching xtask conventions.
- Other machines' baselines are shown as raw log content only, without validation or comparison. Current system and toolchain descriptions also appear in the log; a human should assess whether to record again after system/toolchain changes.

## Hard-target integration plan

| Metric | Target | Current status | Integration milestone |
| --- | --- | --- | --- |
| `input_to_pixel_ms` | 8.0 ms | pending | M1 adds real input-to-presentation measurement |
| `idle_cpu_percent` | 0.0 % | pending | M1 adds idle frame policy and CPU measurement |
| `list_100k_scroll_dropped_frames` | 0 frames | pending | M7 adds 100,000-row list scrolling measurement |

These three metrics reserve targets only; they neither fabricate measurements nor affect this step's pass/fail result. Integration must add real measurement sources, unit and direction validation, hard-target checks, and corresponding self-tests together, then upgrade the baseline protocol. The current protocol rejects manually populated measured values for them. This step implements no windows, rendering, frame policies, or widgets.
