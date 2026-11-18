---
status: accepted
---

# Event handlers: deferred mutations and command validation

M2 event handlers can request node removal, reparenting, style changes, and
pointer capture. Applying these changes while the same event is still being
delivered would let earlier handlers alter the state and route seen by later
handlers. Defer framework-owned mutations until the complete event dispatch
ends, preserving one target, route, and readable tree/style/capture state
throughout delivery. The owner confirmed this boundary during M2 planning on
2026-11-17 and subsequently confirmed application-time validation with
individual rejection and partial batch success, queue-tail insertion for
commands produced during application, and completed-batch reports delivered to
the application on the window thread. The preview implements deferred edits and
reports their outcomes. Tests exercise a failed reserved parent, dependent
child failure and successful independent edits in the same sequence.
[Core tests](../../crates/oxiview-core/src/ui/tests.rs)

This complements the single arena in
[ADR-0007](0007-single-arena-node-tree.md) and the capture lifecycle in
[ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md). It does not change
the window-thread ownership model in
[ADR-0003](0003-window-threading-model.md).

The later-confirmed [ADR-0031](0031-hover-transition-notification-batches.md)
refines the mutation boundary for the enter/leave events of one hover
transition: their commands wait until the whole notification batch finishes.
The ordinary single-event boundary below remains in effect for move/down/up/
cancel delivery.

## Decision

- Fix an event's target and structural propagation route before invoking its
  handlers. The current receiving node advances along that fixed route.
- During dispatch, handlers submit tree, style, and pointer-capture changes as
  mutation commands. They do not apply those changes directly to the live tree.
- For ordinary event dispatch, apply commands in submission order after the
  whole dispatch completes or further propagation is stopped. Do not flush
  between individual handlers of the same event. Hover notification batches
  use the extended boundary in ADR-0031.
- Reads of tree, style, and capture state during that dispatch observe the
  state before command application. A submitting handler and subsequent
  handlers cannot read back a queued change as though it had already happened.
- Stopping propagation acts immediately on dispatch control. It skips remaining
  ancestor handlers but does not discard commands already submitted.
- Requested removal happens after the original dispatch. If the affected node
  still holds capture when removal is applied, deliver cancel before destroying
  it, as required by ADR-0028. Preserve unconditional capture cleanup.

For example, P can request its own removal while handling an event. P stays
alive and the event continues to its structural ancestors unless propagation
is stopped. Only after this delivery ends may removal proceed. Similarly,
setting a style and reading it in the same dispatch returns its previous value.

The decision covers framework-owned retained state and capture requests, not
arbitrary application-local variables or all future reactive state. Keeping the
live state unchanged during dispatch does not require copying the whole tree.

### Queued creation identity

[ADR-0032](0032-owned-generational-ui-node-identity.md) reserves a `NodeId` when
an actual creation command is enqueued, so later queued commands can reference
it immediately. Reservation does not create a live node or change the readable
tree/style/capture state. Reading that pending node returns "not yet created".
Creation still validates at its FIFO turn, mounts on success, and permanently
invalidates the reserved ID on failure. Dependent commands reject individually
and unrelated commands continue; the completed-batch report can include failed
creation IDs without reading node storage. This refines identity availability,
not the mutation, validation, queue-order, or reporting boundaries below.
The creation command owns its reservation: discarded ID copies do not cancel
it, while creation failure or queue destruction before execution releases the
reservation and command-owned resources and permanently invalidates the ID.
A never-created node receives no enter/leave/cancel. This cleanup rule does
not define an unfinished-batch reporting policy or change the completed-batch
report contract.

## Commands queued during application

- Commands produced by callbacks while applying a command batch join the tail
  of the existing mutation queue. Finish the current batch and retain older
  queued batches ahead of the new commands.
- Preserve submission order within a batch and first-in, first-out ordering
  across queued batches. Do not recursively apply newly produced commands
  inside the current command or callback.
- Completing a dispatch or hover notification batch makes its mutation commands
  eligible for application; it does not move them ahead of existing work.
- Lifecycle notifications still run at their required points. Removal sends
  cancel while a capture holder is live, before destroying it. Commands from
  that cancel callback wait behind the existing queue.
- Validation uses the state when each deferred command reaches its turn.
  Application-local cleanup can still run in the callback; only the retained
  tree/style/capture mutations are deferred under this policy.

For C1 (delete P), C2 (modify R), suppose C1's cancel callback enqueues C3
(modify S). Deliver cancel before destroying P, finish C1, then apply C2 and
C3. If C3 instead targets P, reject it when reached because P is no longer live.
The dispatch boundary does not allow C3 to run recursively before C1 finishes.

## Command validation

- Validate a queued command when it reaches its turn for application, against
  the current state, including earlier successfully applied commands. Enqueueing
  a command alone does not establish that it will still be valid then.
- A validation failure rejects that command without applying any part of its
  state change. Record an error identifying the command and rejection reason.
- Continue validating and applying later commands in submission order. Later
  commands see earlier successful changes; dependent commands can fail
  independently when their prerequisites no longer hold.
