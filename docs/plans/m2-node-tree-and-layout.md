---
status: done
milestone: M2
---

# M2 node tree and layout: detailed plan

Goal: drive the existing native window from one retained UI node tree. Nodes
carry typed styles, layout results, painting content, and pointer handlers.
Mutations invalidate only the required style, layout, and paint work; Taffy
computes layout behind an owned protocol; hit testing routes pointer input to
nodes. The same application runs with macOS/GPU and headless/CPU composition.

Planning started on 2026-11-17 after [M1 close-out](m1-first-window.md).
The owner confirmed the detailed contracts below, then approved the remaining
decision groups 3-12 and requested a quickly runnable native preview using
parallel agents. The owner delegated implementation choices within this plan
and the accepted ADRs. M2 closed on 2026-11-20 when the owner confirmed that
the native Layout Lab passed manual acceptance. [Execution record](#execution-record)

## Decision-group progress

The remaining discussion was consolidated into twelve groups on 2026-11-17.
Previously confirmed decisions below stay in effect. Related details are
discussed together; internal names and fields are not separate approval items.
Groups 1 and 2 were individually confirmed. The owner then approved groups
3-12 together and delegated their remaining concrete choices to engineering
judgment for immediate preview implementation. All twelve groups are approved;
this does not mark their implementation or acceptance coverage complete.

| Group | Topic | Status |
| --- | --- | --- |
| 1 | Hover lifecycle: removal, suppression, interruptions, capture cancellation | Confirmed in [ADR-0033](../adr/0033-hover-lifecycle-cleanup.md) |
| 2 | Input processing order: geometry, hits, events, commands, automatic capture release | Confirmed in [ADR-0034](../adr/0034-pointer-input-processing-order.md) |
| 3 | Queue shutdown and reporting of unexecuted commands | Approved; concrete choices delegated |
| 4 | Bounds and recovery for command/hover feedback loops | Approved; concrete choices delegated |
| 5 | Supported tree depth and excessive-depth behavior | Approved; preview bound selected as 64 levels including the root; reject level 65 before structural mutation |
| 6 | Overall node, command, event, and batch-result API shape | Approved; concrete choices delegated |
| 7 | Custom-node measurement/painting/input contracts and reserved semantic data | Approved; concrete choices delegated |
| 8 | Owned layout inputs/results and Taffy integration over one tree | Approved; concrete choices delegated |
| 9 | Remaining style defaults, percentage bases, and numeric validation | Approved; concrete choices delegated |
| 10 | Dirty propagation, layout boundaries, and cache invalidation | Approved; concrete choices delegated |
| 11 | Layout/painting publication, coordinate conversion, and preparation failures | Approved; concrete choices delegated |
| 12 | S1-S8 delivery sequence, demo acceptance, observability, and performance metrics | Approved; concrete choices delegated; acceptance gates still apply |

## Confirmed decisions

### Milestone acceptance goal

The owner confirmed an interactive native layout demo with these outcomes:

- Nested layout updates correctly when the window is resized.
- Pointer input hits nodes and can trigger visible color changes.
- Interactions can add/remove nodes or change their dimensions, exercising
  layout and painting updates.
- An unchanged application produces no new frames.

Text, complete widgets, and reactive components remain in their later
milestones. This confirms the demo's acceptance goal, not the proposed layout
property subset, event semantics, API design, or implementation sequence.

### UI node identity

The owner confirmed the `NodeId` identity contract, recorded in
[ADR-0032](../adr/0032-owned-generational-ui-node-identity.md):

- UI `NodeId` is an opaque framework-owned value supporting copying, equality
  comparison, and hashing. Its internal fields are not part of the caller API.
- Identity distinguishes the owning tree, storage slot, and slot generation.
  Access through a different tree is rejected even when slot and generation
  numbers would otherwise match.
- Deleting a node permanently invalidates its old ID. Reusing its slot cannot
  make that ID identify a replacement node. Creating a replacement tree likewise
  cannot revive IDs belonging to a destroyed tree.
- Access with a stale or foreign-tree ID returns an explicit error. Identity
  counter exhaustion returns an error instead of wrapping or reusing an
  identity in a way that could revive an old ID.
- UI node identity and Scene node identity use distinct types and cannot be
  interchanged. Painting maintains any required association explicitly.

Copies of an ID can remain in command reports and diagnostics after the node
is removed; retaining the value does not keep that node alive. Queued creation
reserves a `NodeId` on enqueue under the contract below. Bit widths, arena
storage/library selection, allocator representation, and concrete structural
APIs remain separate design decisions. Root/subtree lifetimes and
creation/mounting/insertion/movement semantics are confirmed below.

### Root and subtree lifetimes

The owner confirmed the lifecycle contract recorded in ADR-0032:

- Creating a window's UI tree automatically creates its unique root node.
  The root's `NodeId` remains unchanged for that tree's lifetime.
- Removing an ordinary node removes the complete subtree rooted at that node.
  All removed nodes' IDs become invalid under the confirmed identity rules.
- Clearing window contents removes every descendant of the root and preserves
  the root itself, its ID, its own styles, and its handlers. New content can
  subsequently be added using the same root ID.
- Removing the root or assigning it a parent is rejected with an explicit
  error and no tree changes.
- Destroying the entire tree invalidates all node IDs, including the root.
  A newly created tree cannot revive any of those IDs.
- If removal includes the capture holder, deliver cancel before destroying it
  under ADR-0028. Clearing descendants leaves a capture held by the surviving
  root intact unless another confirmed cancellation condition applies.
- Release node-owned resources in ordered postorder: process each child's
  complete subtree in child-list order, then release the parent node's own
  resources. Each node's owned resources are released exactly once.
- Use an explicit stack for the framework's subtree teardown traversal. The
  teardown must not rely on recursive destruction of child nodes.

For P with children Q and R in that order, and Q with child S, release resources
in the order `S -> Q -> R -> P`. Parent-owned resources remain available while
descendant resources are released. Clearing contents applies the same order
to root descendants and retains the root's resources; whole-tree destruction
releases the root last.

These rules distinguish clearing contents from destroying the tree. Queued
commands still follow the confirmed mutation boundary, FIFO ordering, and
application-time validation. Insertion/movement semantics are confirmed below;
concrete method signatures and supported traversal limits are delegated
engineering choices. ADR-0034 defines global input processing order.
ADR-0033 completes lifecycle
staging: required cancel precedes hover cleanup, and leave notifications finish
before resource release.

### Deferred creation and mounting

The owner confirmed the creation/mounting contract recorded in ADR-0032:

- Applications may retain data, properties, or a creation function before
  requesting a real node. Keeping such a declaration does not itself allocate
  a live retained UI node; the caller decides when to request creation.
- Creating an ordinary node specifies a destination parent and append or
  indexed insertion. Successful execution creates and mounts the node as one
  structural operation. A rejected creation leaves no partially created live
  node or partial attachment.
- At observable operation boundaries, every live non-root node has exactly
  one parent and is reachable from the tree's unique root. M2 provides no
  independent detach operation that keeps a live node outside that tree.
- Moving an existing subtree remains one complete operation, without an
  observable detached intermediate state. Removal still destroys the subtree
  under the confirmed lifecycle rules.
- Creation and mounting do not imply immediate layout, painting, or frame
  presentation. Existing participation and scheduling rules govern that work.

This preserves ADR-0007's single retained tree: data and creation functions do
not require a second maintained virtual node tree. Callers can defer creation
to avoid unused node storage and initialization, or retain already created
nodes when preserving their identity/state is useful. Display none preserves
node storage even though it excludes the subtree from layout, painting, and
hits. Delayed creation can add work when content is first needed; recreating
removed nodes requires restoring any state kept outside their lifetimes.

M2 defines this node lifecycle boundary. Component/conditional construction
APIs remain in M3, and virtual lists remain in M7. The representation of
declarations and exact creation signatures remain separate decisions.
Queued creation uses the reserved-identity contract below; its reservation
is not a live detached node.

### Reserved identities for queued creation

The owner selected immediate `NodeId` reservation when an actual creation
command is enqueued, recorded in ADR-0032:

- Enqueueing creation returns a reserved `NodeId` that later queued commands
  can reference directly, including as the parent of another creation command.
  Keeping data, properties, or an unevaluated creation function alone reserves
  no node identity and allocates no live node.
- Reservation retains identity metadata and queued command data, but does not
  create a live node, change parent/child relationships, or participate in
  layout, painting, hit testing, or capture ownership. Other handlers continue
  to observe the pre-application live tree/style/capture state.
- The creation command owns the reservation lifecycle. Copying or discarding
  caller-held `NodeId` values does not cancel creation or release its reservation.
  Successful creation transitions the reservation to the live node's identity
  without changing the ID.
- Reading a reserved node before successful creation returns an explicit
  "not yet created" error. Exact Rust error names and method signatures remain
  separate API decisions.
- At its normal FIFO turn, the creation command validates against the current
  state. Success creates and mounts a live node using the same reserved ID.
  Later commands validate against the state at their own application time;
  merely possessing the ID does not make their targets live.
- If creation fails, its reserved ID becomes permanently invalid and must
  never identify a future node. Dependent commands fail individually when
  reached; independent later commands continue under ADR-0029. A failed child
  creation likewise permanently invalidates its own reserved ID.
- Failed creation releases its reservation and command-owned resources. If
  the queue is destroyed before the creation command executes, release those
  resources and permanently invalidate the reserved ID as well. Reclaimed
  storage may be reused only with a different identity; the old ID cannot
  revive. A node that was never created emits no enter, leave, or cancel event.
- Reservations follow the existing tree-ownership, generation, and checked
  exhaustion contract. A reservation does not permit identity wrapping or
  reuse that could revive a failed creation's ID. Tree destruction invalidates
  its outstanding reservations as well as its live node IDs.

For example, one callback can enqueue creation of panel P under the root,
creation of button Q under reserved P, and a style change targeting reserved Q.
Reads of P and Q during the callback report "not yet created". If both creations
succeed in order, Q's style command applies normally. If P's creation fails,
Q's creation fails because its parent is invalid, Q's reserved ID becomes
invalid, and its style command fails; an independent change to live R can still
succeed. The completed-batch report identifies each outcome and relevant IDs
without resolving a separate temporary-reference type.

Reservation storage and enqueue/error signatures are delegated implementation
choices. Cleanup on queue destruction does not select a separate
cancellation API or unfinished-batch reporting policy. The confirmed contract
does not change command ordering, dispatch boundaries, or completed-batch
reporting boundaries.

### Insertion, reordering, and reparenting

The owner confirmed these structural rules, recorded in ADR-0032:

- Children are ordered, each node has at most one parent, and the structure
  must remain acyclic.
- Support appending and insertion at a zero-based child index. For a destination
  list of length n, insertion accepts indices from 0 through n, with n appending.
- Moving an existing node within the same tree moves its complete subtree and
  preserves every affected node's ID, styles, and handlers. Evaluate layout in
  the new parent context; the resulting geometry may change.
- For a move, interpret the destination index against the destination child
  list after conceptually removing the node from its old position. Validate
  against the state at command application time. For a same-parent move, the
  destination list therefore contains one fewer child before insertion.
- Out-of-range indices, cross-tree moves, and moving a node into itself or
  one of its descendants return explicit errors without changing either tree,
  its parent links, or child order. Root removal/reparenting remains forbidden.
- Moving a node to its current parent and position is a successful no-op. The
  move itself introduces no structural invalidation or frame request and does
  not clear work already marked dirty by other changes.

For `[A, B, C, D]`, moving B to index 3 first considers `[A, C, D]` and produces
`[A, C, D, B]`. Moving B to index 1 preserves the original order. Index 4 is
invalid for that same-parent move because the post-removal list has length 3;
the rejected request must leave `[A, B, C, D]` intact.

These operation semantics leave concrete API signatures, reservation storage,
depth limits, and arena implementation to delegated engineering work. Creation,
mounting, and reserved IDs follow the confirmed rules above. Capture and hover
follow ADR-0033 for lifecycle cleanup and captured ancestry changes, and
ADR-0034 for global input processing order.

### Layout families

The owner selected Flex, Grid, and basic Block layout for M2. Acceptance must
cover their mixed nesting, rather than postponing Grid or Block to a later
milestone. This confirms the layout families only; their property subsets,
defaults, and edge-case semantics are confirmed separately below or delegated
under the approved implementation scope.

### Shared dimensions and box model

The owner confirmed the following shared property scope:

- Width and height accept auto, logical lengths, and percentages.
- Minimum/maximum width and height constraints and an optional aspect ratio
  are included.
- Margins are independent on each edge, with lengths, percentages, and auto;
  negative margin values are supported.
- Padding is independent on each edge, with nonnegative lengths and percentages.
- Border width is a nonnegative logical length, uniform across all four edges
  in M2.
- Box sizing is fixed to border-box: specified width and height include padding
  and borders.

Layout-specific properties, including Flex/Grid gaps, are discussed separately.

### Flex properties

The owner confirmed the following Flex property scope:

- Row and column directions, including their reverse variants.
- No wrapping, wrapping, and reverse wrapping.
- Flex grow, shrink, and basis; basis accepts auto, lengths, and percentages.
- Row and column gaps accepting nonnegative logical lengths and percentages.
- Main-axis distribution, cross-axis alignment, per-item alignment overrides,
  and multi-line alignment (`justify-content`, `align-items`, `align-self`, and
  `align-content`).

M2 does not expose an `order` property; structural node order expresses
reordering. Explicit baseline alignment is deferred to M4 with text support.

### Grid properties

The owner confirmed the following Grid property scope:

- Explicit row and column tracks sized by logical lengths, percentages, auto,
  fractional allocation (`fr`), and min/max track constraints (`minmax`).
- Explicit start/end grid-line placement and row/column spans.
- Sequential automatic placement by row or by column.
- Configurable sizing for implicit rows and columns created beyond the explicit
  grid.
- Row and column gaps accepting nonnegative logical lengths and percentages.
- Item alignment, per-item alignment overrides, and alignment/space distribution
  for the track collection.

Named lines and areas, subgrid, automatic repetition (`auto-fill`/`auto-fit`),
and dense placement are deferred beyond M2. Explicit baseline alignment is
deferred to M4, consistently with Flex.

### Block behavior

The owner confirmed the following basic Block behavior:

- Children follow tree order from top to bottom.
- Auto width uses available horizontal space; auto height is determined by
  normal-flow content, subject to the confirmed size constraints.
- A specified width with left and right auto margins supports horizontal
  centering.
- Adjoining vertical margins collapse according to the standard rules,
  including siblings and eligible parent/child and empty-block cases. The
  already confirmed negative-margin support participates in those rules.
- Block containers may contain Flex, Grid, and Block nodes.
- M2 Block has no gap property, floats, or inline formatting. Positioning is a
  separate decision and is not confirmed by this scope.

Use the W3C rules for [normal-flow block sizing](https://www.w3.org/TR/CSS22/visudet.html#blockwidth)
and [margin collapsing](https://www.w3.org/TR/CSS22/box.html#collapsing-margins)
when documenting the owned properties and constructing numerical fixtures.

### Positioning

The owner confirmed one positioning model across Flex, Grid, and Block:

- Static is the default: normal-flow layout with no applied inset offset.
- Relative positioning applies offsets after normal-flow layout, preserving
  the original occupied space and leaving other nodes' positions unchanged.
- Absolute positioning removes the node from normal flow without reserving
  space. Its reference is the nearest relative or absolute ancestor, or the
  window's logical viewport when there is no such ancestor.
- Top, right, bottom, and left insets accept auto, logical lengths, and
  percentages; negative offsets are supported.
- Fixed and sticky positioning are outside M2.

The [upstream positioning model](https://docs.rs/taffy/latest/taffy/style/enum.Position.html)
supports this distinction between normal-flow placement, relative offsets, and
positioned ancestors. S4 still reviews the selected dependency release.

### Painting styles and clipping

The owner confirmed the following painting style scope:

- Solid backgrounds support color alpha and default to transparent.
- Borders use one solid color and one width across all four edges; the default
  width is zero.
- Each corner has an independent nonnegative circular radius in logical units,
  defaulting to zero. Percentage and elliptical radii are outside M2. When
  adjacent radii would overlap, scale all used radii proportionally to fit.
- Opacity is in the range 0 to 1, defaults to 1, and applies to the node and its
  subtree as a group.
- Overflow defaults to visible. Clip applies to both axes together and clips
  overflowing content at the padding box with the corresponding rounded
  corners. It introduces neither scrolling nor a new formatting context, so
  clipping alone does not prevent the confirmed margin collapsing behavior.
- M2 adds no style APIs for gradients, shadows, background images, or dashed
  borders. Existing Scene painting capabilities are not removed by this scope.

The numerical and visual fixtures follow the
[corner-overlap rule](https://www.w3.org/TR/css-backgrounds-3/#corner-overlap)
and the [clip definition](https://www.w3.org/TR/css-overflow-3/#valdef-overflow-clip).
Painting order and hit-test eligibility are confirmed separately below.

### Z-index and stacking scope

The owner requires z-index in M2 so painting order can be controlled without
changing structural node order or normal-flow layout. The earlier proposal to
use tree order alone is not the selected scope.

The owner selected CSS stacking-context semantics for the properties supported
in M2. An ordinary parent does not automatically isolate its descendants'
stacking levels. A node that establishes a stacking context groups its
descendants for ordering against content outside that context. Parent-local
sorting at every node is not the selected model.

The owner confirmed the following rules, recorded in
[ADR-0027](../adr/0027-z-index-stacking-contexts.md):

- Z-index defaults to auto and also accepts positive, zero, and negative
  integers.
- It applies to relative/absolute nodes and Flex/Grid items, including static
  items. An ordinary static node outside those item roles has no effective
  z-index.
- The root establishes a stacking context. An eligible node with non-auto
  z-index, including zero, also establishes one. Any node with opacity below
  one establishes a context independently of z-index applicability.
- Background color alpha, corner radii, and overflow clipping alone do not
  establish stacking contexts.
- Painting follows CSS painting phases for the supported properties. Within
  the same context, phase, and stack level, tree order breaks ties, with later
  content on top. A descendant context is atomic in its containing context.
- Auto does not itself create a context; explicit zero creates one where
  z-index applies. Descendants of an ordinary wrapper can therefore compete
  with content outside that wrapper.
- Establishing a stacking context does not change the containing block used
  for absolute positioning.

The ADR links the reference rules and explains the alternatives. Hit-test
eligibility, pointer event propagation, and outside-window capture scope are
confirmed below, together with capture acquisition, release, and cancellation.
Captured hover behavior and the callback mutation boundary are also confirmed
below. Detailed input processing order follows ADR-0034.

### Hit testing and participation

The owner confirmed the following hit-test rules:

- Search from front to back in final painting order, respecting stacking
  contexts, and select the first eligible node at the pointer position.
- A node's hit region is its border box, including the border and excluding
  margins. Rounded corners exclude the area outside the curved border edge.
- Effective rectangular or rounded clips exclude clipped regions from hits.
  With visible overflow, a child can be hit outside its parent's border box
  when no effective clip excludes that position.
- A transparent background or zero opacity does not disable hits. M2 uses
  geometry rather than per-pixel alpha tests.
- Nodes participate by default, including nodes with no event handler. An
  absent handler does not make the node transparent to hit testing.
- An independent per-node hit-test switch can exclude the node itself. It does
  not propagate to descendants: children retain their own participation and
  can still be hit. Disabling this switch does not hide the node or its children.
- Display none excludes the entire subtree from layout, painting, and hits.

For example, a transparent overlay still blocks nodes behind it by default.
Disabling the overlay's own participation permits hits through its empty areas
while preserving hits on its participating children. Event propagation through
ancestors follows the separately confirmed rules below.

The geometry follows the [rounded border-edge rule](https://www.w3.org/TR/css-backgrounds-3/#corner-clipping).
Keeping opacity independent of participation follows the
[opacity hit-testing rule](https://www.w3.org/TR/css-color-4/#transparency).

### Pointer events and propagation

The owner confirmed the following event and propagation scope:

- M2 exposes move, down, up, enter, leave, and cancel pointer events.
- Move/down/up/cancel first reach their target, then bubble through its
  structural ancestors to the root. Z-index does not change this route.
- Enter/leave are direct notifications to the nodes affected by ancestor-path
  changes; they do not bubble.
- A handler can stop further bubbling. This prevents delivery to remaining
  ancestors and does not redirect the event to an obscured node underneath.
- An ancestor excluded from hit testing by its own participation switch can
  still receive events bubbling from its descendants.
- Event data includes a stable target, the current receiving node, logical
  window coordinates and coordinates local to the current node, mouse button
  state, and the original timestamp.
- M2 uses target delivery followed by bubbling. A root-to-target capture
  propagation phase is deferred. Click and double-click synthesis are deferred
  to later component work.

The bubbling distinction follows the
[Pointer Events event-type rules](https://www.w3.org/TR/pointerevents3/#attributes-and-default-actions).
Pointer capture, which redirects delivery during an interaction, is separate
from a capture propagation phase. Its outside-window scope is confirmed below;
its acquisition, release, and cancellation rules are also confirmed below.
Captured hover behavior and deferred mutation during handlers are confirmed
separately below. Detailed input processing order follows ADR-0034.

### Pointer capture scope

The owner selected capture that continues outside the originating window:

- During a captured drag, leaving the captured node or the window does not
  end capture. The captured node continues receiving movement and button
  release events, including events with positions outside the window.
- Crossing the window boundary or returning inside does not by itself cancel
  the interaction or change its capture target.
- M2 acceptance includes native outside-window movement and release, together
  with deterministic headless coverage of the same event sequence. Normal
  boundary crossing must not be mistaken for a platform interruption.

The November 17 native preview verified dragging and releasing beyond the
right window edge. The owner confirmed the complete native review on
November 20. [Execution record](#execution-record)

### Pointer capture lifecycle

The owner confirmed the following rules, recorded in
[ADR-0028](../adr/0028-pointer-capture-ownership-and-lifecycle.md):

- Capture is requested explicitly while at least one mouse button is pressed.
  The target must be a live node in the same window, not suppressed by display
  none on itself or an ancestor.
- Each pointer has one capture holder. A repeated valid request by that holder
  has no effect. Another node must wait for the current holder to release;
  requests cannot take ownership from it implicitly.
- Subsequent move/down/up events go to the capture holder, then bubble through
  its structural ancestors under the confirmed propagation rules.
- Releasing the last pressed mouse button delivers the final up event before
  automatically releasing capture.
- The holder may release early. Subsequent delivery returns to ordinary hit
  testing; explicit release synthesizes neither up nor cancel.
- Window deactivation, hiding, destruction, or system input interruption
  cancels capture. Removing the holder, or suppressing it with display none
  through its own or an ancestor's state, also cancels capture.
- Exceptional cancellation sends one cancel notification and clears capture.
  Notify before destroying the affected node; stopping cancel propagation
  cannot prevent framework cleanup.
- Changes to opacity or to the holder's own hit-test participation do not
  automatically cancel existing capture.

This uses explicit handoff rather than allowing handlers on the same bubble
route to take capture from each other. Captured hover and the callback mutation
boundary follow the confirmed rules below. ADR-0034 orders automatic release
and hover reconciliation before applying final-up callback commands.

### Hover and boundary events during capture

The owner selected capture-based hover (option B), recorded in ADR-0028:

- When reconciling hover without capture, select the target by ordinary
  geometric hit testing. Geometry changes under a stationary pointer do not
  trigger this reconciliation by default, as confirmed separately below.
  During capture, the holder is the effective hover target, and its structural
  ancestors form the hover path.
- Moving over another node, leaving the holder's box, or leaving the window
  does not by itself move hover away from the holder. These geometric changes
  do not emit leave for the holder or enter for the node under the pointer
  while capture remains active.
- Moving the captured node or changing overlap does not end capture or retarget
  hover. The holder's own hit-test participation switch likewise does not
  remove it from the captured hover path.
- Acquisition reconciles hover with the holder's path. Normal release restores
  ordinary geometric hit testing and the corresponding enter/leave changes,
  even if the pointer has not moved. Releasing outside the originating window
  leaves no ordinary hover target in that window.
- The final up still reaches the holder before automatic release. Exceptional
  cancellation removes the capture override under the confirmed lifecycle;
  it must not leave a destroyed or display-suppressed node in the hover path.

For nonoverlapping siblings P and Q, capture on P followed by movement over Q
keeps hover on P. Releasing over Q delivers the final up to P, releases capture,
and then reconciles hover from P to Q. This follows the
[Pointer Events capture boundary rule](https://www.w3.org/TR/pointerevents3/#firing-events-using-the-pointerevent-interface)
and [release reconciliation](https://www.w3.org/TR/pointerevents3/#implicit-release-of-pointer-capture).

Geometric hit testing remains independent of capture. Applications/components
control positions and z-index; drag-target queries and any exclusion interface
remain separate design work. Selecting B does not add a high-level drag/drop
system to M2. The callback mutation boundary and ordering within a hover
notification batch are confirmed below. ADR-0033 defines cancellation/cleanup
and captured reparenting. ADR-0034 defines global input, command, geometry, and
automatic-release ordering.

### Hover updates for a stationary pointer

The owner selected updates only through an explicit style opt-in, recorded in
[ADR-0030](../adr/0030-stationary-pointer-hover-updates.md):

- With an uncaptured, stationary pointer, changes to live nodes' geometry,
  stacking, clipping, or hit-test participation do not automatically reconcile
  hover or emit enter/leave by default.
- A node's style exposes `stationary_hover_updates: bool`, defaulting to
  `false`, to explicitly enable automatic hover updates under a stationary
  pointer. The property is not inherited. Merely declaring a hover style does
  not enable it.
- With no capture, inspect the previous hover path and the new geometric hit
  path, including their structural ancestors. An enabled option on any node
  in either path triggers a complete hover-path reconciliation. If neither
  path has an enabled node, geometry changes alone do not trigger an update.
  An enabled node outside both paths does not trigger an update.
- A triggered reconciliation updates the complete hover path and sends the
  corresponding direct enter/leave notifications to all affected nodes,
  including nodes whose own option is disabled. Unchanged path membership
  does not produce duplicate notifications. An ancestor's enabled option
  qualifies independently of a descendant's disabled option.
- Enabling the option schedules one eligibility check after the current
  command batch is applied and any necessary geometry preparation completes.
  Use the final styles and geometry: if the enabled node is on the old hover
  path or new geometric hit path, reconcile hover. An unchanged path produces
  no duplicate enter/leave; intermediate option values within a batch do not
  produce their own boundary notifications.
- Disabling the option preserves current hover and removes that node as a
  trigger for future stationary-pointer updates. Other enabled nodes on either
  relevant path still qualify. A descendant's `false` cannot override an
  ancestor's `true`, because the ancestor participates directly in the path
  check rather than supplying an inherited property value.
- This is an interaction policy exposed through typed styles. It does not
  bring the state-variant style APIs or reactive signals from M5/M3 into M2.
- Skipping an automatic hover update does not freeze geometry or geometric
  hit-test results. Actual pointer input still follows the confirmed hit and
  routing rules and ADR-0034's geometry preparation and processing order.
- Capture continues to supply the effective hover target. Acquisition and
  release still reconcile hover under ADR-0028, even without pointer movement;
  cancellation and safe node cleanup are not disabled by this style option.

For live nodes P and Q with no enabled option on either relevant path, moving P
away from a stationary pointer to expose Q does not by itself emit P.leave or
Q.enter. The retained hover target can therefore differ from the current
geometric hit until a later hover reconciliation. If P is disabled and Q is
enabled, Q becoming the new geometric hit triggers a complete switch, including
P.leave and Q.enter. The same rule applies when only P or a relevant ancestor
is enabled. Shared ancestors whose path membership stays unchanged receive no
extra enter/leave.

The property name, type, default, lack of inheritance, and runtime switching
rules are confirmed. The order within each hover notification batch is
confirmed below. ADR-0033 defines mandatory cleanup on removal/suppression and
conditional replacement entry, including option values retained from deleted
old-path nodes. The default does not permit dispatch to destroyed nodes.
Capture acquisition, release, and cancellation retain their confirmed lifecycle
rules; switching this option during capture does not change the effective
hover target.

### Hover lifecycle cleanup

The owner confirmed decision group 1 in
[ADR-0033](../adr/0033-hover-lifecycle-cleanup.md):

- Removal or clear-contents removes affected nodes from hover before destroying
  them. Display none on a node or ancestor performs the same cleanup while
  retaining node storage. Preserve surviving, unsuppressed ancestor membership.
  These exits do not depend on `stationary_hover_updates`.
- Validate a requested removal/suppression first. Rejected commands emit no
  cleanup events. If capture is affected, send one cancel while nodes remain
  readable; cancel callbacks observe old capture and hover. Then clear that
  capture, publish cleaned hover, and send leaves from inner to outer. Leave
  callbacks observe the cleaned state. Complete suppression or ordered resource
  release only after notifications. Callback mutations remain queued and
  stopped cancel propagation cannot prevent cleanup.
- Pointer leave clears hover without capture and preserves the holder path
  with capture. Window deactivation, hiding, destruction, or system input
  interruption cancels any capture and clears the complete hover path in the
  same cancel-before-clear-before-leave order. Repeated interruptions produce
  no duplicate cancel/leave. Never-created nodes receive none of these events.
- If cancellation ends capture while the pointer is outside the window,
  clear the complete hover path. Interruption or restoration alone produces
  no new enter; the next valid pointer input rebuilds hover.
- After removal/suppression exposes a replacement target, perform complete
  stationary-pointer reconciliation only if either relevant path opts in,
  after the command batch and required geometry preparation. Deleted old-path
  nodes contribute their last option value before deletion; surviving nodes
  contribute their final batch value. Do not read reclaimed storage or repeat
  leaves already delivered during cleanup.
- Reparenting a captured node or its ancestry preserves its holder and updates
  hover to the new ancestor path. Shared node identities, including the holder,
  receive no duplicate notifications. Moving into suppressed ancestry instead
  follows hiding cleanup, including capture cancellation.
- Normal capture release still reconciles geometric hover without requiring
  the stationary update option. Changes to geometry, stacking, clipping,
  opacity, or hit-test participation retain the previously confirmed policies;
  they do not become unconditional cleanup triggers.

For stationary P hiding/removal to expose Q, required P.leave happens before
suppression/destruction. With no qualifying opt-in, Q gets no enter merely from
that exposure. With a qualifying old/new path, a later complete reconciliation
can enter Q without repeating P.leave. Global input processing order follows
ADR-0034. Queue shutdown/reporting and feedback bounds are delegated work in
approved groups 3 and 4.

### Pointer input processing order

The owner confirmed decision group 2 in
[ADR-0034](../adr/0034-pointer-input-processing-order.md). For positional
move/down/up input:

1. Apply older queued mutation batches before selecting the next input target.
2. Update the pointer position/button snapshot and refresh dirty geometry as
   needed; fix the target, ancestor route, and coordinates for this input.
3. Publish hover and deliver required leave/enter notifications. Their callback
   commands wait through the associated primary event.
4. Deliver move/down/up to the fixed target and along the fixed bubble route.
5. On the final button up, automatically release capture and reconcile hover
   before applying callback commands.
6. Apply command batches FIFO. Deliver each completion report before checking
   stationary hover against final batch state with necessary geometry prepared.

Thus Q.enter requesting Q's removal still permits `Q.enter -> Q.down -> remove
Q`. P's final up requesting self-removal gives `P.up -> normal capture release
-> hover reconciliation -> remove P`, without canceling an already ended
capture.

Capture acquisition, explicit release, captured-path reparenting, and required
lifecycle cleanup deliver notifications within their corresponding command,
before it finishes. Generated commands append to the queue tail without
recursive execution. Ordinary stationary updates use final batch state.
Geometry preparation does not present; rendering remains coalesced through the
existing frame scheduler. Budget/yield details and preparation failure handling
are delegated to approved groups 4 and 11.

### Mutation during event dispatch

The owner confirmed deferred application of handler mutations, recorded in
[ADR-0029](../adr/0029-deferred-event-handler-mutations.md):

- Fix the current event's target and structural propagation route before
  invoking its handlers. Tree changes requested during delivery cannot change
  that event's route.
- Handlers submit tree, style, and pointer-capture changes as mutation commands
  instead of applying them to the live tree during dispatch.
- For ordinary event dispatch, apply those commands in submission order after
  the complete dispatch finishes, including when a handler stops further
  bubbling. The boundary is the whole dispatch, not each individual handler.
  The later-confirmed hover notification batch rule below extends this boundary
  through all enter/leave notifications for one transition.
- Reads of tree, style, and capture state during the same dispatch see the
  state before these commands are applied. This includes reads by the handler
  that submitted a command and by later handlers on the route.
- Stopping propagation is immediate dispatch control, not a deferred mutation
  command. It skips remaining ancestor handlers without discarding commands
  already submitted.
- A handler can request its own node's removal without destroying it during
  the current dispatch. When removal is applied, existing capture cancellation
  rules still require cancel delivery before destroying the holder.

For example, if P requests its own removal, its ancestors still receive the
current event unless propagation is stopped. Removal happens after that
dispatch; if P still holds capture, cancel is delivered before destruction.
Setting a style and reading it back in the same dispatch returns the old value.

This boundary covers framework-owned tree, style, and capture changes, not all
application-local variables. It does not require a second tree or a full-tree
copy. Validation rejection, partial success, queue ordering, and batch-result
reporting follow the confirmed rules below. Concrete Rust types/callback
signatures are delegated engineering choices; geometry/input scheduling follows
ADR-0034.

### Validation of queued mutation commands

The owner confirmed rejection of an individual invalid command while continuing
the batch, recorded in ADR-0029:

- Validate each command when it reaches its turn for application, against the
  current state, including earlier successfully applied commands. Enqueueing
  alone does not establish that a command will be valid when applied.
- A validation failure rejects that command without applying any part of its
  state change. Record an error that identifies the rejected command and the
  reason; do not silently ignore it.
- Continue validating and applying later commands in submission order. Each
  later command is checked against the state resulting from earlier successes;
  dependent commands may fail independently.
- Preserve successful commands already applied. The batch permits partial
  success and does not roll back the batch on validation failure.

For commands that delete Q, set Q's width, and set R's color in that order:
deletion succeeds, the width update is rejected because Q is no longer live,
and R's valid color update still succeeds. Q stays deleted.

This applies to command validation failures under the existing
[error conventions](../conventions/unsafe-panic-errors.md). It does not change
fatal-error or unexpected-panic handling. Completed batches provide the result
reports confirmed below; concrete Rust types and callback signatures remain
separate API decisions. Ordinary dispatch and hover notification batch
boundaries still determine when application begins.

### Commands produced during command application

The owner confirmed queue-tail insertion for new mutation commands, recorded
in ADR-0029:

- Commands submitted by callbacks while an existing command batch is being
  applied join the tail of the existing mutation queue. Complete the current
  batch and preserve already queued work ahead of them.
- Preserve submission order within each batch and first-in, first-out ordering
  across queued batches. Do not recursively drain newly submitted commands
  inside the current command or callback.
- The dispatch boundary or hover notification batch boundary determines when
  callback commands become eligible for application; it does not let them
  overtake existing queued commands.
- Required lifecycle notifications still occur at their specified points.
  In particular, removal of a captured node sends cancel before destruction,
  but mutations submitted by that cancel callback wait behind existing work.
- Deferred commands are validated when their turn arrives, including against
  removals already completed. Application-local variable cleanup remains
  outside the mutation queue and can run during the callback.

For an existing batch C1 (delete P), C2 (modify R), suppose C1's cancel callback
submits C3 (modify S). Deliver cancel while P is alive, complete C1 including
P's destruction, apply C2, then apply C3. If C3 instead modifies P, it is rejected
because P has already been destroyed. Earlier successful changes remain applied.

This fixes mutation-command queue ordering. Result reports follow the confirmed
rules below. ADR-0034 places input/hover processing and automatic release among
these batches; bounds on generated work are delegated engineering choices.

### Command batch reports

The owner confirmed completed-batch reporting, recorded in ADR-0029:

- Every completed mutation command batch provides one report listing each
  command's success or failure in submission order, including partial success.
- Reports identify the batch and each command's sequence number, include
  relevant `NodeId` values where applicable, and provide a structured reason
  for each failure. Reporting a removed or invalid node's identity does not
  require accessing that node's storage.
- Deliver the completed report to the application's result callback on the
  window thread after the batch finishes. Do not invoke this result callback
  between commands in the batch.
- Retained tree/style/capture mutations requested by the result callback become
  new commands at the queue tail. They cannot alter the completed batch's
  results or overtake older queued work.
- When no custom result handler is supplied, the default handler logs failures
  with their identifying context and reasons. Successful commands produce no
  default log entries.

For delete-Q, set-Q-width, set-R-color, the report contains success, failure
with the invalid-node reason, and success in that order. A result-handler
response is a later command batch and receives its own results when completed.
Success records command application; frame preparation and presentation follow
their scheduling rules. Concrete Rust types and callback signatures are
delegated implementation choices.

### Hover transition notification batches

The owner confirmed the following ordering and mutation boundary, recorded in
[ADR-0031](../adr/0031-hover-transition-notification-batches.md):

- Fix the old and new hover paths for one transition, then update the effective
  hover state to the new path before invoking any boundary handler. Every
  callback in the batch reads this new hover state.
- Notify nodes present only in the old path with direct leave events, ordered
  from the innermost node toward the root.
- After all leaves, notify nodes present only in the new path with direct
  enter events, ordered from the root toward the innermost node.
- Compare membership by node identity. Nodes present in both paths receive no
  enter/leave, even if their position in the path changes. Identical paths
  produce no boundary notifications.
- Treat the entire leave/enter sequence as one hover notification batch. Queue
  tree, style, and capture commands submitted by any of its callbacks and apply
  them in submission order only after all notifications finish. No such command
  can remove or reparent a later recipient in the middle of this batch.
- Tree, style, and capture reads remain unchanged by those queued commands
  throughout the batch, while hover reads already reflect the new path.
  Application-local variables remain outside this retained-state guarantee.

For `Root -> A -> P` becoming `Root -> B -> Q`, delivery is
`P.leave -> A.leave -> B.enter -> Q.enter`; Root receives no boundary event.
If P.leave requests Q's removal, Q remains alive, receives this batch's enter,
and is removed only after the batch. Any required capture cancellation still
precedes actual destruction under ADR-0028.

This refines ADR-0029 for related boundary notifications; move/down/up/cancel
retain the ordinary single-event mutation boundary. It specifies delivery once
a hover transition is required and does not add new stationary-pointer triggers.
Commands produced by the batch follow ADR-0029's queue-tail rule. ADR-0034
defines placement of the notification batch relative to input and queued
commands. Bounds for callbacks that repeatedly generate more transitions are
delegated engineering work. ADR-0033 defines the cleanup staging above.

## Starting point at M1 close-out

- [`oxiview-core`](../../crates/oxiview-core/src/lib.rs) already owns `Scheduler`,
  `Application`, `PaintedScene`, and the window-thread loop. `Application::event`
  requests invalidation; `Application::paint` produces a complete Scene.
- [`oxiview-platform`](../../crates/oxiview-platform/src/interface.rs) delivers
  pointer coordinates in logical units, physical window sizes, scale changes,
  input capture timestamps, and resize transactions. M2 subsequently added
  pointer-leave and window-lifecycle events at the port and node routing in core.
- [`oxiview-scene`](../../crates/oxiview-scene/src/lib.rs) already has a retained
  painting representation and its own `NodeId`. Scenes use physical pixel
  coordinates at the presentation boundary. A Scene entry is not a UI node.
- [`oxiview-app`](../../crates/oxiview-app/src/lib.rs) composes the same core loop
  with the headless CPU presenter or native GPU presenter. No second window loop
  is needed for M2.
- [`xtask/layers.toml`](../../xtask/layers.toml) reserved `oxiview-layout` and
  `oxiview-style` before M2 created them. Only `oxiview-layout` may depend
  directly on Taffy.
- The M1 `window` example is also the native latency and idle measurement fixture.
  Its measurement modes and workload remain comparable to their recorded baseline.

## Scope

**Included**:

- One generational arena UI tree per window, safe structural mutation, and owned
  `NodeId` values that reject stale and foreign-tree access. Callers decide when
  to request real nodes; successful creation mounts them in that tree.
- Separate style, layout, and paint dirty flags; layout cache invalidation and
  layout boundaries; coalesced on-demand frame requests.
- Typed box, Flex, Grid, Block, and paint properties, plus the explicit
  stationary-pointer hover update option, within the subset below. Properties
  are set imperatively until M3 adds signals.
- An owned layout protocol, a deterministic test implementation, and the default
  Taffy implementation. Custom leaf measurement works without a text dependency.
- Tree-to-Scene painting with z-index, rectangular and rounded clipping,
  consistent paint order, logical/physical coordinate conversion, and reuse of
  clean work.
- Hit testing, pointer move/down/up/enter/leave/cancel delivery, target and bubble
  routing, and explicit pointer capture for one desktop pointer per window,
  including captured movement and release outside the originating window.
- Reserved node semantics and custom measurement/painting/event extension points.
- A native and headless demo, deterministic tests, pixel snapshots, tracing, and
  benchmarks integrated into the existing local verification tools.

**Excluded**:

- Signals, scopes, function components, the component Builder API, and executors
  (M3). M2's imperative node operations are the primitives those APIs will use.
- Text shaping, text measurement through Parley, and the Vello GPU path (M4).
- Focus traversal, keyboard routing, Actions/Keymaps, state-variant style APIs,
  themes, and system preference signals (M5); text editing and IME (M6).
- Widget libraries, scrolling, virtualization, and the 100,000-row scrolling
  target (M7); animation and transitions (M8).
- Multiple windows, shared-thread mode, native drag and drop, and system services
  (M9); independent compositing layers and continuous rendering (M10).
- Damage regions and partial surface repaint. A produced frame still presents
  the whole window; reusing layout or painting content does not imply partial
  GPU repaint or an implemented repaint boundary.
- Platform accessibility adapters (M12), Windows/Linux backends (M14/M15), CSS
  parsing/cascading/selectors, and browser targets.

## Design constraints

These constraints apply [ADR-0007](../adr/0007-single-arena-node-tree.md),
[ADR-0008](../adr/0008-typed-style-with-css-vocabulary.md),
[ADR-0009](../adr/0009-layout-protocol-taffy-default.md),
[ADR-0014](../adr/0014-accessibility-deferred.md),
[ADR-0027](../adr/0027-z-index-stacking-contexts.md),
[ADR-0028](../adr/0028-pointer-capture-ownership-and-lifecycle.md),
[ADR-0029](../adr/0029-deferred-event-handler-mutations.md),
[ADR-0030](../adr/0030-stationary-pointer-hover-updates.md),
[ADR-0031](../adr/0031-hover-transition-notification-batches.md),
[ADR-0032](../adr/0032-owned-generational-ui-node-identity.md),
[ADR-0033](../adr/0033-hover-lifecycle-cleanup.md), and
[ADR-0034](../adr/0034-pointer-input-processing-order.md). They do not
supersede those decisions. If implementation needs to change tree ownership,
dependency direction, or an accepted boundary, apply the
[ADR criteria](../conventions/git-workflow.md) before making that change.

### Ownership and crate boundaries

| Owner | M2 responsibility | Boundary |
| --- | --- | --- |
| `oxiview-core` | UI arena, `NodeId`, node behavior, dirty propagation, style-to-layout projection, painting traversal, hit testing, pointer dispatch | Depends on owned ports; no direct Taffy or renderer dependency. Tree and callbacks stay on the window thread and need not be `Send`. |
| `oxiview-style` | Author-facing typed properties, defaults, validation, and change classification | Uses owned base geometry/color values. No layout engine, platform objects, signals, themes, or Scene instruction types in style signatures. |
| `oxiview-layout` | Owned constraints, layout inputs/results, traversal and measurement protocol; private Taffy adapter and caches | Depends on base and Taffy, never core or style. The protocol accepts caller-owned node identity through an associated type; Taffy identities stay private. |
| `oxiview-base` | Existing shared geometry and colors | Add shared values only when multiple contexts actually need them; do not move UI ownership here to break a dependency cycle. |
| `oxiview-scene` | Existing painting language | UI nodes emit Scene entries. Scene identity remains independent of UI identity and carries no UI parentage, style, or event state. |
| `oxiview-platform` and its backends | Raw pointer lifecycle and window activation notifications required to clear interaction state | Both macOS and headless implement any added event. Node hit testing remains in core. |
| `oxiview-app` | Select layout and rendering implementations and connect the tree application to the existing loop | Owns integration tests that need core, platform, and rendering adapters together. |

Core projects style properties into the layout port's owned input values. This
keeps the existing allowlists valid: neither style nor layout needs a dependency
on the other. The test layout implementation uses those same inputs and results.

Use `oxiview_core::NodeId` for UI identity. At call sites combining both domains,
import the existing Scene identity as `SceneNodeId` or qualify it explicitly;
`PaintedScene::root` remains a Scene identity. Do not cast between them or assume
a one-to-one mapping. One UI node may emit several Scene entries or none.
ADR-0032 confirms the owned, opaque UI identity and its separation from Scene
identity; the concrete bit layout and arena implementation remain undecided.

The UI tree is authoritative for parentage and child order. Use Taffy's low-level
integration over that tree, with private adapter state for converted inputs,
identity mapping, and caches. Do not synchronize a second `TaffyTree` topology.
Taffy documents a low-level API for embedding its algorithms in an existing UI
tree; the embedder owns storage, caching, and algorithm dispatch.
[Upstream API documentation](https://docs.rs/taffy/latest/taffy/) and
[tree traits](https://docs.rs/taffy/latest/taffy/tree/traits/index.html) were
consulted on 2026-11-17. This is feasibility evidence, not a dependency-version
approval; S4 reviews and pins the selected release and enabled features.

The layout protocol and node APIs remain internal and unstable under the
[API conventions](../conventions/api-stability.md). A test implementation checks
the boundary; it does not establish full layout equivalence or public stability.

### Tree invariants and extension points

The identity, root/subtree lifecycle, creation/mounting, queued ID reservation
and cleanup, and insertion/movement rules in ADR-0032 are confirmed. Concrete
APIs, reservation storage, traversal limits, and extension-point contracts are
delegated engineering choices within the approved scope below.

- Each window tree creates one root whose ID persists until tree destruction.
  Clearing contents removes its descendants while preserving the root's own
  styles and handlers. Root removal or reparenting fails without tree changes.
- Data and creation functions can exist before a live node is requested.
  Ordinary-node creation specifies a parent and mounts the node on successful
  execution. Every live non-root node is reachable from the root with exactly
  one parent; no independent detach keeps a live node outside the tree. Moves
  expose no detached intermediate state, and failed creation leaves no live
  orphan. This does not add M3 construction APIs or M7 virtualization to M2.
- Enqueueing actual creation reserves its `NodeId` for later commands without
  adding a live node. Reads before successful creation return "not yet created".
  Successful creation mounts the node with that ID; failure permanently
  invalidates it. Dependent failures and independent successes follow the
  existing FIFO validation/reporting rules.
- Creation commands own their reservations independently of caller-held ID
  copies. Failure or queue destruction before execution releases reserved and
  command-owned resources and permanently invalidates the ID. Storage reuse
  requires a different identity. Never-created nodes emit no enter/leave/cancel.
- Nodes have at most one parent and ordered children; insertion and reparenting
  preserve acyclicity. Support append and zero-based insertion. Moves use the
  destination index after conceptual detachment, preserve subtree IDs/styles/
  handlers, and evaluate layout in the new parent context. Reject invalid
  indices, cross-tree moves, and cycles without changing the tree. Moving to
  the existing parent/position is a successful no-op.
- Follow ADR-0032: an opaque, copyable, equality-comparable, hashable UI ID
  distinguishes tree ownership, storage slot, and generation. Reuse cannot
  revive stale IDs, including after tree destruction/recreation. Stale and
  foreign-tree access and identity counter exhaustion return explicit errors;
  counters never wrap. Keep Scene identity a separate type.
- Removing an ordinary node removes all its descendants and invalidates all
  their IDs; destroying the tree also invalidates the root. Release node-owned
  behavior and cached data exactly once in child-list-ordered postorder using
  an explicit stack. Preserve parent resources until their descendants have
  been released; clear retains the root, while whole-tree teardown releases
  it last. ADR-0033's required cancel and hover leaves precede resource release.
  Structural validation failures preserve parent links, child order, and node
  state; concrete API signatures are delegated implementation choices.
- Structural operations use safe Rust and bounded or iterative traversal. Define
  and enforce the preview depth limit of 64 levels, with the root at level 1.
  Insertion or reparenting that would create level 65 returns an error before
  changing the tree; the layout port shares the same bound. Framework teardown
  retains its explicit-stack traversal. Full integration coverage remains
  pending; the limit does not claim arbitrary-depth support.
- A custom node can provide intrinsic measurement, paint content, and pointer
  handling through owned core contracts. M2 is not a closed rectangle-only node
  enum. Painting consumes committed layout and does not mutate tree structure.
- Reserve optional owned semantic data for role, label, value, and supported
  Actions, with round-trip and destruction tests. This data has no platform
  accessibility behavior or Action dispatch in M2.

### Typed property subset and units

| Group | M2 contract |
| --- | --- |
| Dimensions | Width/height accept auto, logical lengths, and percentages; min/max width/height constraints and optional aspect ratio are included. The root receives the logical viewport size. |
| Box model | Per-edge margins accept lengths, percentages, and auto, including negative values. Per-edge padding accepts nonnegative lengths and percentages. Border width is a uniform nonnegative logical length; sizing is fixed to border-box. |
| Flex | Row/column directions and reversal; no wrapping, wrapping, and reverse wrapping; nonnegative logical/percentage row and column gaps; grow/shrink/basis (auto, length, percentage); justify-content, align-items, align-self, and align-content. No order property; explicit baseline alignment waits for M4. |
| Grid | Logical/percentage/auto/fraction/minmax track sizing; explicit start/end lines and spans; sequential row/column auto placement; configurable implicit track sizing; nonnegative logical/percentage gaps; item alignment/overrides and track alignment/distribution. Named lines/areas, subgrid, auto-repeat, and dense placement are deferred; explicit baseline alignment waits for M4. |
| Block | Tree-ordered vertical flow, available-space auto width and content-based auto height under size constraints, horizontal auto-margin centering, standard vertical margin collapsing, and mixed Flex/Grid/Block nesting. No Block gap, floats, or inline formatting. |
| Positioning | Static by default, relative offsets preserving flow space, and absolute placement against the nearest relative/absolute ancestor or logical viewport. Four insets accept auto, logical lengths, and percentages, including negative offsets. Fixed/sticky positioning is deferred. |
| Painting | Transparent-by-default solid background; uniform solid border with zero default width; independent logical circular corner radii, proportionally reduced on overlap; group opacity defaulting to 1; visible overflow by default or two-axis padding-box clipping without scrolling or a new formatting context. No new gradient, shadow, background-image, or dashed-border style APIs. |
| Stacking | Auto by default or a signed integer; applies to relative/absolute nodes and Flex/Grid items. Contexts arise at the root, from eligible non-auto z-index, or from opacity below one. CSS painting phases and tree-order ties apply; contexts are atomic, ordinary parents do not isolate descendants, and stacking does not change layout order or containing blocks. See ADR-0027 for the confirmed rules. |
| Participation | Display none excludes a subtree from layout, painting, and hits. Nodes otherwise participate by default, regardless of transparency, zero opacity, or handler presence. An independent hit-test switch excludes only the node itself; children retain their own participation. |
| Hover update policy | `stationary_hover_updates: bool`, default `false`, not inherited. Any enabled node on the old hover path or new geometric hit path, including ancestors, triggers a complete path update and notifications to all affected nodes. Enabling checks the final styles/geometry after the command batch and necessary geometry preparation; disabling preserves current hover and removes only that node's trigger. Merely declaring a hover style does not enable it. ADR-0030 defines the policy; ADR-0031 defines notification order and batching. |

S2 records defaults, percentage bases, supported intrinsic sizing behavior,
validation, and invalidation for every exposed property. Omit unsupported
variants rather than silently substituting another meaning. CSS vocabulary is
used with documented semantics, without accepting CSS text.

Layout and pointer routing use logical coordinates. Convert physical viewport
size to logical size using the scale factor; apply logical-to-physical scaling
once when producing the Scene. Keep fractional geometry through layout and
painting, with no implicit integer rounding in logical space. The Taffy adapter
checks finite values and numeric range when converting between owned geometry
and its internal precision. Tests state numerical tolerances explicitly.

M2 supports layout translation and root scaling; arbitrary style transforms
remain outside the draft scope. Painting must honor z-index and the selected
CSS stacking-context model without changing the UI tree's structure or layout
order. The confirmed painting phases and context rules are defined in
[ADR-0027](../adr/0027-z-index-stacking-contexts.md).
Hit testing follows the final painting order in reverse, uses rounded border
boxes excluding margins, and respects the same effective clip geometry.
Transparency, zero opacity, and absent handlers do not disable participation;
the per-node switch and display suppression follow the confirmed rules above.
Capture overrides the routed target and hover path according to ADR-0028; it
does not change the result of ordinary geometric hit testing.
Without capture, stationary-pointer hover updates follow ADR-0030. Retained
hover may differ from a new geometric hit when no hover update is triggered.

### Dirty propagation and cache validity

| Change | Required work | Frame behavior |
| --- | --- | --- |
| Setting an equal value | None | No request from this mutation. |
| Background, border color, radii, opacity, or other paint-only value | Validate/update style, then invalidate painting and affected hit geometry; no layout computation | Coalesce one request. |
| Effective z-index change or opacity crossing the context-creation threshold | Invalidate stacking-context membership, painting order, and corresponding hit ordering without changing geometric layout | Coalesce one request. |
| Dimension, padding, border width, layout mode, or other layout input | Update style; invalidate layout caches and paint for affected geometry | Coalesce one request. |
| Child insertion/removal/reorder/reparent | Invalidate affected old/new parent layouts, topology-dependent caches, paint, and hit paths | Coalesce one request. |
| Intrinsic measurement content changes | Invalidate measurement and dependent layout/paint | Coalesce one request. |
| Hit-test participation changes | Update hit eligibility and affected hit paths without geometric layout or painting changes | No automatic visual frame unless event handling changes visible state. |
| Handler or reserved semantic data changes | Update the corresponding data; no layout/paint unless the handler also mutates visible state | No automatic visual frame. |
| System repaint | Reuse valid style/layout and retained painting content; submit a complete Scene | One frame even with a clean tree. |
| Resize or scale change | Recompute logical constraints and physical projection; invalidate layout where constraints changed and painting where scale changed | Preserve M1 resize acknowledgement and zero-size suspension. |

Style dirtiness means properties need validation/projection, not selector
matching. Track pending descendants so locating dirty work does not require
resolving every clean node on each mutation. Layout caches include relevant
constraints, style/content revisions, child order, and measurement inputs;
removal and slot reuse cannot inherit another node's cached result.

A layout boundary stops outward propagation only when its size and other
parent-visible layout contributions are independent of descendants under the
current constraints. Fixed dimensions alone are insufficient if intrinsic
minimums, collapsed margins, or baseline contributions can still change the parent. Use a
conservative boundary classification until that independence is proven. Boundary
subtrees still relayout internally; parent-constraint changes invalidate them
from above. Test both independent and content-sized ancestors.

Complete style/layout/paint preparation may make tree caches clean before
presentation. The scheduler separately retains an outstanding presentation
until `PresentationOutcome::Presented`; `Skipped` must not lose the frame or
its oldest input timestamp. Any mutation after preparation invalidates the
relevant caches again. An idle tree requests no work or polling.

### Pointer dispatch

Event types, propagation, capture scope/lifecycle, captured hover behavior,
`stationary_hover_updates` and its complete-path trigger/notification scope,
default/non-inheritance/runtime switching rules, the callback mutation boundary,
ordering within a hover notification batch, per-command validation rejection
with partial success, queue-tail insertion of callback-generated commands, and
completed-batch result reporting, ADR-0033 lifecycle cleanup, and ADR-0034 input
processing order are confirmed above. Concrete result types and callback
signatures are delegated implementation choices.

- Hit test the latest committed layout. Before a positional event following
  structural or layout changes, bring geometry up to date without presenting
  a frame; cache the result for the next paint. Do not route against stale boxes.
- Follow ADR-0034: prepare geometry and fix the next input's target/route after
  older command work. Deliver its hover batch before the primary event, keeping
  that batch's edits queued. Complete final-up automatic release and hover
  reconciliation before applying the callbacks' commands.
- Deliver move/down/up/cancel to the target and then its structural ancestors,
  with stable target, current-target, logical window/local positions, button
  state, and original timestamp. Per-node hit exclusion does not remove an
  ancestor from this route. A handler may stop bubbling without passing the
  event to obscured nodes. A capture propagation phase is outside M2.
- Follow ADR-0029: fix the route before calling handlers, queue tree/style/capture
  commands, and apply them in submission order after the complete ordinary
  dispatch, including stopped propagation. Reads throughout that dispatch see
  state before command application; stop-propagation control itself is immediate.
  A requested removal cannot destroy a node on the current route mid-dispatch.
  ADR-0031 extends the mutation boundary for a hover notification batch through
  the whole leave/enter sequence.
- Commands generated during command application join the existing queue's tail
  under ADR-0029. Complete the current batch and older queued batches before
  newer ones. Do not recursively apply new commands inside an executing command.
  Required cancel-before-destruction notification still runs while the node is
  live; its callback's mutation commands wait their turn and may then fail
  validation if they refer to the destroyed node.
- Apply ADR-0029's validation policy at each command's turn: check against the
  current state, reject an invalid command without state changes, record its
  identity and failure reason, and continue later commands in submission order.
  Preserve earlier successes; batches permit partial success. Handlers must not
  hold a mutable tree borrow while another callback or a layout pass runs.
- After each mutation command batch completes, deliver its ordered success/
  failure report to the application on the window thread. Include batch and
  command identity, relevant NodeIds, and structured rejection reasons. The
  default handler logs failures and leaves successes silent; custom-handler
  mutations join the queue tail. Concrete types/signatures remain to be settled.
- Track the effective hover path: the capture holder and its structural
  ancestors while captured; otherwise select the geometric hit path when
  reconciling hover. Under ADR-0030, geometry or hit-condition changes with a
  stationary pointer do not trigger hover reconciliation by default. Expose
  the non-inherited `stationary_hover_updates: bool` with default `false`.
  Any enabled node on the old hover path or new geometric hit path, including
  ancestors, triggers a complete reconciliation;
  nodes outside both paths do not qualify. Deliver direct enter/leave to all
  nodes whose membership changes, regardless of their individual option;
  unchanged membership emits no duplicates. Geometry changes alone do not
  retarget captured hover. Do not use retained hover as a cached answer to a
  fresh geometric hit query.
- Follow ADR-0031 when a hover transition is required: fix old/new paths, update
  hover state first, deliver old-only leaves from inner to outer, then new-only
  enters from outer to inner. Common node identities receive neither event.
  All boundary callbacks see the new hover state and unchanged tree/style/
  capture state until the batch finishes; only then apply their queued commands
  in submission order. A removal requested by an earlier callback cannot skip
  a later recipient in this batch.
- Enabling `stationary_hover_updates` checks the final styles and geometry
  after command application and necessary geometry preparation. Only a node on
  either relevant path qualifies. Disabling preserves hover and removes only
  that node's trigger; other enabled nodes, including ancestors, still qualify.
  Do not emit notifications from intermediate option values in the batch.
- Explicit pointer capture follows ADR-0028: one holder, no implicit takeover,
  acquisition only with pressed buttons, final-up delivery before automatic
  release, and explicit early release without synthetic up/cancel. Captured
  delivery continues outside the originating window. Hover follows the capture
  holder until release, then returns to ordinary hit testing. Native drag and
  drop, gestures, and multi-touch remain outside the proposed scope.
- Follow ADR-0033: pointer leave clears hover without capture; window
  deactivation/hiding/destruction or input interruption cancels capture and
  clears hover. Validated removal/suppression sends required cancel with old
  capture/hover readable, publishes cleaned hover, then delivers leaves before
  resource release or suppression. Retain valid ancestor membership unless
  interruption or cancellation outside the window requires an empty path.
  Replacement entry obeys the stationary opt-in with deleted nodes' last values
  and surviving nodes' final batch values. Reparented capture follows the new
  ancestor path unless suppression requires cancellation. Interruption or
  restoration alone creates no new enter. Never dispatch to a stale ID.
- Add the minimal raw lifecycle notifications to the platform interface and
  both backends to distinguish pointer leave from window deactivation, hiding,
  destruction, or interrupted input delivery. They describe window lifecycle,
  not M5 node focus. No lifecycle path synthesizes a click.
- Pointer state is plain window-thread data. Demo handlers may change paint
  properties; reactive state variants and a high-level click/gesture API wait
  for their later milestones. Input with no visible change requests no frame.

Capture-scope evidence: the current
[macOS content view](../../crates/oxiview-platform-macos/src/macos.rs) forwards
mouse-dragged and mouse-up events to the platform event stream and preserves
coordinates outside the content bounds. AppKit documents an originating-view
[down/drag/up dispatch sequence](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/EventArchitecture/EventArchitecture.html).
The [execution record](#execution-record) documents the native right-edge
drag/release observation and the headless lifecycle tests.

## Steps

Implement independently reviewable steps. The owner's preview authorization
permits parallel work across module boundaries, with integrated local
`cargo xtask verify` before delivery or implementation commits. Snapshot acceptance
and performance recording remain explicit human gates at the steps that add
them; planning does not authorize either operation.

### S1 Arena node tree (`oxiview-core`)

- Introduce ADR-0032's opaque, copyable, equality-comparable, hashable UI `NodeId`
  with tree ownership, slot, and generation, backed by a safe arena. Implement
  automatic root creation, stable root identity, clear-contents preservation,
  subtree removal, and whole-tree destruction under the confirmed lifecycle.
  Implement ordered append/insertion and same-tree subtree movement with the
  confirmed post-removal index semantics, identity/state preservation, atomic
  rejection, and successful no-op behavior. Keep existing scheduling working.
- Implement child-list-ordered postorder teardown with an explicit stack,
  releasing each node's owned resources once after its descendants. Apply this
  to subtree removal, clear-contents, and whole-tree destruction; preserve the
  root on clear and release it last on full teardown.
- Create ordinary nodes only as part of mounting them under a specified parent.
  Leave creation timing to callers and expose no live detached-node operation.
  Reject creation atomically; do not allocate live nodes merely because an
  application retains data or an unevaluated creation function. Provide
  identity reservation for actual queued creation, pending-read errors, and
  permanent invalidation after failed creation without exposing live orphans.
  Creation commands own reservation cleanup on failure or destruction of their
  queue before execution; dropping ID copies does not cancel queued creation.
  S6 connects these primitives to event-handler queues and result reports.
- Distinguish Scene identity in core imports and documentation. Add node payload
  and reserved semantics ownership without implementing later step behavior.
- Validate owner/generation, parent links, indices, cycles, root rules, and depth
  before mutation. Reuse existing property-test tooling; any arena dependency
  must be reviewed and hidden behind the owned API.

Acceptance: deterministic tests and generated edit sequences agree with a small
reference model. Cover slot reuse, foreign/stale IDs, rejected-operation
atomicity, subtree destruction, sibling order, and deep-tree rejection. Identity
fixtures must distinguish equal slot/generation numbers from different owners,
reject an old ID after reuse and after owner destruction/recreation, and prove
copy/equality/hash consistency. Exercise counter exhaustion deterministically
without wrapping or reviving an old identity. API checks must keep representation
opaque and UI/Scene identities non-interchangeable. These are identity acceptance
requirements; bit widths and storage choices remain implementation design work.
Root fixtures verify creation of one root, unchanged root ID after clearing and
repopulating, preservation of existing root-owned data, atomic rejection of root
removal/reparenting, descendant invalidation on subtree removal/clear, and root
invalidation on whole-tree destruction. Verify removed node-owned state is
released exactly once. Resource-release traces cover leaf, branching, and deep
trees, including `S -> Q -> R -> P` for P's ordered children Q/R and Q's child S.
Verify parent resources outlive descendant release, current child-list order
after reordering/reparenting, root preservation on clear, and root-last release
on whole-tree destruction. Deep-tree teardown uses an explicit stack within
the 64-level contract. Include rejection of level 65 and of a reparenting that
would push any descendant beyond level 64, with no partial edit. As typed styles
and handlers are introduced in S2/S6, extend clear-content fixtures to cover those concrete root
properties; capture lifecycle integration remains S6 work.
Structural fixtures cover insertion at the start/middle/end, the empty-list
boundary, forward/backward same-parent moves, and cross-parent subtree moves.
Verify all subtree IDs and existing node-owned data survive movement, and
extend preservation checks to typed styles/handlers as they are introduced.
Test destination-index bounds after conceptual removal, invalid owner/index/
cycle requests leaving both trees unchanged, and original-position moves
returning success without new invalidation or losing existing dirty work.
Later layout integration verifies the moved subtree in its new parent context.
Creation fixtures verify parent/child agreement and root reachability on
success. Generated edit sequences check that the root is the only parentless
live node after every operation. Rejected parent/index requests leave live-node
count and structure unchanged; no successful move exposes a detached
intermediate state. Document that applications can keep data or creation
functions before requesting nodes without introducing a component Builder or
virtual-list implementation. Reservation fixtures verify no live-node/child-list
changes, explicit pending-read errors, successful creation using the reserved
ID, and permanent rejection after failed creation and slot reuse. Verify
owner checks, checked exhaustion, and invalidation on whole-tree destruction
for reservations too. Verify dropping caller-held ID copies does not prevent
queued creation, and success retains the same identity and transfers node-owned
data without premature release. Failure and queue destruction before execution
release owned resources exactly once, including when failure is followed by
queue teardown; retained IDs remain invalid after storage reuse. S6 covers
deferred handlers, absent notifications for never-created nodes, and reports
end to end.
No layout/style crate or new renderer dependency is needed for this step.

### S2 Typed styles and invalidation (`oxiview-style`, `oxiview-core`)

- Create the registered crate with
  `cargo xtask new-crate oxiview-style --context styling`, following the
  [scaffold skill](../../.agents/skills/scaffold/SKILL.md).
- Define the property subset, documented defaults/units, validated setters, and
  an explicit property-to-invalidation table. Add its inherited workspace
  dependency only where needed.
- Include ADR-0030's `stationary_hover_updates: bool`, default `false`, without
  property inheritance. Defining a hover style must not implicitly enable it;
  state-variant APIs remain M5. S6 applies its runtime update policy.
- Add separate dirty flags, descendant work tracking, revision information, and
  mutation batching. Provide the boundary-classification hook; S3/S4 validate
  which layouts can safely stop propagation.

Acceptance: table-driven tests cover every property category, equal-value
updates, invalid numeric values, insertion/removal/reparenting, and coalescing.
Paint-only changes never mark layout dirty. Rejected changes preserve both
values and dirty state. Existing M1 scheduler tests remain green.

### S3 Owned layout protocol and test implementation (`oxiview-layout`, `oxiview-core`)

- Create the registered crate with
  `cargo xtask new-crate oxiview-layout --context layout`.
- Define owned constraints, available-space modes, known dimensions, local box
  results, intrinsic measurement, cache invalidation, and boundary reporting.
  Define traversal using an associated caller-owned identity, with no core or
  Taffy types in signatures.
- Project style into protocol inputs in core. Implement the protocol over the
  core arena without copying its parent/child graph.
- Add a deterministic test implementation with known sizing/positioning rules,
  counting measurement calls and visited nodes. Unsupported layout modes in
  that implementation return explicit errors.
- Specify failure handling for invalid constraints/measure results and prohibit
  reentrant structural mutations during layout. Keep committed geometry intact
  on failure and report the error through the application boundary.

Acceptance: run protocol contract tests with custom intrinsic-measure leaves,
finite and intrinsic constraints, changed constraints, unchanged reuse, and
independent/content-sized ancestors. Core runs without selecting Taffy. The
test implementation verifies the boundary rather than copying Taffy's results.

### S4 Taffy adapter and incremental layout (`oxiview-layout`, `oxiview-core`)

- Review Taffy's selected version, license, maintenance, Rust 1.97 compatibility,
  features, and resolved dependency footprint under the
  [dependency conventions](../conventions/dependencies.md). Pin it at the
  workspace root and inherit it only in layout.
- Implement the private low-level adapter for the specified Flex, Grid, Block,
  absolute positioning, hidden subtree, and intrinsic leaf cases. Disable
  unneeded parser and tree-storage features after checking the selected release.
- Connect cache invalidation and conservative boundary detection. Reclaim
  adapter state on node removal; use checked numeric conversion and the depth
  guard from S1. Do not add unsafe code for tree access.

Acceptance: hand-calculated fixtures cover nested Flex/Grid/Block, wrapping,
gaps, percentage sizing, constraints, absolute positioning, display suppression,
and custom measurement. Positioning fixtures cover ignored static insets,
relative offsets preserving sibling placement, absolute positioning through
static ancestors, logical-viewport fallback, and negative/percentage insets.
Block fixtures cover sibling, parent/child, and
empty-block margin collapsing, negative margins, and the applicable
padding/border conditions that prevent collapsing.
Repeated clean layout performs no new measurement;
paint-only changes do not invoke layout. A contained descendant change leaves
unrelated branch measurements unchanged; an intrinsic size change reaches the
required ancestors. Generated edits produce equivalent geometry to a fresh
uncached layout. Both protocol implementations pass shared contract tests.

### S5 Tree painting and frame integration (`oxiview-core`, `oxiview-app`)

- Connect tree preparation and painting to `Application` and the existing
  window loop. Wire the selected layout implementation in the composition root.
- Emit Scene content for backgrounds, borders, custom painting, opacity, and
  clips in the confirmed z-index order. Reuse clean node painting content while
  submitting complete frames.
  Handle removal/reparenting without orphaned Scene references.
- Apply viewport scaling exactly once. Preserve resize tokens, zero-size
  suspension, system redraws, skipped-frame retries, and input attribution.
- Add CPU integration snapshots in `oxiview-app`, which can depend on both core
  and rendering adapters. Keep `oxiview-testing` a dev-only image harness.

Acceptance: snapshots cover nested layout, sibling overlap, border/radius/clip
edges, opacity, and scales 1, 1.5, and 2. Include oversized-radius reduction,
rounded padding-box clipping with visible borders, and overlapping descendants
under group opacity. Z-index updates must change the expected overlap while
preserving layout geometry and avoiding layout computation. Stacking fixtures
must distinguish an ordinary wrapper from a wrapper that establishes a stacking
context, including descendants competing with content outside that wrapper.
Cover auto versus explicit zero, negative and positive levels, painting phases,
tree-order ties, static Flex/Grid items, and ineffective z-index on an ordinary
static node. For a node with no other context trigger, changing opacity between
one and a lower value must remove/create its context without geometric layout
work; background alpha alone must not create one. Layout fixtures also verify
that context creation alone does not change
an absolute descendant's containing block.
Layout fixtures verify that clip alone does not prevent Block margin collapsing.
Inspect actual/diff images and obtain
human acceptance before adding baselines. Headless tests prove one presentation
for a batch of visible changes, zero frames over 1,000 idle intervals, repaint
with clean layout, and retry after a skipped presentation. A failed preparation
cannot publish partially updated geometry or a partial Scene.

### S6 Hit testing and pointer routing (`oxiview-core`, platform port/backends)

- Add the minimal raw lifecycle events to macOS and headless, with documented
  ordering relative to pointer input and destruction.
- Implement the confirmed hit rules over committed rounded border boxes,
  final painting order, effective clips, display suppression, and independent
  per-node participation. Keep hit candidates for transparent nodes whose
  Scene content is omitted. Specify edge inclusion consistently for adjacent
  boxes; use half-open rectangular bounds before rounded-edge and clip checks.
- Implement the confirmed event types and target/bubble/direct propagation
  rules and ADR-0028 capture scope/lifecycle, including delivery outside the
  originating window and capture-based hover. Implement ADR-0029's deferred
  tree/style/capture commands, consistent reads during dispatch, and stable
  routes. Apply ADR-0030's `stationary_hover_updates` policy: an enabled node on
  either the old or new path, including ancestors, triggers complete-path
  updates and notifications to all affected nodes. Enabling checks final state
  after the batch and necessary geometry preparation; disabling preserves hover
  and removes only that node's trigger. Apply ADR-0031's state update before
  ordered leave/enter delivery and defer callback mutations through the complete
  notification batch. Validate commands against the state at their turn, reject
  individual invalid commands with identifiable errors, continue later commands,
  and preserve prior successes. Append callback-generated command batches to
  the existing queue's tail, preserving older work without recursive command
  application. Deliver ordered batch reports after completion on the window
  thread, with identifiable results and a failure-logging default handler;
  mutations from the application result callback join the queue tail. Apply
  ADR-0034's input order, including fixed targeting through initial hover and
  primary dispatch, automatic release before up-callback commands, mandatory
  notifications within their commands, and report-before-stationary-check batch
  completion. Concrete signatures are delegated engineering choices. Apply
  ADR-0033's confirmed cleanup staging.
- Preserve capture timestamps through routing. A handler that changes visible
  state invalidates the scheduler; a no-op handler does not.

Acceptance: deterministic tests cover overlap in final stacking order, border
inclusion, margin exclusion, clips and rounded corners, a visible-overflow child
outside its parent's box, and exclusion by an effective ancestor clip. Verify
that transparent and zero-opacity nodes still intercept hits, that removing a
handler does not change eligibility, that disabling only a parent's own hit
participation preserves child hits, and that display none excludes the entire
subtree. Event traces must show move/down/up/cancel reaching the target and
structural ancestors in order, unchanged by z-index; a hit-excluded ancestor
still receives bubbled events. Stopping propagation must skip remaining
ancestors without dispatching to nodes underneath. Enter/leave notifications
must be direct rather than bubbled. Verify stable target identity, changing
current-target and logical local coordinates, and unchanged window coordinates,
button state, and original timestamp along the route.

Also cover fractional scales, hover paths, capture/release, removal and slot
reuse, deactivation, leave/cancel, and layout changes immediately before input.
Headless sequences must cover press inside, capture, leave the node/window,
movement outside, and release outside, plus leaving and returning before
release. Capture must retain its target across boundary crossings, without a
spurious cancel or clamping out-of-bounds logical positions. Distinguish normal
window leave from an interruption that ends delivery under ADR-0028.
Capture lifecycle tests cover requests without pressed buttons, stale/foreign
or display-suppressed targets, repeated requests by the holder, and competing
requests that must preserve existing ownership. Multi-button sequences must
retain capture until the final up is delivered. Explicit early release must
restore hit-based routing without synthetic up/cancel; subsequent handoff is
possible only after release.

Exercise each cancellation trigger, including ancestor removal or display
suppression, and overlapping interruption notifications. Verify one cancel per
active capture, notification before destruction, no stale dispatch, and cleanup
even when a handler stops cancel bubbling. Opacity changes and disabling the
holder's own hit-test participation must preserve capture.

ADR-0033 lifecycle traces cover valid and rejected removal/suppression, hovered
ancestor removal, clear-contents, each interruption kind, and repeated
notifications. Verify cancel callbacks read old capture/hover; leave callbacks
read published cleaned hover and cleared capture where applicable. All required
notifications precede resource release or suppression. Preserve valid ancestor
membership unless interruption or cancellation outside the window clears the
whole path. Rejected requests emit no cleanup events. Callback edits cannot
run before the current cleanup finishes or rescue a node ahead of its removal.
Interruption and restoration alone generate no new enter; the next valid
pointer input rebuilds hover. Reparented capture retains its holder and updates
ancestor membership without duplicating shared-node notifications; moving into
hidden ancestry cancels before completing the move.

Root/clear-content integration fixtures preserve the root's styles and handlers
while removing all descendants. A captured descendant must receive cancel
before destruction; a capture held by the surviving root must remain active
when clearing descendants alone. Rejected root removal/reparenting must not
cancel capture or partially modify the tree. Full tree destruction invalidates
every ID, including the root, after required capture cleanup.

Captured-hover fixtures cover P retaining hover while the pointer is over
nonoverlapping sibling Q or outside the window, with no geometry-only leave
from P or enter to Q. Ordinary geometric queries must still return their actual
result, independently of the capture override. Include a moving holder,
overlap/z-index changes, and a holder excluded from ordinary hit testing.
Acquisition by a different eligible node must reconcile the hover path when
no current holder blocks the request. Releasing over Q must deliver the final
up to P before ordinary hover resumes on Q; early release also restores the
geometric hover path, and release outside clears it. Cancellation/removal must
leave no stale captured hover path. Each hover batch follows ADR-0031; ordering
within lifecycle cleanup follows ADR-0033; global input/command scheduling
follows ADR-0034.

Stationary-pointer fixtures with live nodes and no enabled option on either
relevant path must show no automatic hover transitions or enter/leave solely
from position, size, stacking, clip, or hit-participation changes. Fresh geometric
hit queries must still reflect the updated state. Cover P and Q with only P,
only Q, both, or neither enabled: any enabled node on the old/new path triggers
the complete switch, including notifications to disabled nodes. Include an
enabled ancestor with disabled descendants, the root, an enabled node outside
both paths, and cases with one empty path. Shared or unchanged path membership
must not produce duplicate notifications. Verify that capture acquisition/
release reconciliation and capture cancellation remain independent of this
option. Removal/display-suppression fixtures must show required leave even with
no opt-in, with no automatic replacement enter in that case. If either old/new
path opts in, reconcile after batch completion and geometry preparation without
repeating cleanup leaves. Deleted old-path nodes use their last option values;
surviving nodes use final batch values, including an ancestor enabled and then
disabled before completion. Never deliver to destroyed nodes.

Runtime-option fixtures verify default `false`, no property inheritance, and
ancestor participation even when a descendant reads its own value as `false`.
Enabling on either relevant path must check final styles/geometry after the
batch, with no duplicate notifications for unchanged paths; enabling an
unrelated node must not trigger a hover switch. Disabling preserves hover,
while other enabled nodes retain their effect. Include repeated toggles in one
batch and geometry edits in that same batch: no intermediate toggle may emit
its own boundary notifications. Switching the option during capture must
preserve the holder's effective hover path.

Hover notification batch traces must show the new effective hover state before
the first leave handler and throughout all callbacks, old-only leaves in
inner-to-outer order, then new-only enters in outer-to-inner order. Shared node
identities receive neither event, including a common node whose ancestry has
changed. Include empty/identical paths. Assert unchanged tree/style/capture
reads through the full batch and ordered application of its queued commands
after the last notification. In particular, P.leave requesting removal of Q
must still be followed by Q.enter before Q is destroyed. Ordinary move/down/up/
cancel dispatch must retain its own mutation boundary. Test required capture
cleanup and hover leaves before destruction in ADR-0033's confirmed order.

Mutation fixtures must show that removal or reparenting requested by a handler
leaves the current target and ancestor route intact through the end of dispatch.
Reads by that handler and later ancestors must observe the pre-application
tree, style, and capture state. Commands apply afterward in submission order;
stopping bubbling skips remaining handlers but still applies submitted commands.
Cover deferred capture acquisition/release and self-removal while captured:
the original event finishes before removal, and cancel reaches the live holder
before destruction. Verify no stale delivery or borrow panic.

ADR-0034 input-order traces must show older queued layout edits affect the next
input's hit, geometry is current without immediate presentation, and initial
hover callbacks cannot retarget the associated primary event. Q.enter enqueueing
Q's deletion must still deliver Q.down before deletion, with fixed route and
coordinates. Final up with queued self-removal must release capture and
reconcile hover first, without a later synthetic cancel; non-final up retains
capture. Mandatory capture/reparenting/cleanup notifications finish within
their commands. Verify each completed-batch report precedes its final-state
stationary-hover check, while all generated commands preserve FIFO tail order.

Command validation fixtures cover rejection at the first, middle, and last
positions in a batch. Earlier successes must remain applied, later independent
commands must still run, and every failure must identify its command and reason.
Verify application-time validation with delete-Q, set-Q-width, set-R-color:
the first and third succeed, the second is rejected, and Q is not restored.
Cover dependent commands becoming invalid after earlier successes and rejected
commands leaving tree, style, capture, and invalidation state unchanged. Apply
the same policy to batches from ordinary handlers and hover notifications.

Queued-creation fixtures reserve P and Q in one callback, enqueue P under the
root and Q under P, then queue a style edit for Q. Verify callback reads report
"not yet created" while the live tree remains unchanged, then both nodes mount
with their original reserved IDs before the style edit applies. A rejected P
creation must invalidate P, reject and invalidate Q, reject Q's later style edit,
and still apply an independent valid edit to R. Reports identify every outcome
and the reserved IDs even when creation failed; no stale storage is accessed.
Verify no enter, leave, or cancel is delivered to a never-created node when
creation fails or its unexecuted command is destroyed with the queue. This
does not suppress required lifecycle notifications for existing live nodes.

Queue-order fixtures must show C1 deleting captured P, cancel running while P
is live and enqueueing C3, P being destroyed, C2 completing, and then C3 being
applied. Include C3 targeting P and being rejected after destruction. Also cover
an older batch already waiting behind the current batch: it must remain ahead
of newly generated commands. Further commands generated while later batches
run must follow the same tail-insertion rule, without recursive command drain.
Verify immediate application-local cleanup without immediate retained-state
mutation.

Report fixtures must cover all-success, all-failure, and mixed batches, retaining
submission order and batch/command identity, relevant NodeIds, and structured
reasons. The application receives the report only after all commands in that
batch finish, on the window thread. Report-handler mutations must join the tail
behind already queued work and appear in a later batch's results. Verify the
default handler logs failures with identifying context, stays silent for
successes, and permits failure reporting for destroyed/stale nodes without
accessing their storage. Result handling with no visible mutation must not
request a frame. ADR-0034 defines input/hover placement; its traces and the
delegated work-budget and API contracts need matching implementation fixtures.
Headless input-to-CPU-pixel tests show the selected target's visible response on
the next virtual frame and no redraw for no-op input. Both backends cover the
new lifecycle contract; native interaction is also checked in S7.

### S7 Demo, observability, and measurements (`window`, core/layout benches, `xtask`)

- Add a `--layout-demo` mode to the existing `window` example, including a
  headless version. Show nested Flex/Grid/Block regions, a clipped overlap,
  pointer hover/press feedback, a captured drag that continues outside the
  window, and an interaction that changes layout or adds/removes a node. Use
  colored geometry without introducing text or a widget library.
- Keep the M1 default and measurement scene intact. Reject combinations of
  `--layout-demo` with the existing native measurement/fullscreen diagnostic
  modes rather than silently changing their workload or JSON contract.
- Add `tracing` spans for style, layout, painting, hit testing, and event
  dispatch, plus counters for resolved styles, measured/laid-out nodes, cache
  reuse, painted nodes, and dispatched handlers. Counters are available to
  deterministic tests without building an Inspector UI.
- Add Divan targets in root `benches/`, owned by core/layout as appropriate.
  Measure arena edits, initial layout at 100/1,000/10,000 nodes, unchanged
  preparation, a contained layout change, a paint-only update, and hit testing.
  Fix tree shapes and inputs; keep unrelated setup outside the timed region.
- Integrate new metrics with the existing parser and report. New metric names
  without baselines fail comparison; they are never reported as passing or
  silently skipped. Review candidate measurements before requesting recording.

Commands introduced by this step (not available at plan creation):

```sh
cargo run --release -p window -- --layout-demo
cargo run --release -p window -- --layout-demo --headless
```

Acceptance: human inspection confirms layout during resize, pointer targeting,
clipping, hover/press feedback, drag cancellation, and fullscreen restoration.
Native capture acceptance includes dragging outside each window edge, releasing
outside, and leaving then returning before release. Record the delivered
movement/up sequence and verify that normal boundary crossings keep capture;
exercise interruption cleanup under the confirmed ADR-0028 cancellation policy.
Verify captured hover stays with the holder across those crossings and resumes
ordinary hit-based hover on release.
Deterministic counters enforce zero layout work for paint-only updates, no
remeasurement of clean unrelated branches in boundary fixtures, and zero idle
frames. New timing metrics use the existing baseline protocol rather than
invented absolute budgets. After explicit human approval, record the new metric
set and immediately compare it; review changes to existing metrics as well.
M1 native latency/idle gates remain enforced against their defined workload;
they do not constitute a measurement of M2 demo latency. The M7 scrolling metric
remains pending.

### S8 Close-out

- Update this plan's execution record, the roadmap, README, and canonical agent
  instructions with actual delivery and validation status. Do not update
  translations unless explicitly requested.
- Add dated execution notes to M2's accepted ADRs identifying the implemented
  subset and what remains deferred, without rewriting their historical intent.
- Document the node mutation/measurement/painting/event contracts and remaining
  limitations for M3. Record any unresolved performance or native observations
  with their actual results.

Acceptance: a fresh local checkout passes `cargo xtask verify`; this Mac passes
`cargo xtask verify --full` with reviewed snapshots, human-approved performance
baselines, and the native visual checks recorded. Do not mark M2 done while an
included feature, new metric, or required acceptance remains pending.

## Verification and approval gates

All verification runs locally. Follow the
[verify skill](../../.agents/skills/verify/SKILL.md),
[snapshot conventions](../conventions/testing-snapshots.md), and
[performance protocol](../conventions/testing-performance.md).

| Gate | Evidence | First step |
| --- | --- | --- |
| Tree correctness and ownership | Reference-model/property tests, invalid-edit atomicity, stale/foreign IDs, bounded depth and destruction | S1 |
| Precise invalidation | Property classification and dirty-propagation tests | S2 |
| Replaceable layout boundary | Shared protocol contract tests with independent test implementation | S3 |
| Layout correctness and cache reuse | Numerical fixtures, cached-versus-fresh equivalence, measurement counters | S4 |
| Complete painting and scheduling | Human-reviewed CPU snapshots, headless pixel tests, resize/retry/idle tests | S5 |
| Pointer semantics | Routing traces, lifecycle tests on both backends, headless input-to-pixel tests | S6 |
| Native behavior and timing | Human demo review, recorded metric set, successful immediate comparison | S7 |
| Milestone completion | Fresh-checkout ordinary verify and local full verify, execution record | S8 |

Ordinary `verify` checks deterministic behavior and snapshot comparisons; it does
not open the demo for human acceptance or measure benchmark timing. Full verify
adds benchmarks and M1 native measurements under their documented display/power
conditions. Neither command accepts images or records baselines automatically.
Review `target/xtask/report.json`, individual logs, and generated artifacts;
report environment errors and pending work distinctly from passes.

## Risks and responses

| Risk | Response and evidence |
| --- | --- |
| Single-tree integration exposes Taffy borrowing or infallible callback constraints | Prove traversal/measurement and error propagation in S3/S4 with the owned protocol and safe adapter. Do not bypass the boundary with Taffy types in core or add unsafe access. |
| Boundary detection or caches hide required ancestor layout | Begin conservatively; compare incremental results against fresh layout after generated mutations, including percentage and intrinsic constraints. |
| UI identity, Scene identity, and reused slots are confused | Apply ADR-0032's distinct types, tree ownership, slot generations, and checked exhaustion. Test removal/reuse, foreign owners, and destroyed/recreated owners across painting, hit paths, capture, caches, and diagnostics. |
| Geometric hit results differ from painted geometry | Share committed layout, clip policy, and ordering; test fractional scale and rounded clip edges end to end. Verify capture's intentional target/hover override separately from geometric hits. |
| Callback mutation or hover recomputation causes reentrancy | Stable routes, queued commands, geometry revisions, and tests for handlers that alter or remove the target. Bound feedback processing and report non-settling interaction rather than looping indefinitely. |
| A large synchronous layout stalls a resize transaction | Trace preparation separately; retain the existing bounded platform wait and timeout diagnostics. Use depth guards and fixed benchmark sizes to expose pathological inputs. |
| M2 changes invalidate historical performance comparisons | Retain the M1 fixture and units, add distinct M2 metric names, and review all baseline updates. A new metric remains missing until human-approved recording. |

## Definition of done

1. One arena UI tree owns node structure. Callers can defer node creation, and
   successful creation mounts every ordinary node under a parent; all live
   nodes are reachable from the root. Stale/foreign IDs and invalid edits are
   rejected without corrupting live state.
2. Typed properties and three dirty-flag levels drive correct incremental work;
   paint-only changes do no layout and idle state produces no frames.
3. Taffy and an independent test implementation exercise the owned layout
   protocol. The documented layout subset, custom leaf measurement, cache
   invalidation, and conservative layout boundaries pass their tests.
4. Nodes paint through the existing Scene and both presenters with consistent
   units, z-index ordering, clips, resize behavior, and skipped-frame recovery.
5. Pointer routing, hover, capture including outside-window movement/release,
   queued mutation, and cancellation work on headless and macOS without stale
   targeting or unwanted redraws.
6. The M2 demo is runnable, visually accepted, and backed by deterministic tests,
   approved snapshots, trace/counter evidence, and compared performance metrics.
7. Ordinary and full local verification pass, and deferred milestone features
   remain explicitly outside the completion claim.

## Execution record

| Step | Status | Delivery and validation |
| --- | --- | --- |
| Planning | Approved for implementation | Started on 2026-11-17. Groups 1-2 are recorded in ADR-0033/0034; the owner approved groups 3-12 together and delegated concrete engineering choices for a native preview. This approval does not complete milestone acceptance. |
| S1 | Implemented and tested | Generated edits are compared with an independent tree after every operation. Fixtures cover identity exhaustion, reservation cleanup, rejected edits and ordered destruction through level 64. [Tree contracts](../../crates/oxiview-core/src/ui/tree_contract_tests.rs) |
| S2 | Implemented and tested | Typed properties distinguish layout, paint and input changes. A mutation journal coalesces layout edits. Tests check equal and rejected writes, clean siblings, and style projection counts. [Invalidation tests](../../crates/oxiview-core/src/ui/layout_tests.rs) |
| S3 | Implemented and tested | Taffy and a deterministic column engine pass public layout contracts for finite and intrinsic constraints, cache reuse, ancestor sizing and failure recovery. Engines can report which nodes are layout boundaries under their last committed constraints. [Shared contracts](../../crates/oxiview-core/tests/m2_layout_contract.rs) |
| S4 | Implemented and measured | The default adapter retains converted inputs and commits changed geometry after successful layout. Incremental edits, moves, hidden branches and viewport changes are compared against fresh layout. The M2 layout timings passed baseline comparison. [Adapter fixtures](../../crates/oxiview-layout/src/tests.rs) |
| S5 | Implemented, snapshots accepted | Clean nodes retain prepared painting content. CPU pixel tests exercise stacking, group opacity and rounded clips at 1×, 1.5× and 2×. Offscreen Metal tests check asymmetric corners against CPU pixels and geometric hits. [CPU fixtures](../../crates/oxiview-app/tests/ui_paint.rs) · [GPU fixtures](../../crates/oxiview-app/tests/m2_gpu_geometry.rs) |
| S6 | Native review passed | The owner confirmed native Layout Lab acceptance. Deterministic traces cover release outside and re-entry before release at all four edges, hover and cancellation. A headless window presents the hit node's new color at the next frame and stays idle for 1,000 intervals. [Capture traces](../../crates/oxiview-core/tests/m2_capture_matrix.rs) · [Window-loop pixels](../../crates/oxiview-app/tests/headless.rs) |
| S7 | Demo and benchmarks accepted | Layout Lab exposes native input tracing. All eight Divan measurements passed immediate comparison after recording. They cover arena edits, first layout at three sizes, clean preparation, a local layout change, a paint-only update and hit testing. [Tree workloads](../../benches/ui_tree.rs) · [Initial layout](../../benches/layout.rs) |
| S8 | Complete | Fresh-checkout verification passed all seven checks. Local full verification passed all eight, including the M2 performance comparisons and the M1 native checks. The owner's manual acceptance closed M2 on November 20. |

### Native preview execution note (2026-11-17)

The owner requested the first runnable native preview rather than further
per-detail planning rounds. Parallel work covers typed styles/layout, the core
tree and interactions, native/headless lifecycle input, and example composition.
The preview is runnable with `cargo run --release -p window -- --layout-demo`.
The original M1 demo and measurement modes retain their separate workload.
No full M2 completion is claimed here.

Early platform-only checks passed: 15 headless tests, 7 macOS adapter tests,
and Clippy across the three platform crates. The new headless fixtures cover
lifecycle-event FIFO/timestamps and rejection after destruction. Integration
subsequently passed `cargo xtask verify` with all seven checks and exit code 0.
Core coverage includes 19 unit tests and 4 public input-contract tests; the
example's interaction test checks node creation/removal, stale identity, wrap,
dimensions, Grid placement, drag capture/release, stacking, and unchanged-input
redraw suppression.

`cargo run -p window -- --layout-demo --headless` passed the same interaction
exercise and produced `target/xtask/m2-preview/initial.png`, `interacted.png`,
and `resized.png`. All three were visually inspected. A release native window
was also inspected: ADD/REMOVE changed live nodes, RESIZE changed tile layout,
LAYER changed overlap, and dragging the orange tile beyond the window's right
edge still delivered release and cleared capture. RESET restored its position.
Headless rendering covers a resized viewport; the full native window-resize,
fullscreen, interruption, and minimize/restore acceptance matrix is not yet
complete. Preview images are not approved snapshot baselines.

The first preview supplied its event handler to `UiTree::dispatch`. Subsequent
M3 integration added node-owned handlers, per-batch report callbacks, scoped
automatic bindings, and persistent Taffy caches with conservative ancestor
invalidation. Unchanged sibling measurements are reused across dirty passes;
failed passes preserve prior geometry/caches. Painting still rebuilds the Scene
when dirty while reusing retained content and cached text; finer per-node paint
reuse and complete performance acceptance remain pending.
The implementation caps one dispatch at 4,096 applied commands and
16,384 emitted events, reports budget exhaustion, and cancels remaining queued
work while preserving completed outcomes. Shutdown explicitly cancels queued
work and automatic bindings; full M2 performance gates remain pending.

The preview uses a shared maximum layout depth of 64 levels including the root.
The layout workstream selected this after a 256-level Taffy stress case
overflowed the default test-thread stack; Flex, Grid, and Block chains at
64 levels passed its targeted stress test. Core insertion/reparenting enforces
the same limit before mutation; its rejection and iterative teardown tests
passed. Snapshot acceptance, performance baseline recording, and full milestone
acceptance remain pending.

### Integration check (2026-11-18)

Snake and the continuous animation demo exercise this tree in two native
applications. The full workspace check passed after the drawable scheduling
change. It compared the recorded M1 metrics. M2-specific layout benchmarks
and snapshot acceptance remain open. [Integration demo](../../examples/snake/README.md) · [Frame pacing evidence](../adr/0036-fifo-drawable-readiness.md)

### M2 acceptance run (2026-11-20)

After three edits to the same leaf in a 204-node fixture, the layout pass read
one changed input, projected one style and skipped the unchanged sibling's
200 descendants. Tree diagnostics also count intrinsic measurements, layout
cache hits, node painting preparations and node-owned pointer callbacks.
Failed layout passes retain the previous geometry and pending edits for retry.
[Incremental fixtures](../../crates/oxiview-core/src/ui/layout_tests.rs) ·
[Diagnostic contracts](../../crates/oxiview-core/tests/ui_diagnostics.rs)

A color change in a 1,025-node tree visited one node and updated the retained
Scene without a full assembly or a layout pass. Background and border colors
and equal-length custom draw lists use this path. Earlier frame snapshots
keep their pixels when a later edit succeeds or fails. [Paint locality tests](../../crates/oxiview-core/tests/ui_diagnostics.rs) · [Pixel and retry tests](../../crates/oxiview-app/tests/ui_paint.rs)

The layout engine can relayout a fixed-size absolute branch positioned by
the viewport when a descendant changes under the same constraints. Intrinsic
sizes and overflow from a positioned ancestor still propagate. The geometry
fixtures also cover negative and collapsed block margins, padding and border
boundaries, and static and relative insets. [Geometry and containment](../../crates/oxiview-core/tests/m2_layout_boundaries.rs)

The owner reviewed and accepted all three CPU snapshots on November 20.
[1× baseline](../../crates/oxiview-app/tests/snapshots/m2-ui-tree-1x.png) ·
[1.5× baseline](../../crates/oxiview-app/tests/snapshots/m2-ui-tree-1_5x.png) ·
[2× baseline](../../crates/oxiview-app/tests/snapshots/m2-ui-tree-2x.png)

Ordinary local verification passed all seven checks after snapshot acceptance.
A fresh local clone with the same source changes and no copied build output
also passed. The M2-only benchmark runs three rounds of all eight CPU
measurements without opening a window. The owner approved recording the eight
new metrics on the Mac14,10. The scoped recording preserved all nine existing
baseline entries. Immediate comparison passed all eight M2 metrics.
[Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json) · [Workloads and comparison](../conventions/testing-performance.md#m2-retained-tree-and-layout-benchmarks)

| M2 workload | Recorded | Immediate comparison |
| --- | ---: | ---: |
| Initial layout, 100 nodes | 0.1931 ms | 0.1899 ms |
| Initial layout, 1,000 nodes | 2.045 ms | 2.044 ms |
| Initial layout, 10,000 nodes | 26.95 ms | 27.17 ms |
| Arena edits | 841.3 ns | 874.7 ns |
| Clean preparation | 12.71 ns | 12.71 ns |
| Contained layout change | 43.18 µs | 45.79 µs |
| Paint-only update | 226.7 ns | 226.3 ns |
| Hit testing | 48.55 µs | 48.09 µs |

The first full verification and its benchmark retry completed the Divan
rounds. Both stopped during the second M1 latency window while waiting for
the submitted frame matching a synthetic AppKit input. The first native
round in each attempt reported an 8.33 ms refresh interval. The full native
comparison requires a quiet desktop and actual Metal presentation feedback.

Once the owner reserved the desktop, `cargo xtask verify --full` passed all
eight checks in one run. The three native latency rounds measured 3.882 ms
input-to-submit p95 and 25.736 ms input-to-pixel p95 (median of the round
p95s) on the 120 Hz display. CPU usage was 0.0121% over 30.00 seconds idle,
with zero submissions, actual presentations and display-link events. The
same run passed all eight M2 performance comparisons. The 100,000-row
scrolling metric belongs to M7.

On 2026-11-20 the owner confirmed that the native Layout Lab passed manual
acceptance. The checklist covers captured drags across window edges, release
outside, return before release, interruption, live resize, fullscreen, and
restoration after minimizing. [Native review procedure](../../examples/window/README.md#native-capture-review)
