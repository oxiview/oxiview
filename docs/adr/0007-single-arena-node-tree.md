---
status: accepted
---

# Retained tree: a single arena node tree with style / layout / paint dirty flags

The framework has **one** retained node-tree layer: component functions produce nodes directly, and signals write node properties directly. There is no intermediate view-tree-to-render-tree layer, removing synchronization and memory overhead. This matches fine-grained signals (ADR-0005). Store nodes in a slotmap-style arena referenced by `NodeId`, not interlinked `Rc<RefCell>` values, to avoid borrow conflicts and improve cache locality.

Each node maintains separate **style, layout, and paint** dirty flags. Style changes need not trigger layout; layout need not propagate to ancestors; repainting affects only the containing layer. Invalidation stops at **layout boundaries** (subtrees sized independently of content). **Repaint boundaries** correspond to independent compositing layers, supporting ADR-0004's continuous layer rendering.

## Considered Options

- **Separate view and render trees** (Flutter, Xilem/Masonry): clear layering, but fine-grained signals need no diffable view layer, so the second tree only adds overhead.
- **ECS storage**: more compact data layout, but greater conceptual burden for general UI, while layout and events still require tree traversal.

## Consequences

- Node types must be openly extensible: custom widgets define measurement, painting, and event handling, potentially through trait objects attached to arena nodes.
- Derive the accessibility tree (decision 13) directly from this node tree instead of building a separate semantic tree.

## M2 identity and lifecycle refinement

[ADR-0032](0032-owned-generational-ui-node-identity.md) defines the opaque UI
`NodeId` contract: tree ownership, reusable slots distinguished by generation,
permanent stale-ID rejection, checked exhaustion, and a separate type from
Scene identity. It also defines stable root identity, clearing descendants while
preserving root styles/handlers, subtree removal, and whole-tree destruction.
It also defines delayed creation with mounting on successful execution, ordered
insertion, and same-tree movement using post-removal indices, preserving subtree
identity and rejecting invalid edits atomically. Data or creation functions may
precede live nodes without introducing another retained tree; every live node
remains reachable from the root, with no independent live-node detach operation.
Enqueueing actual creation reserves its `NodeId` without creating a live node;
reads report "not yet created" until success, and creation failure permanently
invalidates that ID. Deferred command validation and reporting remain unchanged.
Node-owned resources are released in child-list-ordered postorder with an
explicit stack, preserving parent resources through descendant cleanup.
These contracts refine the node tree without replacing this arena-tree decision.

## M2 execution evidence (2026-11-20)

The retained `UiTree` supplies layout inputs from its arena, and tests check
that local edits skip clean descendants and paint-only edits skip layout.
Clean nodes reuse painting content in complete Scenes, with independent
repaint boundaries deferred to M10.
[Layout tests](../../crates/oxiview-core/src/ui/layout_tests.rs) · [Paint tests](../../crates/oxiview-app/tests/ui_paint.rs)
