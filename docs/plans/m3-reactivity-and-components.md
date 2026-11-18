---
status: in-progress
milestone: M3
---

# M3 reactivity and components

The owner authorized implementation through a real M4 Snake demo on
2026-11-17, including optional JSX-like expression syntax in M3. This plan
implements [ADR-0005](../adr/0005-fine-grained-reactivity.md) and
[ADR-0006](../adr/0006-builder-api-and-function-components.md). Internal APIs
remain unstable. A working demo does not replace remaining milestone acceptance.

## Implemented foundation

- `oxiview-reactive` provides thread-local `Signal`, `Memo`, effects, batching,
  untracked reads, checked ownership/disposal, and bounded feedback processing.
  Signal handles are copyable and cannot be sent between threads. Memos use
  dependency tracking and equality suppression, including reads within a batch.
- Scopes own signals, effects, local tasks, and cleanup. View scopes follow
  retained node lifetime; hiding a Show branch destroys its scope. Keyed list
  entries retain identities and data signals until their keys are removed.
- Local tasks are wake-driven and scoped. A bounded background pool returns
  results to local futures; errors and full queues are explicit. The window
  loop establishes its executor before application construction and polls
  ready work on wakes, without an idle timer.
- `oxiview-core::View` is a consumed construction description. `node()` Builder
  chains, function components, typed properties, pointer handlers, and
  style/content/text bindings create one retained UI tree.
- `show` and `keyed_for` own dynamic structure. Stable keys keep node IDs and
  item signals; duplicate keys reject updates. Function component factories run
  once per mount, rather than on every signal change.
- `view!` parses nested Node/component tags and Rust expression children,
  lowering directly to the Builder API. Component tags accept ordinary props;
  `Props` derive generates checked Builders with required/default fields.
  Authors write these expressions in Rust `.rs` files. The optional macro
  compiles as part of the Rust crate. Standalone `.oxi` files have no loader
  or compiler in M3. [Component API decision](../adr/0006-builder-api-and-function-components.md)

## Mutation and lifecycle boundaries

Node-owned pointer callbacks follow the M2 fixed-route and queued-command
contracts. Effects that update bound node properties join the active event
command buffer in order. Per-batch report callbacks run after command completion
and before the batch's stationary-hover check.

Automatic binding/reconciliation jobs carry their scope owner. If the owner is
disposed before application, these jobs are counted as canceled bindings;
they are not reported as successful writes to deleted nodes. Direct commands
targeting stale nodes still fail explicitly. This distinction prevents normal
Show removal plus a source update from generating stale-node errors.

Reparenting preserves private mounted-component state and effects without
rerunning factories. Scope ownership follows the moved subtree, while references
to external ancestor-owned signals keep their original lifetime.
[ADR-0035](../adr/0035-node-owned-reactive-scopes.md) records checked scope
adoption, atomic rejection, and the corresponding lifecycle tests.

## Executable evidence

The [Snake application](../../examples/snake/src/application.rs) uses `view!`,
props, Signals/Memos, a conditional game overlay, keyed recent runs, scoped
button press state, bound custom board painting, and bound real text. A keyboard
or mouse action changes data and retained properties; it does not reconstruct
the screen component.

Targeted suites cover dynamic dependencies, memo diamonds, batching, checked
borrows, scope disposal, local/background task wake and cancellation, parser
diagnostics, optional macro expansion, keyed identity, report ordering, disposed
bindings, and native-independent window-loop integration. Snake's replay checks
`screen_mounts == 1` through gameplay and repeated restart, retained history IDs,
and zero rejected commands in ordinary operation.

## Completion boundary

Run ordinary local verification, retain meaningful error/cancellation tests,
and inspect the actual demo artifacts. Native visual acceptance and performance
baseline review remain separate gates. Dependency-graph Inspector UI, focus and
Action/Keymap, editable text, and a widget library remain in their later
milestones; this work does not pull them into M3.
