---
status: accepted
---

# Hover lifecycle: cleanup before removal and suppression

Hover can intentionally differ from the geometric hit when stationary-pointer
updates are disabled. That policy cannot keep removed or display-suppressed
nodes in the hover path or defer their final notifications until after resource
release. The owner confirmed the following lifecycle contract as decision group
1 of the remaining M2 planning topics on 2026-11-17. Implementation is pending.

This completes lifecycle staging left open by
[ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md),
[ADR-0030](0030-stationary-pointer-hover-updates.md), and
[ADR-0031](0031-hover-transition-notification-batches.md). It uses
[ADR-0029](0029-deferred-event-handler-mutations.md) for callback commands and
[ADR-0032](0032-owned-generational-ui-node-identity.md) for node lifetimes and
resource release. Those decisions remain in effect.

## Lifecycle triggers

| Trigger | Hover and capture behavior |
| --- | --- |
| Remove a node or clear a subtree | Remove affected nodes from the hover path and notify them before destruction. Preserve surviving, unsuppressed ancestor hover membership. Cancel capture if its holder belongs to the removed subtree. |
| Display none on a node or ancestor | Apply the same hover cleanup to the suppressed subtree while retaining its nodes. Cancel an affected capture. |
| Pointer leaves the window | Clear hover without capture. With capture, keep the holder and its ancestor path. |
| Window deactivation, hiding, destruction, or system input interruption | Cancel an existing capture, then clear the entire hover path. |
| Geometry, stacking, clipping, opacity, or hit-test participation changes | Keep the existing capture and stationary-pointer policies; these changes alone do not force lifecycle cleanup. |

Cleanup is mandatory independently of `stationary_hover_updates`. Repeated
interruptions do not notify an already canceled capture or already departed
hover membership again. Never-created nodes receive no lifecycle input events.

## Cleanup order and callback observations

For node removal or display suppression:

1. Validate the requested operation before emitting cleanup notifications.
   A rejected command emits none of these lifecycle events.
2. If the operation affects the capture holder, send one cancel through its
   fixed target/bubble route while nodes remain readable. Cancel callbacks
   observe the old capture and old hover state.
3. Clear the capture being canceled and publish the cleaned hover path. When
   capture ends while the pointer is outside the window, the new path is empty.
4. Deliver leave directly to departing nodes from inner to outer under
   ADR-0031. Leave callbacks observe the new hover state and cleared capture
   when cancellation was required. Nodes and their pre-operation retained
   state remain available through these notifications.
5. After notification delivery, complete suppression or remove the subtree and
   release its resources in ADR-0032's ordered postorder.

Window interruptions use the same cancel-before-clear-before-leave ordering,
with an empty new hover path; window destruction releases node resources only
after required notifications. This is framework lifecycle ordering, not a
promise to postpone the native operating system's window-state change.

Callback mutations remain queued under ADR-0029. They cannot run ahead of the
current cleanup or prevent it, and stopping cancel propagation does not stop
cleanup. This does not define the separate queue-shutdown/reporting policy.

## Entering a replacement target

Removing or suppressing hovered P can expose Q under a stationary pointer:

- With no enabled `stationary_hover_updates` option on the old hover path or
  new geometric hit path, P still receives its required leave, but exposing Q
  does not by itself produce Q.enter.
- If either relevant path qualifies under ADR-0030, reconcile the complete
  path after the current command batch finishes and necessary geometry is
  prepared. Nodes already removed from hover during cleanup receive no
  duplicate leave. Shared membership receives no extra notifications.
- Preserve the old-path information needed for this trigger check. For a
  deleted node, use its last option value before deletion; for a surviving
  node, use its value at batch completion. Do not read reclaimed node storage
  or let an intermediate toggle of a surviving node override its final value.

Window interruption or restoration alone does not immediately establish a
new hover target. Rebuild hover on the next valid pointer input. Normal capture
release still reconciles geometric hover independently of this style option;
exceptional cancellation uses the cleanup rules above. A remaining active
capture continues to supply its holder path rather than a geometric target.

## Reparenting during capture

Reparenting a captured node or its ancestry preserves the holder and updates
hover to its new structural ancestor path. Apply ADR-0031's identity-based
membership comparison: the holder and other shared nodes receive no duplicate
leave/enter merely because their positions in the path changed. Moving the
holder into a display-suppressed subtree instead follows suppression cleanup,
including cancellation before completing the move.

## Considered options

- **Defer all cleanup until pointer movement**: follows the stationary default
  uniformly, but leaves stale/suppressed hover membership and can lose the last
  opportunity to notify a node before destruction.
- **Always enter the newly exposed target after cleanup**: keeps hover aligned
  with geometry, but makes removal or hiding bypass the owner's explicit opt-in
  for stationary-pointer updates.
- **Mandatory exit cleanup, conditional replacement entry**: selected to
  deliver final notifications safely while preserving deliberate entry
  behavior. Retained hover can temporarily contain only surviving ancestors.
- **Leave before cancel**: ends hover while drag ownership is still active.
  The selected cancel-first order lets the holder finish the interaction before
  leave callbacks observe the cleaned state.

## Verification and remaining decisions

- Trace valid and rejected removal/suppression, removal of a hovered ancestor,
  clear-contents, all window interruption kinds, and window leave with/without
  capture. Verify no stale dispatch, no duplicated notifications, and surviving
  ancestor membership where applicable.
- Verify cancel reads old capture/hover, leave reads the cleaned state, and
  affected nodes remain readable until notifications finish. Follow with the
  exact ordered resource-release trace for removal; suppression retains nodes.
- Verify callback mutations join the queue tail and cannot rescue a node ahead
  of its current removal. Stopped cancel bubbling must still clear capture and
  hover as required.
- Cover P-to-Q replacement with all relevant opt-in combinations, deleted
  old-path nodes using their last value, live nodes using final batch values,
  and no repeated leave after cleanup. Include enabling then disabling a
  surviving ancestor before batch completion.
- Verify cancellation outside the window clears all hover, interruption and
  restoration alone create no new enter, and the next valid pointer input
  rebuilds hover. Keep normal capture release independent of the opt-in.
- Cover reparenting a captured holder and an ancestor, shared-node identity
  suppression, and moving into hidden ancestry.
- [ADR-0034](0034-pointer-input-processing-order.md) completes global
  input/command/geometry ordering. The owner subsequently approved the remaining
  groups and authorized native-preview implementation, delegating queue shutdown,
  unfinished-batch reporting, and work-budget details to engineering judgment.
  Full contract verification and snapshot/performance baseline acceptance remain
  separate from that authorization.