- Preserve all earlier successful changes. The batch permits partial success
  and does not roll back on validation failure.

For example, a batch deletes Q, sets Q's width, and sets R's color. Deletion
succeeds; the width update is rejected because Q is no longer live; R's valid
color update still succeeds. Q remains deleted.

This policy covers command validation failures. The existing
[error conventions](../conventions/unsafe-panic-errors.md) continue to govern
fatal errors and unexpected panics. Completed batches provide reports under
the contract below; concrete Rust types and callback signatures remain separate.
Rejected commands must be identifiable and must not be silently ignored.

## Command batch reports

- Each completed mutation command batch produces one report with every command's
  success or failure in submission order. Partial success is represented directly.
- Include the batch identifier, each command's sequence number, relevant
  `NodeId` values where applicable, and a structured failure reason for each
  rejected command. Node identities remain reportable after removal or invalid
  access; producing the report must not dereference stale node storage.
- Deliver the report after the batch finishes through an application callback
  on the window thread. This result callback does not run between commands of
  the same batch.
- Tree/style/capture mutations requested by the result callback become new
  commands at the existing queue's tail. Completed results remain fixed;
  response commands are validated and reported as later work.
- Without a custom result handler, the default handler logs failures with the
  identifying context and reasons. Successful commands remain silent by default.

The delete-Q, set-Q-width, set-R-color example reports success, failure with
the invalid-node reason, and success in submission order. A successful result
records command application; frame preparation and presentation follow their
own scheduling rules. Concrete Rust types and callback signatures are delegated
implementation choices under the subsequent native-preview authorization.

## Considered options

- **Immediate mutation during handlers**: supports immediate readback, but later
  handlers can observe different structure or styles. Removal and reparenting
  also require rules for an event whose original route has changed, and careful
  management of arena access during callbacks.
- **Deferred mutation after complete dispatch**: selected to keep the route and
  readable retained state consistent across handlers, and to separate callback
  execution from structural edits. The cost is delayed readback and a command
  result interface that must account for validation at application time.
- **Stop the batch at its first invalid command**: avoids attempting later
  dependent commands, but also skips independent valid updates after the error.
- **Roll back the entire batch**: gives all-or-nothing state changes, but requires
  transactional handling across structural edits, capture, and lifecycle effects.
- **Reject individually and continue**: selected so independent valid updates
  can still apply. Earlier successes remain, so callers must account for partial
  success and potentially several identifiable failures.
- **Recursively apply commands from lifecycle callbacks**: makes callback
  mutations take effect sooner, but lets them overtake older queued work and
  change state before the current command finishes.
- **Append new commands to the queue tail**: selected to preserve existing
  command order and avoid recursive command application. A callback cannot use
  queued edits to change a node before an already-running removal completes;
  those edits may instead be rejected when their turn arrives.
- **Per-command result callbacks during application**: exposes outcomes sooner,
  but interrupts application of a batch with result-handling callbacks.
- **One result report after batch completion**: selected to expose all outcomes
  of a partially successful batch together, with stable ordering and identifying
  context. Callers receive the report after every command in that batch has
  been attempted.

## Consequences

- The event API must distinguish reading retained state, submitting mutations,
  and controlling propagation. Callback implementations must not assume that
  a submitted style or capture request has already taken effect.
- Deterministic traces must cover removal/reparenting without a route change,
  consistent reads by the submitting handler and later ancestors, command
  application order, stopped propagation, delayed capture acquisition/release,
  and cancellation before destruction of a captured node.
- Tests cover application-time validation, failures at different batch positions,
  later valid commands continuing, preservation of earlier successes, dependent
  commands becoming invalid, unchanged state for rejected commands, and errors
  identifying the affected command and reason. The same validation policy
  applies to commands from ordinary dispatch and hover notification batches.
- Reports make each command's outcome explicit; enqueueing or another command's
  success does not establish that this command succeeded. Concrete report types
  and callback signatures are delegated implementation choices.
- Tests cover all-success/all-failure/mixed batches, ordered identifiable
  outcomes, structured reasons, delivery after completion on the window thread,
  and result-handler commands waiting behind older queued work. The default
  handler must log failures and stay silent for successes. Reporting stale or
  destroyed NodeIds must not access their storage; a report alone must not
  request a frame.
- Queue-order traces cover cancel before P's destruction, completion of C1 and
  C2 before a callback-generated C3, rejection of C3 if it targets destroyed P,
  and preservation of older waiting batches ahead of newly generated work.
  Include further generations of callback commands without recursive drain.
- ADR-0031 fixes ordering and deferred mutation within each hover notification
  batch. [ADR-0034](0034-pointer-input-processing-order.md) completes input and
  command scheduling: initial hover callbacks cannot mutate the primary event's
  target, final-up release precedes queued commands, and completion reports
  precede final-state stationary-hover checks. The owner subsequently authorized
  native-preview implementation and delegated remaining engineering choices,
  including callback-work budgets. Full milestone acceptance remains pending.
