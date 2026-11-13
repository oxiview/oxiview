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
