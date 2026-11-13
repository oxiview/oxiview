---
name: verify
description: Use after routine, rendering, or performance changes, or when verification, snapshots, or performance baselines fail; select xtask commands and interpret reports.
---

# Verification

Run all commands from the repository root. Verify locally only.

## Select commands

- During routine iteration, run `cargo xtask fmt`, `cargo xtask lint`, or `cargo xtask test` as needed; run `cargo xtask deps` for dependency changes.
- Before delivery, run `cargo xtask verify`, which includes doctor, fmt, lint, test, deps, and docs. docs has no standalone subcommand.
- For rendering changes, run `cargo xtask test`, inspect relevant pixel snapshots, then run verify. M-1 has only harness self-tests with generated images; do not claim real rendering was verified.
- For performance changes, run `cargo xtask bench` or `cargo xtask verify --full`; ordinary verify skips benchmarks. See the [benchmark conventions](../../../docs/conventions/testing-performance.md) for measurement requirements.
- For environment issues, run `cargo xtask doctor`. Report installation commands for missing tools; do not install automatically. For arguments, use `cargo xtask --help` or `cargo xtask help <subcommand>`.

## Read results

Each invocation overwrites `target/xtask/report.json`; preserve results needed for reporting before the next run. Read each check's `name`, `status`, `duration_ms`, `failure_summary`, and `artifacts`, then inspect `target/xtask/<check-name>.log` as needed. `--json` restricts xtask stdout to JSON.

Exit codes: 0 for success; 1 for failed checks; 2 for environment, argument, or xtask errors (takes precedence over 1). verify returns early if doctor fails; other failures are aggregated. Benchmark details are in `performance` and `bench-results.json`, with raw measurements in `bench-1.log` through `bench-3.log`.

## Snapshot failures

1. Run `cargo xtask snapshot --review`; read the reasons and artifact paths.
2. Show the human `actual.png` and `diff.png` and explain the differences. A missing baseline also requires confirmation.
3. Fix unintended changes first. Run `cargo xtask snapshot --accept <name>` only after explicit human confirmation; use `<crate>/<name>` for duplicate names. `--accept all` requires confirmation of every pending item.
4. Rerun tests and verify, then review baseline changes. See the [pixel snapshot conventions](../../../docs/conventions/testing-snapshots.md).

## Performance regressions

First investigate code, benchmark inputs, machine load, and toolchain differences. Run `cargo xtask bench --record` only after the human confirms the change is expected; then run `cargo xtask bench` to confirm reproducibility and review the baseline. A missing initial baseline also requires human confirmation. Never use accept or record to hide failures.

Report the actual commands, exit codes, failure reasons, and unverified areas. Do not report pending metrics as passing.
