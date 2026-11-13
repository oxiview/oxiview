# Unsafe, panic, and error handling

## Unsafe boundaries

The workspace defaults to `unsafe_code = "deny"`. Under [ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) and the [dependency rules](../../xtask/layers.toml), only the following planned platform and rendering adapters may use unsafe for necessary system calls or resource handoffs:

- `oxiview-platform-macos`
- `oxiview-platform-headless`
- `oxiview-render-cpu`
- `oxiview-render-wgpu`

This is an allowance, not a requirement to create crates or use unsafe. Prefer safe Rust in the headless platform backend and CPU implementation. Windows / Linux adapters are not yet registered; name them and update the rules and these conventions at their milestones, without creating shells early. All other crates, including ports and the composition root, retain the workspace deny setting.

Use **crate-level `#![allow(unsafe_code)]`** to opt in explicitly: add it only when necessary at the top of an eligible adapter's `src/lib.rs`, documenting the boundary requiring unsafe. Keep `[lints] workspace = true` in Cargo.toml rather than replacing the full Cargo lint configuration. Do not bypass the rules with module- or function-level allows in other crates.

Each unsafe block must have an adjacent English `// SAFETY:` comment explaining why this call satisfies the relevant invariants: pointer validity, length and alignment, initialization, aliasing and lifetimes, resource ownership, and thread affinity, including the source of each guarantee. Unsafe function documentation must include `# Safety` describing caller obligations. Unsafe operations inside its body still belong in explicit unsafe blocks with SAFETY comments. Safe APIs must not silently transfer these obligations to callers.

## FFI boundaries

Keep FFI inside adapters: verify ABI, argument layout, null pointers, error codes, callback lifetimes, and matching resource release. Convert system objects to owned types before crossing contexts. Respect main-thread / window-thread constraints; raw pointers or unjustified Send/Sync implementations must not bypass thread affinity.

Rust panics must not unwind across FFI boundaries; foreign exceptions must not cross an undefined ABI. For system callbacks entering Rust under unwinding configurations, catch unexpected panics at the boundary, convert them to a host-recognized failure result, and request safe shutdown. Do not keep using affected state unless recovery is demonstrably safe. Catching and reporting must not panic again. `catch_unwind` cannot catch aborts, process termination, or arbitrary foreign exceptions; it is neither a safety guarantee nor a substitute for input validation.

## Panic scope and exceptions

Library production paths prohibit `unwrap`, `expect`, and `panic!`, as well as equivalent runtime panics through `assert!`, out-of-bounds indexing, `todo!`, `unreachable!`, and similar operations. Handle external input, unsupported capabilities, and device loss explicitly. Frame-loop paths, including event dispatch, layout, painting, presentation, and callbacks, must not deliberately panic.

Limited exceptions cover tests (including test-only modules and benchmarks), xtask, examples, and const assertions **guaranteed to be evaluated at compile time**. Explicitly allow the relevant Clippy lint in the smallest scope and explain its purpose; never relax it at a library root. A const fn may still execute at runtime, so const alone does not permit panics. Example shorthand must not become production error handling; test assertions are not a runtime recovery strategy.

The future runtime will catch unexpected unwindable panics at explicit outer boundaries such as window-thread entry points and FFI callbacks, record context, notify the application to terminate the affected window or process, and release only resources known to be safe to release. Do not resume the frame loop after catching a panic or use AssertUnwindSafe to conceal invariant problems. If safe state cannot be maintained, terminate the process with a nonzero status, prioritizing avoidance of repeated unwinding or deadlock. These constrain later implementation; M-1 has no runtime panic handler.

## Error categories and types

Return recoverable errors as `Result<T, Error>` so callers can retry, choose alternatives, or display errors. Libraries define structured errors with `thiserror`. Propagate fatal errors explicitly to the composition root; the application boundary reports the cause and window/operation context with `tracing::error!` and exits safely. Without logging facilities, the application boundary provides stderr fallback. Never silently swallow errors or use panic as an error channel. Observability arrives at the relevant milestone; this step adds no dependencies or runtime code.

Crates returning errors use one top-level `Error` enum as their common entry point, optionally wrapping private categories; name variants in the context's own terms. Map upstream errors to owned errors through an anti-corruption layer. Public variant fields, aliases, generic bounds, and return types must not leak third-party error types. Preserve causes through private wrappers or standard-library error chains without making other contexts depend on concrete upstream errors. Crates that return no errors need not create empty enums for formality.

Existing named S4 harness errors such as `SnapshotError` remain compatible and are not refactored in this step. Handle migration separately when unifying entry points later. `anyhow` is limited to engineering tools, tests, and examples; it does not enter library production APIs ([ADR-0022](../adr/0022-foundation-libraries.md)).
