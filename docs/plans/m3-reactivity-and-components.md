---
status: done
milestone: M3
---

# M3 reactivity and components

The owner authorized implementation through a real M4 Snake demo on
2026-11-17, including optional JSX-like expression syntax in M3. This plan
implements [ADR-0005](../adr/0005-fine-grained-reactivity.md) and
[ADR-0006](../adr/0006-builder-api-and-function-components.md). Internal APIs
remain unstable. M3 closed on 2026-11-20. [Acceptance record](#acceptance-record)

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

M3 acceptance includes ordinary local verification, error and cancellation
tests, inspection of the replay images, native visual review and performance
baseline review. The dependency-graph Inspector, focus and Action/Keymap,
editable text and the widget library have their own milestones.

## Acceptance record

On 2026-11-20 the owner confirmed that the current native Snake application
passed the manual review. The checklist covered starting and steering,
pause and resume, the game-over history, and the score and buttons after a
restart. The release bundle also passed all eight headless replay checks.
The replay reached a score of 30, mounted the screen once, retained keyed
history rows across restarts and reported zero rejected commands. The
ready, playing, paused, game-over and compact images were inspected.
[Snake replay and native review](../../examples/snake/README.md#headless-acceptance)

Two regression tests cover keyed-list failures. If a row factory rejects
new data, the list discards rows staged during that mount and releases their
scopes. If an item binding fails after rows have mounted, the next valid
update reuses those same nodes. The failing update still reports the error.
[Public component contracts](../../crates/oxiview-core/tests/m3_views.rs)

Local `cargo xtask verify` passed all seven checks. The workspace test run
passed 369 tests, including component recovery, scope adoption, executor
wake and cancellation, macro compilation, and Snake replay parity.

The owner approved recording four M3 metrics on the Mac14,10. The immediate
window-free comparison passed all four and the scoped recording preserved
the 17 earlier machine-baseline entries. [Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json) · [Workloads and measurement method](../conventions/testing-performance.md#m3-reactivity-and-component-benchmarks)

| M3 workload | Recorded | Immediate comparison |
| --- | ---: | ---: |
| Batched memo diamond | 821.3 ns | 823.2 ns |
| Bound style update | 593.6 ns | 618.8 ns |
| Show mount and disposal | 9.901 µs | 10.160 µs |
| Reorder 32 keyed rows | 61.52 µs | 62.57 µs |
