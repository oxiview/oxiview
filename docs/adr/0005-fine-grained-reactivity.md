---
status: accepted
---

# Reactivity: thread-local fine-grained signals and structural reconciliation, with a built-in local executor

Build views only once. **Signal** changes directly update bound node properties through dependency tracking, without re-executing components or diffing virtual trees. Dedicated control-flow primitives handle **structural changes** such as conditionals and lists, reconciling lists by key. The approach resembles SolidJS, Leptos, and Floem. It provides the finest invalidation granularity and zero idle overhead, best matching ADR-0001's hard performance targets.

The reactive runtime is **thread-local**, with one instance per window thread (ADR-0003). Signal handles are `Copy + !Send`, with lock-free reads and writes. Each window thread includes a single-threaded **local executor**: futures started by `spawn_local` can access signals directly; `spawn_background` sends compute-intensive work to a background pool and delivers results to the originating window thread. The framework does not bind to a particular async runtime such as tokio.

## Considered Options

- **Component re-execution and virtual-tree diffing** (Dioxus, Xilem): simple mental model, but incurs diff overhead and limits invalidation granularity to components.
- **Elm unidirectional data flow** (iced): predictable, but rebuilds large trees and requires substantial boilerplate.
- **Pluggable reactive layer**: would force the widget library to support multiple models; rejected.

## Consequences

- Provide control-flow primitives such as Show / For / Switch; component authors cannot express dynamic structure with ordinary `if` / `for`.
- Dependency-graph debugging (who depends on whom and what triggered an update) must be part of developer tools (decision 15).
- Scopes own signal lifetimes. Destroying a scope releases its signals and effects together.
