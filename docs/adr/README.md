# Architecture decision records

Language: **English**

| # | Decision | Dimension |
|---|------|------|
| [0001](0001-goals-and-non-goals.md) | Goals and non-goals: broad coverage, milestone progression, house style, hard performance targets (M1 latency gate partially superseded by 0026) | 1 Goals and non-goals |
| [0002](0002-own-platform-backends.md) | Platform layer: owned backends, Wayland first, capability traits | 2 Platform abstraction |
| [0003](0003-window-threading-model.md) | Threading: main thread pumps events, one window thread per window by default | 2 Platform abstraction |
| [0004](0004-frame-scheduling.md) | Frame scheduling: on-demand by default, reserved continuous rendering, display vsync (M1 FIFO admission rule superseded by [0036](0036-fifo-drawable-readiness.md)) | 3 Frame scheduling |
| [0005](0005-fine-grained-reactivity.md) | Reactivity: thread-local fine-grained signals, structural reconciliation, local executor | 4 State and reactivity |
| [0006](0006-builder-api-and-function-components.md) | Component API: Builder core, function components, props | 5 Components and API |
| [0007](0007-single-arena-node-tree.md) | Retained tree: one arena node tree, three dirty-flag levels | 6 Retained tree / invalidation |
| [0008](0008-typed-style-with-css-vocabulary.md) | Styling: typed properties, CSS vocabulary, restricted theme rules | 7 Styling and themes |
| [0009](0009-layout-protocol-taffy-default.md) | Layout: owned layout protocol, Taffy default | 8 Layout |
| [0010](0010-pure-rust-text-stack.md) | Text: pure Rust stack, hybrid rasterization, owned editing core | 9 Text stack |
| [0011](0011-ime-and-action-keymap.md) | Input: inline IME preedit, Actions, layered Keymaps | 10 IME and input |
| [0012](0012-scene-abstraction-wgpu-vello.md) | Rendering: owned Scene, wgpu + Vello, SDF fast path, linear color | 11 Rendering |
| [0013](0013-external-content-compositing.md) | Compositing: texture layers and embedded native child views | 12 Compositing and external content |
| [0014](0014-accessibility-deferred.md) | Accessibility: deferred implementation, reserved node semantics | 13 Accessibility |
| [0015](0015-interruptible-animation.md) | Animation: interruptible springs/easing, compositing fast path | 14 Animation |
| [0016](0016-devtools-in-framework.md) | Developer tools: hot reload, Inspector, performance, snapshots, replay | 15 Developer experience |
| [0017](0017-native-system-integration.md) | Native integration: title bars/materials, preferences, lifecycle, distribution | 16 Native integration |
| [0018](0018-macos-first-headless-backend-hedge.md) | Platform order: macOS first, headless safeguard, Windows/Linux later | 2 Platform abstraction |
| [0019](0019-local-only-verification.md) | Verification: local only, cargo xtask entry point, no CI | Engineering |
| [0020](0020-cargo-workspace-xtask-stable.md) | Build: Cargo workspace + xtask, pinned stable, edition 2024 | Engineering |
| [0021](0021-crate-layout-strategic-ddd.md) | Directories and crates: flat layout, oxiview- prefix, strategic DDD, ports/adapters | Engineering |
| [0022](0022-foundation-libraries.md) | Foundation library choices | Engineering |
| [0023](0023-agent-conventions-dual-harness.md) | Agent conventions: one canonical source for Claude Code and Codex | Engineering |
| [0024](0024-project-name-oxiview.md) | Project name: oxiview | Engineering |
| [0025](0025-owned-native-surface-handle.md) | Owned native surface handle and one unsafe constructor declaration in oxiview-base | 2 Platform abstraction |
| [0026](0026-m1-native-latency-gates.md) | M1 native latency gates: one-interval submission and actual presentation within a human-recorded baseline margin | Performance |
| [0027](0027-z-index-stacking-contexts.md) | Z-index: CSS stacking contexts for supported typed properties | 7 Styling and themes / 11 Rendering |
| [0028](0028-pointer-capture-ownership-and-lifecycle.md) | Pointer capture: explicit ownership, lifecycle, and hover | 10 IME and input |
| [0029](0029-deferred-event-handler-mutations.md) | Event handlers: deferred mutations and command validation | 6 Retained tree / invalidation / 10 IME and input |
| [0030](0030-stationary-pointer-hover-updates.md) | Stationary pointer hover: explicit style opt-in | 7 Styling and themes / 10 IME and input |
| [0031](0031-hover-transition-notification-batches.md) | Hover transitions: ordered notifications with deferred mutation | 6 Retained tree / invalidation / 10 IME and input |
| [0032](0032-owned-generational-ui-node-identity.md) | UI nodes: identity, lifecycle, and structural edits | 6 Retained tree / invalidation |
| [0033](0033-hover-lifecycle-cleanup.md) | Hover lifecycle: cleanup before removal and suppression | 6 Retained tree / invalidation / 10 IME and input |
| [0034](0034-pointer-input-processing-order.md) | Pointer input: stable dispatch and ordered command application | 6 Retained tree / invalidation / 10 IME and input |
| [0035](0035-node-owned-reactive-scopes.md) | Reactive ownership: node scopes follow retained subtree moves | 4 State and reactivity / 6 Retained tree / invalidation |
| [0036](0036-fifo-drawable-readiness.md) | Pace native frames by drawable availability | 3 Frame scheduling / 11 Rendering |
| [0037](0037-optional-precompiled-oxi.md) | Optional precompiled .oxi sources share the Rust Builder contract | 5 Components and API |
| [0038](0038-vello-gpu-and-glyph-atlas.md) | Use Vello GPU and a device-owned glyph atlas | 9 Text stack / 11 Rendering |
| [0039](0039-keymap-prefix-conflicts.md) | Reject ambiguous shortcut prefixes within a focus context | 10 IME and input |
| [0040](0040-local-style-over-theme-states.md) | Give explicit local properties precedence over theme state rules | 7 Styling and themes |
| [0041](0041-focus-recovery-in-tab-order.md) | Recover focus at the next eligible Tab stop | 6 Retained tree / 10 IME and input |
| [0042](0042-publish-committed-editor-text.md) | Publish committed editor text separately from inline preedit | 9 Text stack / 10 IME and input |
| [0043](0043-native-text-input-session-snapshots.md) | Answer native text-input queries from a session snapshot | 2 Platform abstraction / 10 IME and input |
