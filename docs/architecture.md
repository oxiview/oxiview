# Context mapping and dependency boundaries

oxiview targets only native macOS, Windows, and Linux desktop platforms. This document explains the bounded contexts, published language, and ports/adapters organization in [ADR-0021](adr/0021-crate-layout-strategic-ddd.md); it does not define framework features.

[`xtask/layers.toml`](../xtask/layers.toml) is the sole source of truth for dependency direction. Maintain each crate's context, role, and direct dependency allowlist there when adding or changing crates; neither this document nor the checker duplicates those rules. Planned crates that do not exist are skipped; unregistered workspace members are errors. M-1 S3 does not create empty framework crates.

## Contexts and ports

Reactivity, rendering, text, layout, platform, styling, UI core, and widgets each form a bounded context. The shared foundation provides cross-context data types; Scene is the published language between the node tree and rendering backends. Contexts such as platform and rendering express required capabilities through their own ports. Adapters implement ports; ports do not depend on concrete implementations.

The following conceptual dependency graph points from consumers to providers. Shared-foundation edges and test dependencies are omitted; the rules file defines the exact allowlists.

```mermaid
flowchart TD
    facade[Public facade] --> app[Composition root]
    facade --> widgets[Widgets]
    facade --> macros[Procedural macros]
    widgets --> core[UI core]
    app --> core
    core --> ports[Reactivity / Scene / Text / Layout / Platform / Styling]
    app --> ports
    app --> render[Rendering adapters]
    app --> platform[Platform adapters]
    render --> scene[Scene port]
    platform --> platform_port[Platform port]
    ports --> base[Shared foundation]
```

Adapters use only their own ports and permitted lower layers, never one another. The composition root selects implementations and wires ports together. UI core depends on ports. The public facade and widget library are planned. The examples currently use the internal crates directly.

## Implemented UI and presentation path

`oxiview-core` owns each window's retained UI tree. Optional `oxiview-macros`
syntax expands to the same Rust Builders as handwritten views. Signals and
node-owned scopes in `oxiview-reactive` drive bound properties and structural
updates. The Taffy adapter reads typed layout inputs from the tree.
[M2 layout](plans/m2-node-tree-and-layout.md) · [M3 views](plans/m3-reactivity-and-components.md)

M3+1 adds precompiled view fragments and named view, style and Rust declarations
in optional `.oxi` files. The procedural macro crate uses `oxiview-syntax` for
file and inline views. The standalone `oxiview-lsp` uses the same grammar and
keeps editor indexes outside the application runtime. [M3+1 sources](plans/m3-plus-1-oxi.md)

The [view abstract syntax tree (AST)](../crates/oxiview-syntax/src/view/ast.rs)
represents element tags, attributes, children and embedded Rust expressions.
The parser builds it for both inline and external views. The compiler lowers
it to Builders, while the [source binding pass](../crates/oxiview-syntax/src/bindings.rs)
retains the original token ranges for navigation. For example, the `style`
attribute selects the inherent method on the value returned by the current
node factory. Component declarations and ordinary Rust items use syn's Rust
syntax nodes during module compilation.

For text, core uses `oxiview-text` to shape paragraphs and measure their
first-line baseline. Small hinted glyphs and bitmap strikes carry owned
image data. Scalable glyphs carry their font, variation coordinates and
local baseline position through the Scene. Native and offscreen GPU
rendering share a Vello adapter and device-owned glyph atlas.
[M4 text and rendering](plans/m4-text-and-snake.md) · [Renderer selection](adr/0038-vello-gpu-and-glyph-atlas.md)

Each window tree owns its focus, state variants, Action registrations and layered Keymaps. Platform adapters supply physical and logical key identities and an initial preference snapshot followed by ordered changes. The styling port validates typed theme files. Core resolves them against the window's preferences, active states and local declarations. A development file watcher in the composition root publishes a valid revision to the window thread. Snake persists the player's settings separately from framework state. [M5 contracts](plans/m5-focus-keyboard-and-themes.md) · [Runnable demo](../examples/snake-m5-oxi/README.md)

On macOS, the window thread owns input dispatch, painting and submission.
For an ordinary native frame, a worker waits for a surface texture and wakes
the window thread when it is available. The window can process input during
that wait. Live resize configures, paints and acknowledges synchronously.
[Native frame pacing](adr/0036-fifo-drawable-readiness.md)

## Anti-corruption layers

Convert third-party implementation types through the owning context's anti-corruption layer so implementation choices do not propagate into other contexts' public APIs. The rules file's external tables restrict direct consumers of third-party libraries, supporting exact package names and trailing `*` prefix matching. New restrictions do not require checker changes. Type leakage and whether styling uses only Scene color/geometry types still require code review; dependency graph checks cannot prove type-level boundaries.

[ADR-0013](adr/0013-external-content-compositing.md) explicitly requires a shared GPU device and allows wgpu in the application-facing API. This is a specific exception to ADR-0021's general anti-corruption principle. This step only registers the composition root's dependency permissions as requested; it adds no API and grants no additional permissions to other contexts.

## Check semantics and verification

`cargo xtask deps` implements the common Check interface and is included in `cargo xtask verify`. The checker reads workspace members and direct dependency declarations from `cargo metadata --format-version 1 --no-deps`, independently of default-feature resolution. It checks normal, build, dev, optional, and target-specific dependencies, matching renamed dependencies by their real package names. Transitive third-party dependencies are not subject to direct-consumer restrictions, which would otherwise mistake upstream internals for project cross-context usage.

Ordinary dependency allowlists apply to all three dependency kinds. Test tools marked dev-only are a general exception only in dev-dependencies; this does not allow other direction violations to bypass checks through dev-dependencies. Engineering tools may be exempt from outgoing-edge checks, but other crates may not depend on them. Each violation reports the consumer, dependency package, dependency kind, target condition, and violated rule.

The same check first runs `cargo deny fetch db` to refresh the advisory database, then `cargo deny --offline check` to check licenses, advisories, duplicate versions, and sources against the freshly fetched database. An explicit fetch failure is an environment error (exit code 2), with guidance to check networking, proxies, and cache permissions. A stale database is never presented as a pass. Rule violations or dependency review failures exit with 1; success exits with 0. Retry after connectivity is restored.

License, advisory, and source policies live in the root [`deny.toml`](../deny.toml). Permissive licenses use an allowlist; unlisted licenses, including copyleft, are rejected. Multi-licensed dependencies may select an allowed branch. Duplicate versions produce warnings only; crates.io is the only permitted remote source.

Reports go to `target/xtask/report.json`; full logs go to `target/xtask/deps.log`. `--json` keeps stdout machine-readable. Tests create isolated workspaces under `target/xtask/deps-fixture-*` and clean them up afterward; they add no members to the main workspace and require no network access.
