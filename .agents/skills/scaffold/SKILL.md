---
name: scaffold
description: Use when creating an oxiview crate or planning widget or platform capability scaffolding; check registered contexts and invoke new-crate.
---

# Scaffolding

## Create a crate

1. Confirm creation is authorized for the current milestone. Read the [architecture](../../../docs/architecture.md) and the context, role, and allow fields in [layers.toml](../../../xtask/layers.toml).
2. The crate must already be registered. If it is not, discuss bounded contexts and dependency rules before registering it. Use `oxiview-*` or the public facade name `oxiview`.
3. Run `cargo xtask new-crate <name> --context '<context>'` from the repository root, using the exact context value from the rules. An unregistered name, mismatched context, or existing directory exits with 2. The command neither edits the rules nor overwrites directories.
4. Check that only `crates/<name>/Cargo.toml` and `src/lib.rs` were generated: inherited workspace package fields (including publish) and lints, with English crate-level documentation naming the context and role. Wildcard members include the crate automatically.
5. Add only dependencies needed for the current task, following the [dependency conventions](../../../docs/conventions/dependencies.md). Define versions in root workspace.dependencies and inherit them in crates; do not prefill future dependencies or functionality.
6. Run `cargo xtask verify` and review files and Cargo.lock. If unsafe is necessary, follow the [safety boundaries](../../../docs/conventions/unsafe-panic-errors.md); do not relax lints in advance.

## Widget checklist

Planning only: code templates arrive in M7; do not generate widget implementations now.

- Confirm the owning crate and dependency boundaries.
- Reserve node semantics fields and accessibility extension points.
- List applicable state variants such as hover, pressed, focused, and disabled.
- Plan pixel snapshot tests for normal and boundary states under the [snapshot conventions](../../../docs/conventions/testing-snapshots.md).

## Platform capability checklist

Planning only: code templates arrive in M9; do not generate capability implementations now.

- Define the capability trait's owned-type boundaries and error semantics.
- Plan `capabilities()` queries and behavior when unsupported; these are future interface requirements, not existing implementations.
- Plan a headless platform backend implementation and tests.
- List a macOS / Windows / Linux / headless platform backend support matrix, distinguishing supported, unsupported, and pending implementation.
