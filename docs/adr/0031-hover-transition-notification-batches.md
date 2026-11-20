---
status: accepted
---

# Hover transitions: ordered notifications with deferred mutation

One hover transition can notify several nodes. Applying mutations after each
individual enter/leave event would let an earlier handler change or destroy
recipients before the rest of that transition is delivered. Treat the related
notifications as one batch, publish its new hover state before delivery, and
apply callback mutations after the complete batch. The owner confirmed this
refinement during M2 planning on 2026-11-17. The preview batches hover
notifications. Tests exercise an enter callback that requests deletion and
inner-first leave order during removal. [Input contract tests](../../crates/oxiview-core/tests/m2_input_contract.rs)

This refines the mutation boundary in
[ADR-0029](0029-deferred-event-handler-mutations.md) for a hover notification
batch. Ordinary move/down/up/cancel dispatch retains its single-event boundary.
[ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md) and
[ADR-0030](0030-stationary-pointer-hover-updates.md) continue to govern capture
and stationary-pointer triggers respectively.

## Decision

- Fix the old and new hover paths for the transition and update the effective
  hover state to the new path before invoking any boundary handler. Every
  callback in the batch reads the new hover state.
- Nodes present only in the old path receive direct leave notifications, in
  order from the innermost node toward the root.
- After all leaves, nodes present only in the new path receive direct enter
  notifications, in order from the root toward the innermost node.
- Compare path membership by node identity. Common nodes receive neither
  enter nor leave, even if their position in the path changes. Identical paths
  produce no boundary notifications.
- All enter/leave callbacks in this transition share one mutation boundary.
  Queue their tree, style, and capture commands and apply them in submission
  order only after every notification in the batch finishes. Do not apply a
  callback's removal, reparenting, or other edit between notifications.
  These commands also follow ADR-0029's queue-tail rule and cannot overtake
  current or older queued command batches.
- Tree, style, and capture reads throughout the batch are unaffected by its
  queued commands. Hover reads already reflect the new path. This guarantee
  concerns framework-owned state, not arbitrary application-local variables.

For `Root -> A -> P` becoming `Root -> B -> Q`, notify
`P.leave -> A.leave -> B.enter -> Q.enter`. Root receives no boundary event.
If P.leave requests Q's removal, Q remains alive and receives this batch's
enter before removal is applied. If removal requires capture cancellation,
the existing cancel-before-destruction rule still applies.

## Considered options

- **Apply commands after each boundary event**: follows the ordinary
  single-event boundary uniformly, but can change the tree partway through a
  hover transition and requires rules for later recipients that disappear.
- **Apply commands after the complete hover notification batch**: selected to
  give callbacks a consistent new hover state and stable retained tree during
  the transition. The trade-off is that an earlier callback's removal request
  cannot prevent a later recipient from receiving its planned notification.

## Consequences and remaining decisions

- Each enter/leave remains a direct event dispatch. The enclosing hover
  notification batch determines when their mutation commands may take effect.
  No root-to-target propagation phase or bubbling is added to these events.
- Tests cover new hover state before the first notification, leave/enter order,
  common node identities, changed ancestry, empty/identical paths, unchanged
  tree/style/capture reads throughout the batch, and command application after
  its final notification. Include P.leave requesting Q's removal while Q still
  receives enter before destruction.
- This record defines delivery once a hover transition is required. It does
  not introduce new stationary-pointer triggers or disable capture cleanup.
- Placement of the notification batch itself relative to input, command
  application, and automatic capture release is defined by
  [ADR-0034](0034-pointer-input-processing-order.md). Commands from the initial
  hover batch wait through its associated primary input and automatic release;
  required capture transitions inside commands finish before command completion.
  [ADR-0033](0033-hover-lifecycle-cleanup.md) defines removal/suppression and
  interruption cleanup: cancel first, publish cleaned hover, then leave before
  destruction/suppression. It also defines replacement-entry eligibility and
  captured reparenting without changing this batch contract.
  ADR-0029 defines queue-tail insertion, individual rejection with
  partial batch success, and completed-batch reports to the application on the
  window thread. Concrete report types/signatures and callback-work bounds are
  delegated engineering choices under the owner's native-preview authorization;
  their implemented behavior and verification must be recorded separately.

## M2 execution evidence (2026-11-20)

Reconciliation publishes the new hover path, sends direct leaves inner to
outer and enters outer to inner for changed membership, then enqueues the
callbacks' commands together. Capture-release traces check that the leave
and enter callbacks observe the new hover state.
[Hover implementation](../../crates/oxiview-core/src/ui/input.rs) · [Capture-release tests](../../crates/oxiview-core/tests/m2_capture_matrix.rs)
