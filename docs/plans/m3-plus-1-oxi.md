---
status: done
milestone: M3+1
---

# M3+1 precompiled .oxi sources and Snake

M3+1 adds a source-level increment to [M3](m3-reactivity-and-components.md).
The example uses the text path already delivered with [M4](m4-text-and-snake.md).
The equivalence contract is recorded in [ADR-0037](../adr/0037-optional-precompiled-oxi.md).

## Authoring

Use `oxiview_macros::oxi_module!("src/buttons.oxi");` in a module to bring its
items into scope. Rust `mod`, `pub` and `use` organize imports and exports for
both source formats. Paths in both file macros are relative to the consuming
package's Cargo manifest. Changes to any included file invalidate the build.

```text
use crate::buttons::ButtonFaceProps;

style button_face(pressed: bool) {
    opacity: if pressed { 0.78 } else { 1.0 },
}

pub view button_body(props: ButtonFaceProps) {
    <Node bind_style={move || Ok(button_face(props.pressed.get()?))}>
        {props.title}
    </Node>
}
```

A named view has typed arguments, optional Rust setup statements and one root
written in the existing `view!` syntax. Rust expression children can use
`show` and `keyed_for`. A style declaration constructs `oxiview_style::Style`
from its fields, using `Style::default()` or an explicit `..base` for the rest.
Ordinary Rust functions and types are valid as the entire contents of a module.

Use `oxiview_macros::oxi_view!("src/button.oxi")` as an expression to embed
a bare root such as `<Node on_pointer={handle_pointer}>{title}</Node>`. Rust
resolves both names in the calling function. A pure-`.oxi` Cargo package can
set its binary target path to `src/main.oxi`. The crate root uses an ordinary
Rust macro invocation to load declarations, so the application can keep all
its source in `.oxi` files and run with stock Cargo on stable Rust.

## Acceptance

1. Compile the same view as a Builder chain, an inline `view!`, a named
   `.oxi` view and a mixed Rust/fragment view. Check Builder operations,
   binding and event behavior. Exercise logic-only and style-only modules.
2. Check rejection of malformed markup, unknown names, incorrect property
   types and invalid captures. Change an included file between two incremental
   builds and verify the executable observes the edit.
3. Provide `cargo run --release -p snake-oxi` with the native controls and
   headless CLI of the original game. Write the screen, board panel, controls,
   scores, conditional overlay and keyed history in `.oxi`. Use standard Cargo
   `.rs` entry points and Rust modules for control wiring and message formatting.
   In this example, choose `.oxi` for files that declare a view, a view fragment
   or typed styles. Share the model, simulation, pointer behavior, board drawing
   and replay with Snake.
4. Run both screens with the same seed, viewport, scale and input sequence.
   Compare all six CPU-rendered images, replay results and mount/update counts.
   Verify the existing idle, restart and keyed identity assertions. Each binary
   writes its own artifacts.
5. Run ordinary local `cargo xtask verify`.

## Execution record

On 2026-11-18 the [compiler tests](../../crates/oxiview-macros/tests/compile_oxi.rs)
rebuilt a pure-`.oxi` Cargo binary after independent edits to a named module
and a captured fragment. It also imported a logic-only `.oxi` module. The built
binary ran after removing the imported source files. Rust rejected invalid
local names, incompatible captures at a second
use site, moved values and incorrectly typed style fields. Syntax diagnostics
include the file path. Rust type diagnostics identify the file macro invocation.
The [four-form fixture](../../crates/oxiview-macros/tests/builders.rs) exercised
the same binding, event and child operations through all four source forms.

The [Snake replica](../../examples/snake-oxi/README.md) and the original
release binary passed the same eight replay checks. The six PNG files matched
byte for byte at the default seed, including the 1× and 2× ready screens.
Both reported one screen mount, eight overlay mounts, four keyed history
mounts, 105 board updates, zero rejected commands and a session best of 30.
The [parity test](../../examples/snake-oxi/tests/parity.rs) compares both full
reports and every image in workspace verification. The local macOS bundle also
passed the headless replay at seed 42 with its debug overlay enabled.
