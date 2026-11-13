# oxiview

**English** | [简体中文](README_ZH.md)

A self-drawn Rust UI framework exclusively for native macOS, Windows, and Linux platforms.

The project is at the M-1 engineering baseline milestone. Framework features have not been implemented yet.
The toolchain is pinned to Rust stable 1.97.0 with edition 2024.

Run all local checks with:

```sh
cargo xtask verify
```

See the [M-1 plan](docs/plans/m-1-engineering-baseline.md) for scope and
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

The workspace currently contains `xtask` and the `oxiview-testing` test harness.
Framework crates will be added in later milestones.
