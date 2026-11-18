---
status: accepted
---

# Pointer input: stable dispatch and ordered command application

A positional input can change hover, deliver a primary event, release capture,
and enqueue tree edits. Applying a hover callback's edits before the primary
event could retarget the same input or destroy its recipient. Applying an up
callback's removal before normal capture release could turn a finished drag
into a cancellation. The owner confirmed this processing order as M2 decision
group 2 on 2026-11-17.

This completes scheduling left open by
[ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md),
[ADR-0029](0029-deferred-event-handler-mutations.md), and
[ADR-0031](0031-hover-transition-notification-batches.md). Stationary-hover
eligibility follows [ADR-0030](0030-stationary-pointer-hover-updates.md), and
lifecycle cleanup follows [ADR-0033](0033-hover-lifecycle-cleanup.md). Their
ownership, queue-order, notification, and failure contracts remain in effect.

## Decision

Process a positional move/down/up input in this order:

| Phase | Behavior |
| --- | --- |
| Apply older work | Process previously queued mutation batches before selecting the next positional input's target. |
| Prepare the input | Update the current position/button snapshot. Refresh dirty layout and hit-test data as needed, then fix the effective target, ancestor route, and event coordinates. |
| Reconcile input hover | Publish the new hover state and deliver the required leave/enter batch. Keep commands produced by these callbacks queued through the primary event. |
| Dispatch the primary event | Deliver move/down/up to the fixed target and bubble along its fixed ancestor route. Stop propagation affects this delivery, without discarding queued commands. |
| Complete normal release | If this up releases the final pressed button, automatically release capture and reconcile its hover before applying callback commands. |
| Apply callback commands | Process queued command batches FIFO. At each completed batch, deliver its report before the final-state stationary-hover eligibility check and any required geometry preparation/reconciliation. |

The initial hover batch and primary dispatch observe the same retained tree
state. Their own queued edits cannot change the selected target, route, or
geometry during that input. This scheduling rule extends the minimum mutation
boundaries in ADR-0029/0031; completing the initial hover batch does not flush
its commands ahead of the associated primary event.

For example, Q.enter may enqueue removal of Q. Q still receives this input's
down before the removal command runs:

`Q.enter -> Q.down -> remove Q`

If P holds capture and its final up callback enqueues self-removal, the order is:

`P.up -> normal capture release -> hover reconciliation -> remove P`

Removing P does not cancel a capture that already ended normally.

## Commands and notifications

- Capture acquisition, explicit release, captured-path reparenting, and the
  removal/suppression cleanup from ADR-0033 deliver their required notifications
  within the corresponding command, before that command finishes.
- Commands produced by these notifications or by result callbacks append to
  the existing queue tail. They cannot recursively run inside the current
  callback or command and cannot overtake already queued batches.
- Ordinary stationary-pointer reconciliation uses a batch's final retained
  state, after its completion report and necessary geometry preparation. This
  does not defer mandatory capture/lifecycle notifications until batch end.
- A completion report confirms command application, not successful layout or
  frame presentation. Result-handler commands are later work and do not change
  the just-completed batch's reported outcomes or its retained final state
  before their own execution.

## Geometry and presentation

Refresh geometry when the input or required notification needs it; clean
layout and hit-test data can be reused. Geometry preparation does not present a
frame. Mutations request and coalesce rendering through the existing scheduler;
callbacks and individual commands do not directly present intermediate frames.

Work budgets and yielding/recovery are engineering details in group 4. Layout
or painting preparation failure handling belongs to group 11. Neither is
implicitly settled by this ordering, and neither permits stale-node dispatch.

## Considered options

- **Flush hover callback edits before the primary event**: makes an enter
  callback's edits immediately visible, but can change the recipient of the
  input that caused that enter. The selected stable-target policy keeps the
  recipient alive until its associated primary event completes.
- **Apply up callback edits before automatic release**: follows simple FIFO
  processing immediately after dispatch, but deleting the holder can generate
  cancel after a normal terminal up. The selected release-first order completes
  the interaction before those edits are applied.
- **Coalesce all capture notifications to batch end**: reduces intermediate
  notifications, but lets later commands observe capture and hover that have
  not completed their required transition. Mandatory transitions instead finish
  within the command; ordinary stationary geometry updates use final batch state.

## Verification and execution status

- Verify older queued edits affect the next input's hit and geometry, while
  changes queued during its initial hover batch do not retarget that input.
- Trace Q.enter requesting Q's removal through Q.down and then removal. Check
  fixed target/route/coordinates and pre-command retained-state reads.
- Trace final up, normal release, hover reconciliation, then queued self-removal,
  with no extra cancel. Non-final button releases retain capture.
- Trace capture acquisition/release, captured reparenting, and lifecycle
  notifications inside their commands; generated commands run only later.
- Verify completed-batch report delivery precedes the final-state stationary
  hover check. Both report and hover callback mutations retain FIFO tail order.
- Verify dirty geometry is prepared before routing without forcing presentation,
  and clean inputs reuse geometry while unchanged applications remain idle.

After confirming this group, the owner approved the remaining groups 3-12 and
authorized immediate native-preview implementation using engineering judgment
and parallel agents. The preview's public tests cover stable routing across an
enter callback's deletion request and final-button release before queued removal.
The M2 execution record tracks remaining native, snapshot and performance
acceptance. [Input contract tests](../../crates/oxiview-core/tests/m2_input_contract.rs) · [M2 execution record](../plans/m2-node-tree-and-layout.md#execution-record)
