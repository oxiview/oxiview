---
status: accepted
---

# Reactive ownership: node scopes follow retained subtree moves

M3 adds scopes, effects, and local tasks to M2's retained nodes. Keeping a moved
node's scope beneath its former component would let removal of the old parent
dispose still-visible state and handlers. Remounting would instead lose the
identity and state preservation required by
[ADR-0032](0032-owned-generational-ui-node-identity.md). This implementation
decision follows the owner's authorization to complete M2-M4 without further
intermediate design confirmations.

This refines [ADR-0005](0005-fine-grained-reactivity.md),
[ADR-0006](0006-builder-api-and-function-components.md), and ADR-0032. It keeps
the single retained tree in [ADR-0007](0007-single-arena-node-tree.md) and does
not add a persistent view tree.

## Decision

- Every live node, including the root and imperatively created nodes, owns a
  base reactive scope. A reserved creation identity does not allocate a scope;
  scope creation occurs with successful node creation and attachment.
- Ordinary creation and public Builder mounting enter the destination node's
  scope. Function-component and keyed-item scopes wrap the resulting node's
  private scope without adding an extra UI node. The node owns this complete
  chain; its innermost scope is the node context and its outermost scope is the
  unit transferred on reparenting.
- Node-owned handlers run in their node context. Signals, effects, and local
  tasks created by those handlers therefore share the node's lifetime.
- Moving a subtree to another parent adopts its outermost private scope under
  the destination node's context. Private signals, effects, tasks, and handlers
  remain intact; node IDs stay unchanged and component functions do not rerun.
  Same-parent ordering changes do not change scope ownership.
- Validate the UI owner, index, cycle, and depth rules, then validate scope
  adoption before lifecycle notifications or tree-link changes. Scope adoption
  rejects disposed or foreign-runtime handles, the thread root, ownership
  cycles, and an actively executing moved scope/subtree. An active destination
  is allowed; adoption to the existing scope parent is a no-op.
- Perform the final checked adoption immediately before updating tree links,
  with no application callback between the two. A rejected adoption leaves
  scope membership and structural links unchanged. Handler-generated moves
  remain deferred under [ADR-0029](0029-deferred-event-handler-mutations.md).
- Node removal still releases descendants before parents, including their
  scopes. Removing the old parent after a move cannot release the moved scope;
  removing the moved subtree or its new parent disposes those resources once.

## External dependencies keep their own lifetimes

Adoption transfers owned scopes, not arbitrary signal dependencies. If a moved
component captures a signal owned by its former ancestor, that signal remains
owned by the ancestor. Disposing that external scope invalidates the signal;
the moved component's private state and handlers still exist, but a subsequent
read of the external handle reports disposal. Applications place shared state
in a scope that outlives all of its consumers.

`Scope` is the owning resource. The `Owner` API is a non-owning scope handle for
entering or validating a context; copying it does not extend resource lifetime.
Low-level scope adoption does not itself move UI nodes. Application code uses
the tree's reparenting operation to keep these structures consistent.

## Deferred work and window lifetime

Automatic property bindings and Show/For reconciliation carry their owning
scope handle into the mutation queue. Work whose scope is disposed before
application is canceled without touching a replacement node or reporting a
successful mutation. Batch reports count this as `canceled_bindings`; explicit
application commands still validate normally and report stale-node failures.
The same cancellation applies to automatic bindings produced during terminal
cleanup or a stopped dispatch. Application-local signal cleanup can complete
without creating new UI work; explicit senders still reject closed queues.

Each `run_window` creates a root scope and local executor before application
construction. Local work runs only when ready, with a bounded initial poll and
polls before Wake delivery. Sleeping tasks add no polling timer or frame request.
Window cleanup disposes scoped tasks before the endpoint is released. This
preserves the existing presenter-before-endpoint surface lifetime rule.

## Considered options

- **Leave scopes under their creation parent**: simple, but removing that parent
  can disable nodes that have moved elsewhere and remain live.
- **Remount the component after a move**: reconstructs ownership but discards
  private state, effects, tasks, and the retained-identity contract.
- **Detach moved scopes into a global lifetime**: preserves immediate state but
  makes cleanup depend on unrelated lifetime decisions and can retain resources
  beyond their new parent or window.
- **Adopt the node's private scope chain**: selected to preserve state while
  matching the new parent lifetime. It requires checked ownership changes and
  an explicit distinction between private state and external dependencies.

## Verification

Core integration tests move a mounted component, remove its old parent, and
verify the same node ID, private signal, effect, and handler still work without
another component invocation. They verify that captured external ancestor
signals are disposed, that new-parent removal releases private resources once,
and that active-scope, foreign-node, and cycle failures do not change links or
cancel capture. Imperative nodes receive the same handler ownership guarantee.

Reactive tests cover adoption validation, active nested contexts, unchanged
subscriptions, sleeping-task ownership, and disposal. Window-loop fixtures
exercise mount/handler-spawned tasks, wake-driven updates, node/window
cancellation, error cleanup, and the absence of idle polling timers.
