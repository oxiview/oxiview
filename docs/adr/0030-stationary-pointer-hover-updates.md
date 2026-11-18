---
status: accepted
---

# Stationary pointer hover: explicit style opt-in

Changing layout, stacking, clipping, or hit-test participation can change the
geometric hit under a pointer that has not moved. M2 does not automatically
turn these changes into hover transitions by default. A node style must
explicitly opt into stationary-pointer hover updates. The owner selected this
default and clarified the explicit opt-in during M2 planning on 2026-11-17.
The owner then confirmed that an enabled node on either relevant path triggers
a complete hover-path update, including notifications to disabled nodes.
The property name/type, default, lack of inheritance, and runtime switching
rules were also confirmed. Ordering within each resulting notification batch
is defined by [ADR-0031](0031-hover-transition-notification-batches.md).
Lifecycle cleanup and replacement-entry eligibility are completed by
[ADR-0033](0033-hover-lifecycle-cleanup.md). Implementation remains pending.
[ADR-0034](0034-pointer-input-processing-order.md) places final-state
stationary-hover checks after command-batch reports and required geometry
preparation, while mandatory capture/lifecycle notifications finish within
their commands. Native-preview implementation is now authorized; full
contract coverage remains to be verified.

This adds an input policy to the typed node styles in
[ADR-0008](0008-typed-style-with-css-vocabulary.md). It complements the capture
rules in [ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md) and handler
mutation boundary in [ADR-0029](0029-deferred-event-handler-mutations.md).

## Decision

- For an uncaptured, stationary pointer, changes to live nodes' geometry,
  stacking, clipping, or hit-test participation do not automatically reconcile
  hover or emit enter/leave by default.
- Node styles expose `stationary_hover_updates: bool`, defaulting to `false`,
  without property inheritance. Merely declaring a hover style does not enable
  it.
- With no capture, check the previous hover path and the new geometric hit
  path, including their structural ancestors. An enabled option on any node
  in either path triggers complete hover-path reconciliation. If neither path
  has an enabled node, geometry changes alone do not trigger an update. An
  enabled node outside both paths does not qualify.
- Once triggered, reconcile the complete path and send direct enter/leave
  notifications to all affected nodes, including those whose own option is
  disabled. Unchanged path membership produces no duplicate notifications.
  An enabled ancestor qualifies independently of a disabled descendant. A
  descendant's `false` cannot override an ancestor's `true`; the ancestor is
  checked directly on the path rather than through an inherited value.
- Enabling schedules one eligibility check after the current command batch is
  applied and necessary geometry preparation completes. Use the final styles
  and geometry: an enabled node on either relevant path triggers reconciliation.
  Unchanged paths emit no duplicate enter/leave; intermediate option values
  within a batch do not emit their own boundary notifications.
- Disabling preserves current hover and removes only that node's trigger for
  future stationary-pointer updates. Other enabled nodes on either relevant
  path retain their effect.
- The option is distinct from state-variant styling and reactive signals;
  their implementation remains in the milestones already assigned to them.
- Geometric hit testing and actual input routing remain governed by their
  existing rules. The retained hover path is not a substitute for a fresh
  geometric hit query.
- Capture still controls the effective hover target. Acquisition and release
  reconcile hover under ADR-0028 even without movement. Capture cancellation
  and safe cleanup are independent of this opt-in.

For example, with no enabled option on either relevant path, moving a live
hovered node P away from a stationary pointer to expose Q does not itself
produce P.leave or Q.enter. Retained hover can differ from the new geometric
hit until a later hover reconciliation. If P is disabled and Q is enabled,
Q becoming the new geometric hit triggers a complete switch, including
P.leave and Q.enter. Enabling only P or a relevant ancestor likewise qualifies;
shared ancestors with unchanged membership receive no extra enter/leave.
This default cannot justify delivery to a destroyed node.

## Considered options

- **Automatic updates after every relevant geometry change**: keeps hover in
  step with the geometric hit even without pointer input, but allows layout
  changes alone to trigger additional boundary handlers and their mutations.
- **Enable automatically when a hover style is declared**: couples interaction
  updates to the presence of visual styling. The owner explicitly chose a
  separate opt-in instead.
- **Default off, explicit style opt-in**: selected to let style authors request
  stationary-pointer updates deliberately. The trade-off is that retained hover
  can temporarily differ from geometry when no update is requested.
- **Complete-path update when either relevant path opts in**: selected to keep
  one consistent hover target and ancestor path. The option controls the trigger
  for reconciliation; affected nodes with the option disabled still receive
  the corresponding boundary notifications.

## Consequences and remaining decisions

- Style reads return each node's own `stationary_hover_updates` value. Ancestor
  participation comes from the path check, so an ancestor can enable updates
  while its descendants still read `false` for their own property.
- Runtime enabling uses one check of final styles/geometry after the batch;
  disabling does not clear hover. Repeated toggles must not generate boundary
  notifications for intermediate values. During capture, the holder continues
  to supply the effective hover target independently of this option.
- Trigger evaluation must account for the new geometric hit path as well as
  the old hover path: an enabled newly hit node can initiate a switch even
  when no node on the old path has opted in. A per-recipient notification
  filter would violate the confirmed complete-path rule.
- ADR-0031 defines state publication, notification order, and deferred mutation
  within a hover transition. ADR-0033 requires cleanup on removal or display
  suppression independently of this option, with cancel before hover cleanup
  and leave before destruction/suppression. Replacement entry remains opt-in:
  deleted old-path nodes contribute their last option value, surviving nodes
  contribute their final batch value, and cleanup leaves are not repeated.
  Window interruption/restoration alone does not create a new hover target;
  the next valid pointer input rebuilds hover.
- Tests cover no enabled option on either path, all P/Q enabled/disabled
  combinations, enabled ancestors with disabled descendants, the root,
  enabled unrelated nodes, and an empty old or new path. Verify complete
  notifications including disabled nodes, no duplicate notifications for
  unchanged membership, and fresh hit results despite retained hover. Capture
  acquisition/release/cancellation stays independent of the style option.
- Runtime fixtures cover enabling on and outside the relevant paths, disabling
  while other nodes remain enabled, repeated toggles with geometry edits in one
  batch, and toggling during capture. Verify the declared default, absence of
  property inheritance, final-state evaluation, and preservation of hover when
  disabling.
