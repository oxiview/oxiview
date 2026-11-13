---
name: review-checklist
description: Use when reviewing oxiview changes or checking dependencies, safety boundaries, public APIs, tests, and observability before a commit.
---

# Code review

Identify the change scope and authorized milestone. Give findings or an applicability assessment for every item; M-1 rules do not authorize early implementation of framework features.

- **Dependency direction**: run `cargo xtask deps`; check context, port, adapter, and composition root boundaries against [layers.toml](../../../xtask/layers.toml). Review new dependencies under the [dependency conventions](../../../docs/conventions/dependencies.md).
- **Unsafe**: check allowed scope under the [safety conventions](../../../docs/conventions/unsafe-panic-errors.md). Each English `// SAFETY:` comment explains invariants and their guarantees; unsafe functions have `# Safety`. Check FFI, lifetimes, and thread affinity.
- **Panic**: library production paths contain no unwrap / expect / panic; also check equivalent paths through indexing, assert, todo, and similar operations. Trace panic paths through frame loops and callbacks. Errors must return explicitly or be reported before safe shutdown; exceptions use only the smallest scope allowed by the conventions.
- **Public API**: check internal unstable APIs, public stable / unstable tiers, features, and deprecation migrations under the [stability conventions](../../../docs/conventions/api-stability.md). Do not claim compatibility checks that are not integrated were run.
- **Anti-corruption layers**: check signatures, public fields, type aliases, generic bounds, and error types for third-party type leakage; deps cannot prove type-level boundaries.
- **Tests and snapshots**: cover behavior, boundaries, and failure branches. Visual changes need applicable snapshots; baseline updates need human confirmation. See the [snapshot conventions](../../../docs/conventions/testing-snapshots.md).
- **Observability**: existing critical frame, layout, painting, and event-dispatch paths should have tracing spans. M-1 has none of these paths; do not add framework code or dependencies just for this checklist.
- **Documentation and terminology**: check [GLOSSARY.md](../../../GLOSSARY.md), API documentation, and links; identify canonical wording when terms conflict.
- **ADRs**: decisions that are hard to reverse, surprising without context, and involve real trade-offs need an ADR and an updated index; see the [Git conventions](../../../docs/conventions/git-workflow.md).

Run `cargo xtask lint` and, before delivery, `cargo xtask verify`. Add verification under the [performance conventions](../../../docs/conventions/testing-performance.md) for performance changes.

List findings by severity, each with `file:line`, trigger or evidence, impact, and suggested change. Distinguish confirmed defects from open questions. If there are no findings, say so and report actual verification and coverage gaps.
