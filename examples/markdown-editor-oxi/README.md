# Oxiview Markdown

[English](README.md)

Edit Markdown source on the left and read its live preview on the right. Start with the included field notes or pass a UTF-8 document path on the command line. The welcome document opens as an untitled copy.

```sh
cargo run --release -p markdown-editor-oxi
cargo run --release -p markdown-editor-oxi -- notes.md
```

Use New, Open, Save and Save As in the toolbar or their standard Command shortcuts on macOS. Dirty documents offer Save, Discard and Cancel before closing or switching files. The native panels choose a single file. The editor preserves UTF-8 BOM and the document's line-ending convention. The bundle script produces an application under `target/xtask/markdown-editor-oxi/`. [Bundle script](package-macos.sh)

Use the keyboard or drag to select text. Command+C, X and V use the system plain-text clipboard. Command+Z undoes an edit and Shift+Command+Z redoes it. The Bold and Italic buttons surround the selected source with Markdown delimiters. Command+F opens Find and Replace. Enter and Command+G advance to the next match, Shift+Command+G selects the previous match, and Escape returns to the source. Replace All is one undoable edit.

With a Chinese input method, the marked spelling appears inline while the preview continues to display the committed text. Choosing a candidate updates the preview once. Moving to Find with an outstanding mark restores the old source selection. The preview renders tables, task lists, strikethrough, local images, Mermaid diagrams and inline and display LaTeX. The acceptance suite covers flowcharts, sequence, state, class, ER and pie diagrams. Graphic errors point to the affected source location. [Example document](fixtures/acceptance.md) · [Preview implementation](src/preview/mod.rs)

## Acceptance

```sh
cargo run --release -p markdown-editor-oxi -- --headless --json
cargo test --locked -p markdown-editor-oxi
cargo xtask bench --m6
```

The release replay drives a headless window with the application's own retained controls, CPU presenter, text input and desktop service replies. Each run has a separate directory under `target/xtask/markdown-editor-oxi/`. A successful run writes seventeen PNGs, the document round trip and `replay.json`. The AppKit probe opens a short-lived native window for desktop review. [Window replay](src/replay.rs) · [Native probe](../../crates/oxiview-platform-macos/examples/m6_input_probe.rs) · [Editing fuzz targets](../../fuzz/README.md) · [M6 design and acceptance](../../docs/plans/m6-text-editing-and-markdown.md#acceptance-record)
