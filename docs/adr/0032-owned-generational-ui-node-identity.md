---
status: accepted
---

# UI nodes: identity, lifecycle, and structural edits

Deferred commands, capture state, and diagnostic reports retain node identities
while nodes can be removed and arena slots reused. UI node identity must detect
stale access and use with the wrong tree, while remaining independent of Scene
identity. A stable root also lets applications clear and repopulate contents
without replacing their root handle. The owner confirmed identity, root/subtree
lifecycle, deferred creation with mounting on successful execution, `NodeId`
reservation when actual creation is enqueued with command-owned reservation
cleanup, ordered insertion/reparenting, and child-list-ordered postorder
resource release using an explicit stack during M2 planning on 2026-11-17.
The preview implements generational IDs, reservation, structural validation and
postorder teardown. Regression tests exercise stale and foreign IDs, failed
reserved creation and rejection of invalid reparenting. [Core tests](../../crates/oxiview-core/src/ui/tests.rs)

This refines identity for the arena tree in
[ADR-0007](0007-single-arena-node-tree.md) and supports the deferred commands and
reports in [ADR-0029](0029-deferred-event-handler-mutations.md). It leaves the
existing retained-tree architecture and crate dependency boundaries intact.

## Decision

- UI `NodeId` is an opaque framework-owned value that supports copying,
  equality comparison, and hashing. Callers do not access its representation
  fields or depend on a third-party arena key type.
- Identity distinguishes the owning tree, storage slot, and slot generation.
  Equal slot/generation values in two trees do not identify the same node.
- Deletion permanently invalidates the removed node's ID. Reusing a slot
  cannot make the old ID identify its replacement. Destroying and recreating
  a tree cannot revive IDs from the destroyed tree either.
- Stale and foreign-tree access return explicit errors. Identity counters must
  report exhaustion instead of wrapping or reusing an identity in a way that
  could make an old ID valid again.
- UI `NodeId` and Scene node identity are distinct types. Do not interchange
  them or use a numeric cast as a mapping between the two domains. Painting
  maintains any required relationship explicitly.

An ID can be retained in a command report after its node is deleted. That
value does not keep the node alive and can still be compared, hashed, and
reported; an operation attempting to access the removed node fails explicitly.

## Root and subtree lifetimes

- Creating a window's UI tree automatically creates its unique root node.
  The root's ID remains unchanged until that tree is destroyed.
- Removing an ordinary node removes its complete subtree and invalidates every
  ID in that subtree.
- Clearing contents removes all root descendants while preserving the root's
  ID, its own styles, and its handlers. Applications can add new contents using
  the same root ID afterward.
- Removing or reparenting the root returns an explicit error without changing
  the tree.
- Destroying the entire tree invalidates every node ID, including the root.
- If removal affects the capture holder, deliver cancel before destruction
  under [ADR-0028](0028-pointer-capture-ownership-and-lifecycle.md). Clearing
  descendants preserves a capture held by the surviving root unless another
  confirmed cancellation condition applies.
- Release node-owned resources in child-list-ordered postorder. Finish each
  child's subtree in the current child-list order, then release the parent's
  own resources. Each node's owned resources are released exactly once.
- Framework subtree teardown uses an explicit stack, without relying on
  recursive destruction of child nodes.

For P with children Q/R in that order and Q with child S, resource release is
`S -> Q -> R -> P`. This keeps parent-owned resources available while releasing
descendants. Clearing contents preserves the root and its resources; destroying
the entire tree releases the root last. The ordering is between nodes and does
not define individual field-drop order within a node or extend the lifetime
of resources owned elsewhere.

Queued edits retain the mutation boundaries, queue ordering, and validation in
ADR-0029. Keeping the root does not preserve removed descendants or make their
IDs valid again. Resource release order does not decide the remaining hover
notification/cancellation staging by itself.
[ADR-0033](0033-hover-lifecycle-cleanup.md) now defines that staging: required
cancel precedes hover cleanup, and leave notifications complete before node
resources are released.

## Deferred creation and mounting

- Applications may retain data, properties, or a creation function without
  allocating a live retained node. The caller decides when to request actual
  creation; a declaration does not require immediate node allocation.
- Ordinary-node creation specifies a destination parent and append or indexed
  insertion. Successful execution creates and mounts the node in one operation.
  Rejection leaves no partially created live node or partial attachment.
- At observable operation boundaries, every live non-root node has exactly
  one parent and is reachable from the unique root. M2 has no independent
  detach operation that preserves a live node outside that tree.
- Moving a subtree exposes no detached intermediate state; removal destroys
  the subtree under the confirmed lifecycle rules.
- Creation and mounting do not require immediate layout, painting, or
  presentation. Participation and scheduling rules still determine that work.

