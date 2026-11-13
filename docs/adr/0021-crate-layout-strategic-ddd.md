---
status: accepted
---

# Directories and crates: flat crates/, oxiview- prefix, strategic DDD, ports/adapters, and milestone-based creation

**Directories**: all crates live directly under `crates/`, with directory names exactly matching crate names. An `oxiview-` prefix plus subsystem name expresses hierarchy, following rust-analyzer, Bevy, and wgpu. Examples, cross-crate integration tests, benchmarks, fuzz targets, xtask, and documentation live in root `examples/`, `tests/`, `benches/`, `fuzz/`, `xtask/`, and `docs/`. Internal crates use version `0.0.0` and `publish = false` until publication is explicitly chosen.

**Strategic DDD**: each subsystem is a **bounded context** comprising one or more crates, such as reactivity, rendering, text, layout, platform, UI core, and widgets. Dependency direction between contexts forms the **context map**, documented in `docs/architecture.md` and checked by `cargo xtask`. Third-party types (Taffy, Parley, Vello, objc2) must not appear in project public APIs; convert them through the owning context's **anti-corruption layer**. Stable data types shared across contexts, such as Scene, belong in independent **published-language** crates. Do not introduce tactical DDD entities, aggregates, or repositories, which serve business domains rather than technical frameworks.

**Ports and adapters**: organize platform and rendering as ports (traits) and adapters (implementations). For example, `oxiview-platform` is a port; `oxiview-platform-macos` and `oxiview-platform-headless` are adapters. Only the **composition root** (`oxiview-app`) can access both ports and concrete adapters. The public `oxiview` facade re-exports from the composition root according to features.

**Creation timing**: M-1 establishes only the dependency rules and checker, which skips nonexistent crates. Create each crate when first needed in its milestone, without empty shells in advance.

## Considered Options

- **Nested layer directories** (`crates/platform/macos/`): intuitive hierarchy, but creates a second hierarchy alongside Cargo's flat namespace that can drift over time.
- **Separate libs/ and crates/**: clear publishable/internal boundary; defer until publication is actually chosen.
- **Create every crate shell at once**: establishes boundaries early but leaves many empty crates.

## Consequences

- As contexts multiply, split the glossary into `GLOSSARY-MAP.md` and per-context `GLOSSARY.md` files.
