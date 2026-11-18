---
status: accepted
---

# Z-index: CSS stacking contexts for supported typed properties

M2 includes z-index so applications can change overlap without changing node
structure or layout order. Use CSS stacking-context semantics for the typed
properties supported by oxiview. Ordinary parents do not isolate their
descendants' stack levels; a stacking context is ordered atomically within its
containing context. The owner confirmed this decision during M2 planning on
2026-11-17. Implementation and milestone acceptance remain pending.

This extends the CSS vocabulary decision in
[ADR-0008](0008-typed-style-with-css-vocabulary.md), preserving the single UI
tree in [ADR-0007](0007-single-arena-node-tree.md) and the Scene boundary in
[ADR-0012](0012-scene-abstraction-wgpu-vello.md). It introduces no CSS parser,
selectors, cascading, or browser dependency.

## Supported semantics

Z-index defaults to auto and accepts signed integer levels, including zero.
Higher levels paint above lower levels within the relevant stacking context
and painting phase; equal levels use tree order, with later content on top.
Nested contexts are atomic when ordered with outside content.
See the [stacking model](https://www.w3.org/TR/CSS22/visuren.html#z-index).

Z-index applies to relative and absolute nodes. Non-auto values establish
contexts, including explicit zero; auto does not itself do so. The root always
establishes a context. Ordinary static nodes outside Flex/Grid item roles have
no effective z-index.
See [positioning and painting order](https://www.w3.org/TR/css-position-3/#painting-order).

Flex and Grid items also support z-index when statically positioned, and their
non-auto values establish contexts. Their painting follows the corresponding
item rules. M2 has no order property, so structural tree order supplies item
order. See [Flex painting](https://www.w3.org/TR/css-flexbox-1/#painting) and
[Grid z-order](https://www.w3.org/TR/css-grid-2/#z-order).

Opacity below one establishes a context for any node. Its subtree is composited
as a group. Without another applicable stack level, that context occupies level
zero under the opacity painting rules. Background color alpha is a paint value
and does not trigger a context; neither do corner radii nor overflow clipping.
See [group opacity](https://www.w3.org/TR/css-color-4/#transparency).

Follow the [CSS painting phases](https://www.w3.org/TR/CSS22/zindex.html), with
the Flex/Grid rules above, for M2-supported content: a context's own background
and border, negative contexts, normal-flow content in its applicable phases,
positioned auto/zero-level content, and positive contexts. Phases for features
outside M2, such as floats and inline formatting, add no new M2 capability.
Do not replace the phase rules with one global numeric sort or a sort at every
structural parent.

Stacking contexts do not change absolute-position containing blocks. The
confirmed M2 rule still uses the nearest relative/absolute ancestor or the
logical viewport. This decision adds no positioning or context-triggering
properties beyond the confirmed M2 scope.

## Considered options

- **Tree order only**: simple traversal, but changing overlap would also require
  structural edits that can change layout.
- **Independent sorting at every parent**: simpler local ordering, but every
  ordinary wrapper would become an isolation boundary. Adding a wrapper could
  unexpectedly prevent a descendant from competing with adjacent content.
- **CSS stacking contexts**: selected for familiar property semantics and
  explicit isolation boundaries. It requires painting phases and derived
  ordering across ordinary parent boundaries, rather than only sibling sorting.

## Consequences

- Auto and explicit zero are observably different. Tests must cover an ordinary
  wrapper and a wrapper that establishes a context, including descendants
  competing with outside content.
- Effective z-index changes and opacity transitions that create/remove contexts
  update derived painting order without geometric layout computation. Derived
  ordering does not become a second authoritative UI tree.
- The same ordering must be available to hit testing. Hit-test eligibility and
  pointer routing are separate M2 decisions and are not settled by this ADR.
- A stacking context is an ordering boundary, not necessarily an independent
  compositing layer or repaint boundary. M10's independently updated layers
  remain deferred; group opacity does not bring that milestone forward.
- Verification must cover signed levels, phase and tree-order ties, auto versus
  zero, static Flex/Grid items, opacity triggers, non-triggers, and unchanged
  absolute-position containing blocks. Changing these observable rules later
  requires revisiting this decision.
