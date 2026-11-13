# Dependency management

## Adding and upgrading dependencies

Record the following assessments before adding a dependency. Recheck when an upgrade changes sources, licenses, or the dependency tree:

- A selectable license branch is allowed by the root [deny.toml](../../deny.toml) and compatible with MIT / Apache-2.0 distribution. Do not duplicate the allowlist here.
- Active maintenance: inspect recent releases, commits, issue responses, and security advisories rather than download counts alone.
- Whether it is pure Rust; for non-Rust parts, assess build tools, system libraries, target support, and distribution cost. Prefer pure Rust without treating system bindings as violations.
- Whether the added transitive dependencies, duplicate versions, compilation time, and source footprint are worthwhile.
- Whether the workspace or standard library already provides equivalent functionality that can be reused.
- Whether it can be encapsulated within its owning context without exposing third-party types in public APIs.

Define versions only in `[workspace.dependencies]` in the root [Cargo.toml](../../Cargo.toml). Crates inherit them with `xxx.workspace = true` in dependencies / dev-dependencies / build-dependencies as appropriate; do not repeat versions. Foundation libraries follow [ADR-0022](../adr/0022-foundation-libraries.md).

Register restricted external dependencies with `packages` and `allow` in [layers.toml](../../xtask/layers.toml) `[[external]]` entries. Package names support exact matching and trailing `*`. Checks cover direct consumers declared by the workspace across normal / build / dev, optional, and target-specific dependencies, resolving renames by real package name. They do not restrict transitive consumers inside third-party libraries. Type-level anti-corruption layers still require code review; see [architecture](../architecture.md).

Run `cargo xtask deps` to review direction, licenses, advisories, duplicate versions, and sources. Duplicate versions currently produce warnings rather than a blanket ban. Pass `cargo xtask verify` again before committing. Advisory refresh failure is an environment error; do not present old results as success.

## Upstream patch queue

Keep patches in the root Cargo.toml `[patch.crates-io]` and prioritize upstream fixes. Use traceable pinned commits or version-controlled local patches, not machine-specific directories. Record each new patch below with its crate, reason, upstream issue/PR link, and explicit removal condition. On upgrades, check whether upstream has merged and released the fix. Once the condition is met, remove the patch and entry, then verify again.

The current deny policy rejects unapproved Git sources. Review and explicitly update source allow rules before using a Git patch; never bypass cargo-deny. This step adds no patches and changes no source policy.

| Crate | Reason | Upstream issue / PR | Removal condition |
| --- | --- | --- | --- |
| None | No upstream patches currently | Not applicable | Not applicable |

## When to write an ADR

For costly choices that lock in public interfaces, data formats, runtimes, platform bindings, or extensive implementation, apply the [three Git convention criteria](git-workflow.md). An ADR is required when the decision is hard to reverse, surprising without context, and involves real trade-offs. A routine local tooling dependency does not need an ADR merely because it is new; a complete review record is still required.
