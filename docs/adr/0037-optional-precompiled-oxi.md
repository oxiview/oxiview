---
status: accepted
---

# Optional precompiled .oxi sources share the Rust Builder contract

Developers may express the same program with Rust Builders, Rust and `view!`,
entirely in `.oxi` files, or across Rust and `.oxi` files. Source organization
is a reading preference. Equivalent expressions must construct the same Rust
values and perform the same calls in the same order. Component mounts, reactive
scopes, node identities, binding updates, pointer events and rendering follow
from those calls.

This extends the external-file choice in
[ADR-0006](0006-builder-api-and-function-components.md). The original concern
was the cost of owning a second language and toolchain. The optional macro crate
can reuse the existing tag parser and Builder expansion, while Rust checks
logic, types, visibility, lifetimes and ownership. The cost is a small file
grammar and diagnostics at a macro boundary. A runtime loader would also need
its own name resolution, execution model and deployment assets.

## Decision

- Precompile every referenced source during the ordinary Cargo build. The
  application binary uses ordinary Rust functions, typed styles and Builders.
- A file can contain just a bare view fragment. Embedding it in a Rust or
  `.oxi` function resolves its free names at the embedding site, with type and
  borrow checking at every use.
- A module can contain ordinary Rust items, named `view` declarations, named
  `style` declarations, or any combination. A named declaration has a Rust
  signature, module visibility and explicit inputs. A view returns a `View`.
  A style returns a `Style` with typed Rust fields. [Typed styles](0008-typed-style-with-css-vocabulary.md)
- Authors can move an expression to a fragment or call a named function across
  file boundaries. A named view declaration introduces exactly a function.
  Component tags use the same mount-time factory as `view!`. A fragment
  introduces zero nodes, scopes, subscriptions or runtime file reads.

The [M3+1 plan](../plans/m3-plus-1-oxi.md) specifies the source entry points
and the compiler and gameplay evidence required for acceptance.
