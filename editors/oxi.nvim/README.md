# .oxi support for Neovim and LazyVim

The plugin detects `.oxi` buffers, highlights Rust expressions and view markup,
and connects Neovim's native LSP client to `oxiview-lsp` over stdio. It uses
the LSP mappings and completion UI configured by the editor. Neovim 0.11+
provides the required `vim.lsp.config` and `vim.lsp.enable` APIs.

In LazyVim's explorer, tabs and pickers, `.oxi` files use a sage-green filled
circle with an O cutout. The spec registers the Nerd Fonts `alpha-o-circle`
glyph for both the extension and filetype through MiniIcons. Its color adapts
to the editor's light or dark background.

## Install from a checkout

From the oxiview repository root, install the server and link the local
LazyVim spec. Start a new Neovim session after adding the spec.

```sh
cargo install --locked --path crates/oxiview-lsp
ln -s "$PWD/editors/oxi.nvim/lazyvim.lua" ~/.config/nvim/lua/plugins/oxi.lua
nvim examples/snake-oxi/src/screen.oxi
```

The [LazyVim spec](lazyvim.lua) locates the plugin through the checkout link
and runs the binary in Cargo's installation directory. A custom Neovim setup
can put this plugin on its runtime path and call `require("oxi").setup()`.
Pass `{ cmd = { "/path/to/oxiview-lsp" } }` to select a particular executable.
The default command is `oxiview-lsp` on PATH.

## Try it in Snake

1. In [screen.oxi](../../examples/snake-oxi/src/screen.oxi), place the cursor
   on `game_panel` in the component tag. LazyVim's `K` shows its signature,
   `gd` opens the declaration in `board.oxi`, and `gr` lists references.
2. In [button_face.oxi](../../examples/snake-oxi/src/button_face.oxi), use
   `gd` on `title` or `pointer`. The server follows the embedding macro to
   the local binding in [controls.rs](../../examples/snake-oxi/src/controls.rs).
3. In [board.oxi](../../examples/snake-oxi/src/board.oxi), put the cursor on
   `style` in `<Node style={overlay_surface()}>`. Use `gd`, `gD` or `gI` to
   open the concrete Rust `View::style` method, `gy` to open its `Style`
   argument type, and `gr` to list references across external views, Rust
   `view!` expressions and Rust calls with a resolvable receiver. On
   `overlay_surface` itself, `gd` opens the local style function.
   Completion after `<Node ` lists the Builder attributes.
4. Change a closing tag to `</Wrong>`. The diagnostic points at the name as
   you type and clears when the tag matches again.

Document and workspace symbols include named views, styles and Rust module
items. The editor uses open buffer text, UTF-16 positions and the compiler's
grammar. It indexes local Cargo workspace sources and path dependencies for
definitions, imported aliases and captured names. The view AST supplies the
binding between a markup token and its Rust target. Both `<Node>` and
`</Node>` refer to the node factory. A component's `props` attribute refers
to its input parameter, and a named style's fields refer to Rust struct
fields. Full Rust type inference, borrow checking and
trait method resolution belong to the Rust toolchain. Use
`cargo check -p snake-oxi` for the compiler's type diagnostics.

## Verify the installed editor

Run from the checkout with the plugin installed in LazyVim. This uses the
actual editor configuration, checks the rendered `.oxi` and `.rs` explorer
icons, makes temporary in-memory edits and discards them on exit.

```sh
nvim --headless -i NONE -n examples/snake-oxi/src/screen.oxi \
  '+luafile editors/oxi.nvim/tests/smoke.lua'
```

The [protocol tests](../../crates/oxiview-lsp/tests/protocol.rs) also run in
`cargo xtask verify`, independently of a Neovim installation.
