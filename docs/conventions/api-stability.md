# API stability

[ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) makes all internal crates unstable, inheriting workspace `version = "0.0.0"` and `publish = false`. Cross-context interfaces must still use owned types; instability does not permit third-party type leakage.

## Public facade

The future public `oxiview` facade has two stability tiers. These are release constraints; this step creates no facade or features.

- **stable**: public APIs explicitly covered by the stability commitment and not gated by unstable features, maintained under release compatibility rules. Ordinary features may control compilation of stable capabilities but do not waive compatibility commitments.
- **unstable**: requires explicit opt-in through a disabled-by-default `unstable-<domain-or-capability>` feature, using lowercase English and hyphens (for example `unstable-layout`). Document the reason for instability. These APIs may change in minor releases without compatibility guarantees. Stable API signatures must not depend on types available only through unstable features.

Under [roadmap principle 2](../roadmap.md), abstractions with only one implementation are unstable. Consider stabilization only after a second implementation and tests demonstrate the boundary; future replaceability of an internal implementation is not verification. The 0.0.0 placeholder on crates.io is neither the framework's first release nor a stable API commitment.

Encapsulate third-party types in anti-corruption layers. The shared wgpu device in [ADR-0013](../adr/0013-external-content-compositing.md) and re-export option in [ADR-0022](../adr/0022-foundation-libraries.md) conflict with S6's strict public API boundary. These conventions conservatively require encapsulation and introduce no public exceptions. Make an explicit decision before implementing those capabilities; this step does not rewrite historical ADRs.

## Deprecation and release preparation

Deprecate stable APIs with `#[deprecated(since = "version", note = "replacement and migration instructions")]`. Provide alternatives in API documentation and release migration notes. Retain the API for at least one full subsequent minor release, then remove it only at a version boundary permitting breaking changes. Changes to internal unstable and public unstable APIs still need migration guidance but do not promise the same retention period.

Before the first actual framework release, introduce `cargo-semver-checks`, compare stable APIs against the previous release, and add an installation check to `cargo xtask doctor`. The tool is not currently installed or integrated; doctor checks only Rust and cargo-deny. This step neither installs it nor adds checks, and must not claim compatibility validation that was not performed.
