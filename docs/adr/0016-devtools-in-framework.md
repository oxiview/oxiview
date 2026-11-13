---
status: accepted
---

# Developer tools: hot reload, Inspector, profiling, headless snapshots, and event replay as framework features

The framework owns the following capabilities rather than delegating them to third parties:

- **Hot reload**: theme and asset reload (ADR-0008); binary hot patching for Rust code using tools such as subsecond, preserving signal state where possible and rebuilding the affected window if preservation fails.
- **Inspector**: node tree, computed styles, layout-box overlays, and the **signal dependency graph** showing which signals triggered which updates, addressing fine-grained reactivity's debugging difficulty (ADR-0005).
- **Performance panel**: frame timings by stage (style, layout, painting, GPU), with Tracy / Perfetto trace export.
- **Headless rendering and pixel snapshot tests**: render without windows and compare against baselines pixel by pixel, with injectable input for E2E tests. This also supports ADR-0001's hard performance targets in CI.
- **Event recording and replay**: record input sequences and replay deterministically to reproduce bugs.

## Consequences

- Deterministic replay requires injectable or replaceable frame clocks, randomness, and async scheduling. Framework internals must not read system time directly.
- Headless rendering relies on the CPU fallback renderer (ADR-0012) producing stable CI output. Pixels must either match across platforms or use separate platform baselines.
