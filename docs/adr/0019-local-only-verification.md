---
status: accepted
---

# Verification: local only, with cargo xtask as the sole entry point and no CI

There is currently no resource budget for CI. All verification runs locally on the macOS development machine. `cargo xtask` is the sole entry point: formatting, linting, tests, pixel snapshots, benchmarks and performance metrics, dependency review, and dependency-direction checks all use xtask subcommands. Humans and agents run the same commands. Until CI exists, interpret every roadmap or ADR reference to integration into CI as integration into `cargo xtask verify`.

## Consequences

- Baselines for ADR-0001's hard performance targets depend on local hardware. Baseline files must record machine model and OS version; cross-machine comparisons are informational only.
- Without server-side enforcement, AGENTS.md requires `cargo xtask verify` to pass before commits. A local Git pre-push hook can supplement this.
- Future CI need only invoke the same xtask entry point, without duplicating verification logic in CI configuration.