Data and creation functions preserve ADR-0007's single retained tree without
requiring a second maintained virtual node tree. Delaying creation can avoid
unused node storage and initialization; retaining already created nodes can
instead preserve their identity and state. Display none suppresses layout,
painting, and hits but does not release node storage. Recreating a removed node
requires restoring any state that must outlive it from elsewhere.

This establishes M2's lifecycle boundary, not a component construction or
virtualization API. Component/conditional construction remains in M3; virtual
lists remain in M7. Queued creation uses the reserved-identity contract below;
a reserved identity is not a live detached node.

## Reserved identities for queued creation

- Enqueueing an actual creation command returns a reserved `NodeId` immediately.
  Later commands can use it directly, including as the parent of another
  creation. Retaining data or an unevaluated creation function alone does not
  reserve an ID or allocate a live node.
- A reservation stores identity metadata and queued command data without a
  live node or parent/child link. It does not participate in layout, painting,
  hit testing, or capture ownership. Event-handler reads still observe the
  pre-application live tree/style/capture state under ADR-0029.
- The creation command owns the reservation lifecycle. Copying or discarding
  caller-held `NodeId` values does not cancel creation or release the
  reservation. Success transitions it to the live node's identity with the
  same ID.
- Reading the reserved node before successful creation returns an explicit
  "not yet created" error. This distinction is part of the contract; concrete
  Rust error names and signatures remain separate decisions.
- Creation validates at its normal FIFO application turn. On success, create
  and mount the live node using the reserved ID. Later commands validate
  against the state at their own turns, including earlier successful creation.
- Failed creation permanently invalidates its reserved ID. It cannot become a
  valid identity for any future node. Dependent commands fail individually;
  independent later commands continue. When a dependent child creation fails,
  invalidate that child's own reserved ID too.
- Failure releases the reservation and command-owned resources. Destroying
  the queue before creation executes likewise releases those resources and
  permanently invalidates the ID. Reclaimed storage can be reused with a
  different identity, never one that revives the old ID. Nodes that were never
  created emit no enter, leave, or cancel event.
- Reservations obey tree ownership, generation, and checked exhaustion without
  wrapping or reviving an old identity. Destroying the tree invalidates its
  reservations as well as its live node IDs.

For one callback's queued create-P, create-Q-under-P, and set-Q-style commands,
P and Q can be referenced immediately but cannot be read as live nodes until
their creation succeeds. Failed P creation makes Q's creation fail; Q's ID
then becomes invalid and its style command fails. An unrelated valid command
still runs. ADR-0029's completed-batch report retains all outcomes and relevant
IDs, including IDs whose creation failed, without dereferencing node storage.

The selected contract reserves identity before execution; it does not apply a
live-tree change during a callback, reorder commands, or introduce temporary
reference resolution in result reports. Exact reservation storage and enqueue
error signatures are delegated implementation choices. Queue-destruction cleanup
does not select a separate cancellation API or an unfinished-batch reporting policy;
completed-batch reporting retains ADR-0029's contract.

## Insertion and movement

- Children are ordered; each node has at most one parent, and edits must
  preserve acyclicity.
- Support appending and insertion at a zero-based index from 0 through the
  destination child count, inclusive. Inserting at the child count appends.
- A move is confined to one tree and carries the entire subtree. Every moved
  node retains its ID, styles, and handlers. Layout uses the new parent context,
  so resulting geometry may change.
- Interpret a move's destination index after conceptually removing the node
  from its old position, using the state when the command is applied. A move
  within one parent's children therefore checks an insertion list one item
  shorter than before the move.
- Reject an invalid index, a cross-tree move, or a move into the node itself
  or its descendants without altering either tree. Root removal/reparenting
  remains invalid.
- Moving to the same parent and position succeeds without a structural change.
  It introduces no new invalidation or frame request and preserves any dirty
  work from other changes.

For `[A, B, C, D]`, moving B to index 3 uses `[A, C, D]` as the insertion list,
producing `[A, C, D, B]`. Index 1 preserves the original order; index 4 is
invalid for that move and must leave the original tree unchanged. Conceptual
detachment when validating an index does not permit a partially applied edit.

## Considered options

- **Slot and generation without tree ownership**: a smaller identity, but the
  same numbers can occur in different trees. A separate ownership mechanism
  would still be needed to reject cross-tree access.
- **Tree ownership and a monotonically increasing node number**: avoids reusing
  a node number but needs a mapping or unreused indices to locate reusable
  storage. It still needs explicit exhaustion handling.
- **Tree ownership, slot, and generation in an owned identity**: selected to
  combine reusable arena storage with explicit stale/foreign checks. The cost
  is identity metadata and validation at access boundaries.
- **Replace the root when clearing contents**: treats the root like other
  removable nodes, but invalidates the application's root handle and requires
  restoring root-specific state.
- **Keep one root for the tree's lifetime**: selected so clearing contents
  preserves a stable insertion point and the root's own styles and handlers.
  Root removal/reparenting is therefore invalid; full teardown destroys the tree.
