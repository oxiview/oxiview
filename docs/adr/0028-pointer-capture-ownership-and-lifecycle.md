---
status: accepted
---

# Pointer capture: explicit ownership, lifecycle, and hover

M2 pointer capture keeps a drag directed to one node when the pointer leaves
that node or the originating window. Use explicit acquisition and handoff:
another node cannot take capture from the current holder. Distinguish normal
release from exceptional cancellation so components can end interactions
predictably without depending on which ancestor handler runs last. The owner
confirmed this scope and lifecycle during M2 planning on 2026-11-17, and then
selected capture-based hover during the same planning discussion.
The preview implements capture. A native drag across the right window edge
continued to deliver movement and release. The full native acceptance matrix
remains open. [M2 execution record](../plans/m2-node-tree-and-layout.md#execution-record)

This complements the node ownership in
[ADR-0007](0007-single-arena-node-tree.md) and owned platform adapters in
[ADR-0002](0002-own-platform-backends.md). Pointer capture redirects input;
it is distinct from the root-to-target capture propagation phase deferred by M2.

## Decision

- Acquisition is explicit and requires at least one pressed mouse button and
  a live target node in the same window. Display none on the target or an
  ancestor makes that target ineligible for acquisition.
- Each pointer has one holder. A repeated valid request from the holder is a
  no-op. Another node must wait for release; a competing request cannot replace
  existing ownership. Handoff therefore requires explicit release first.
- Subsequent move/down/up events target the holder and bubble through its
  structural ancestors. During a normal drag, leaving the node/window or
  returning inside does not change capture. Outside-window movement and button
  release remain part of the required delivery scope.
- The final up that leaves no mouse buttons pressed is delivered before
  automatic release. The holder can release early; subsequent events use
  ordinary hit testing, without a synthetic up or cancel for that release.
- Window deactivation, hiding, destruction, and system input interruption
  cancel capture. Holder removal, including removal of an ancestor subtree,
  or display none suppressing the holder through its own or an ancestor's
  state also cancels it.
- Exceptional cancellation sends one cancel notification and clears capture.
  Deliver the notification before node destruction. Stopping event propagation
  cannot prevent framework cleanup; repeated interruption notifications must
  not deliver duplicate cancellation for the same capture.
- Opacity changes and changes to the holder's own hit-test participation do
  not automatically cancel existing capture. That participation switch
  controls target selection by hit testing, not an existing capture assignment.
- Capture also supplies the effective hover target. The holder and its
  structural ancestors form the hover path while capture is active, even when
  the pointer is over another node or outside the window. Geometry changes
  alone do not produce leave from the holder or enter to a node underneath.
- Acquisition reconciles hover to the holder's path. Normal release restores
  ordinary geometric hit testing and reconciles enter/leave, even without
  pointer movement. Releasing outside the originating window clears ordinary
  hover there. Final-up delivery precedes automatic release and this
  reconciliation. Cancellation removes the capture override and cannot leave
  destroyed or display-suppressed nodes in the hover path.

The pressed-button prerequisite and release after the terminal event draw on
[Pointer Events capture](https://www.w3.org/TR/pointerevents3/#setting-pointer-capture)
and [implicit release](https://www.w3.org/TR/pointerevents3/#implicit-release-of-pointer-capture).
The captured hover policy follows the
[capture boundary-event rules](https://www.w3.org/TR/pointerevents3/#firing-events-using-the-pointerevent-interface).
The no-takeover rule and the cancellation triggers above are owned oxiview
contracts, not a claim of implementing the complete browser event model.

Ordinary geometric hit testing remains independent of the capture override.
Application/component code controls positions and stacking levels. Drag-target
queries, exclusion APIs, and high-level drag/drop behavior are separate work;
capture-based hover does not add them to M2. Handler-requested mutations follow
the dispatch boundary in [ADR-0029](0029-deferred-event-handler-mutations.md).
[ADR-0031](0031-hover-transition-notification-batches.md) defines the order
within a hover notification batch and defers its mutations until the batch
finishes. [ADR-0034](0034-pointer-input-processing-order.md) completes geometry
preparation and input/command scheduling, including automatic release and its
hover reconciliation before final-up callback commands are applied.
[ADR-0033](0033-hover-lifecycle-cleanup.md) defines cancel-before-hover-cleanup
and leave-before-destruction/suppression, callback observations, window
interruption recovery, and hover-path updates when captured ancestry changes.

[ADR-0030](0030-stationary-pointer-hover-updates.md) makes geometry-driven hover
updates under an uncaptured, stationary pointer opt-in through node styles.
That default does not gate capture acquisition/release reconciliation or
cancellation under this decision.

## Considered options

- **Cancel on window leave**: requires less outside-window lifecycle coverage,
  but interrupts a slider or divider drag at the window edge. The owner selected
  continued delivery, with native acceptance of outside-window movement/up.
- **Later requests replace the holder**: supports implicit takeover but allows
  an ancestor's handler to take capture from a child during bubbling. Explicit
  handoff makes this ownership change visible to the current holder.
- **Treat every release as cancellation**: gives one termination notification,
  but conflates a deliberate routing change with an interrupted interaction.
  Separate normal release and cancel preserve that distinction.
- **Geometric hover during capture**: keeps ordinary hover tied to pointer
  location and automatically notifies regions crossed during a drag when they
  are hit-testable. It can also trigger unrelated hover behavior, and still
  needs separate handling when a dragged node obscures a potential drop target.
- **Capture-based hover**: selected to keep routed input and boundary events
  associated with the same interaction owner. Release restores ordinary hover;
  applications needing independent spatial feedback use geometric hit testing.

## Consequences

- Platform events must distinguish ordinary boundary crossing from loss of
  input delivery. Headless tests alone cannot establish native outside-window
  delivery; the macOS demo must verify movement, release, and interruption.
- Mutation and destruction must preserve the holder long enough for cancel
  notification, then clear ownership even if propagation stops. Node generation
  and ownership checks must prevent later delivery to stale or foreign IDs.
- Tests cover competing and repeated requests, explicit handoff, multiple
  pressed buttons, early release, every cancellation trigger, and opacity or
  hit-test changes during capture. Hover fixtures distinguish the effective
  capture target from the geometric hit, including sibling overlap, a moving
  holder, boundary crossing, and release reconciliation. Component behavior
  will depend on these rules, so changing them later requires revisiting this
  decision.
