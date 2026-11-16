# oxiview

**English** | [简体中文](README_ZH.md)

A self-drawn Rust UI framework exclusively for native macOS, Windows, and Linux platforms.

M1 delivers the first native macOS window, a deterministic headless platform,
on-demand scheduling on a dedicated window thread, and GPU surface presentation
with an SDF rounded-rectangle path. M2, node trees and layout, is next.
M1 close-out remains in progress: native visual checks and ordinary verification
pass, but performance baseline reproducibility and full verification are pending.
The toolchain is pinned to Rust stable 1.97.0 with edition 2024.

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

See the [M1 plan](docs/plans/m1-first-window.md) for scope and remaining items,
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

Windows and Linux remain planned ports. Widgets, text layout, hit testing and
reactivity are later milestones; this demo exercises rendering and scheduling.