- **Release resources in arena storage order**: avoids specifying tree-based
  release order, but allocation and reuse would determine which resources
  remain available while other nodes are being released.
- **Release descendants before their parent in child-list order**: selected
  to make resource release predictable and keep parent resources available
  through descendant cleanup. An explicit stack avoids recursive framework
  teardown and requires traversal bookkeeping.
- **Create real nodes for all declarations immediately**: prepares node state
  early but pays storage and initialization costs even for unused content.
- **Create live detached nodes, then mount later**: allows preassembly while
  deferring participation in the main tree, but retains node storage costs and
  requires an additional lifecycle for detached subtrees.
- **Allow delayed creation and mount on successful creation**: selected to let
  callers choose when to allocate real nodes while keeping every live node in
  one rooted tree. Parent creation must precede child creation; first use may
  incur creation work, and state that must survive removal needs another owner.
- **Batch-local temporary creation references**: distinguish pending creation
  with a separate type, but need a mapping to `NodeId` after execution and
  extra handling when retaining identities across batches.
- **Reserve `NodeId` when creation is enqueued**: selected so parent/child
  construction and later mutations use one identity type immediately. Costs
  include reservation metadata, pending-read errors, and permanent invalidation
  after failure; the ID alone never guarantees that a live node exists. The
  creation command owns reservation cleanup, so copying or dropping ID values
  does not alter creation behavior, and failed or discarded commands do not
  leave outstanding reservations or command-owned data behind.
- **Move indices against the original child list**: requires interpreting
  forward same-parent moves relative to the node still occupying its old slot.
- **Move indices after conceptual removal**: selected so insertion always uses
  the destination list that will receive the moved subtree. Callers must account
  for the shorter list when moving within one parent.

## Consequences and remaining decisions

- Keep the representation private under the existing
  [API conventions](../conventions/api-stability.md). Exact bit widths, arena
  storage/library choice, and reservation storage are delegated choices; the
  accepted timing contract does not select a dependency or make the internal
  API stable.
- Concrete structural/enqueue signatures, queue shutdown/reporting policy,
  and supported traversal limits are delegated M2 engineering choices. ADR-0033
  complements this resource-release contract with hover/cancellation staging.
- Tests cover copying and equality/hash consistency, owner distinctions even
  with matching slot/generation values, slot reuse, stale/foreign rejection,
  and owner destruction/recreation. Exercise counter exhaustion with a
  deterministic fixture rather than relying on a production-sized wraparound.
- API checks must preserve opacity and prevent UI/Scene identity interchange.
  Integration fixtures retain deleted IDs in reports and reject later commands
  targeting them without accessing reclaimed storage.
- Lifecycle tests cover automatic root creation, stable root identity through
  clear/repopulate, preservation of root-owned state, rejected root edits with
  no partial changes, subtree invalidation, and whole-tree invalidation. Verify
  node-owned state is released exactly once in child-list-ordered postorder,
  including after reordering/reparenting. Cover leaf, branching, and deep-tree
  teardown, the `S -> Q -> R -> P` example, parent resources outliving descendant
  release, root preservation on clear, and root-last full teardown. Use explicit
  stack traversal within the supported depth contract once selected. Extend
  tests with typed styles, handlers, and capture as those capabilities are
  introduced in M2.
- Structural tests cover append and indexed insertion, same-parent moves in
  both directions, cross-parent subtree movement, post-removal index bounds,
  owner/cycle rejection without partial edits, and preservation of subtree
  IDs/styles/handlers. Original-position moves must succeed without new
  invalidation or clearing existing dirty work. Layout integration verifies
  the moved subtree in its new parent context.
- Creation tests cover successful creation with immediate structural attachment
  and rejected creation leaving no live orphan or partial edit. Generated edit
  sequences verify every live non-root node has exactly one parent and is
  reachable from the root. No M3/M7 API is required for these fixtures.
- Reservation tests cover unchanged live membership, pending-read errors,
  successful creation with the same ID, permanent invalidation after failure
  and slot reuse, foreign access, checked exhaustion, and tree destruction.
  Handler integration covers queued parent/child creation, later mutation of
  the child, dependent failures with independent continuation, and reporting
  failed IDs without dereferencing their storage.
- Ownership fixtures discard caller-held ID copies without canceling creation,
  verify success keeps the same ID and transfers node-owned data without
  premature release, and verify failure or queue destruction releases reserved
  and command-owned resources exactly once. Cover failure followed by queue
  teardown and storage reuse that cannot revive old IDs. Input integration
  emits no enter/leave/cancel for nodes that never existed.

## M2 execution evidence (2026-11-20)

Generated edits compare live parent links and child order with a reference
tree. Focused fixtures check owner and generation exhaustion, atomic rejection
at the depth limit, reservation cleanup, and child-list-ordered postorder
release with the root preserved on clear.
[Tree contract tests](../../crates/oxiview-core/src/ui/tree_contract_tests.rs)
