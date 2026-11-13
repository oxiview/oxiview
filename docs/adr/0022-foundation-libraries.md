---
status: accepted
---

# Foundation library choices

Use these foundation libraries consistently across the workspace:

| Purpose | Choice | Constraints |
|---|---|---|
| Errors | `thiserror` | No `anyhow` in library crates; limit it to xtask and examples |
| Observability | `tracing` | Critical frame, layout, painting, and event-dispatch paths must have spans |
| Geometry and brushes | `kurbo`, `peniko` | Align with Vello; re-export or wrap in `oxiview-base` |
| Structural snapshots | `insta` | `oxiview-testing` implements pixel comparisons itself |
| Benchmarks | `divan` for microbenchmarks, `criterion` when statistical reports are needed | xtask manages baseline results |
| Property testing | `proptest` | |
| Fuzzing | `cargo-fuzz` | One of the few permitted nightly use cases |
| macOS bindings | `objc2` family (`objc2-app-kit`, `objc2-metal`, `objc2-quartz-core`, and others) | Allowed only in `oxiview-platform-macos` |
| Dependency review | `cargo-deny` | Check licenses, advisories, and duplicate versions locally through `cargo xtask` |

Review new foundation dependencies under the dependency-management conventions. Choices with broad impact require an ADR.
