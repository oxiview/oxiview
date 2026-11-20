---
status: done
milestone: M6
---

# M6 text editing and the Markdown editor

M6 delivers a self-drawn desktop Markdown editor. A writer can open a real file, edit its source on the left, read a live preview on the right and save the result. The multiline source and the single-line Find and Replace fields exercise the same editing core. [Markdown editor](../../examples/markdown-editor-oxi/README.md) · [Roadmap](../roadmap.md) · [Text stack](../adr/0010-pure-rust-text-stack.md) · [M5 acceptance](m5-focus-keyboard-and-themes.md#acceptance-record)

## Owner decisions

| Decision | What the writer can observe |
| --- | --- |
| Use a standalone Markdown editor with two panes. | The source stays editable and visible alongside its rendered preview. |
| Bring system plain-text copy, cut and paste and native single-file Open and Save panels into M6. | The writer can paste from another Mac app and reopen the saved file through the system picker. M9 owns the wider desktop integrations. |
| Render Markdown extensions, Mermaid and mathematics. | The preview displays task lists, tables, strikethrough, local images, diagrams and typeset formulas alongside the basic Markdown constructs. |
| Publish text when the input method confirms it. | During Chinese input the source shows underlined preedit. Choosing a candidate updates the preview and adds one undo step for that commit. |
| Cancel outstanding preedit when focus leaves the editor. | Clicking Find restores the original source selection. A commit already delivered to the old editor is retained. [Preedit and committed text](../adr/0042-publish-committed-editor-text.md) |

## Acceptance session

1. Open a UTF-8 Markdown file through the macOS Open panel. It contains a Chinese heading, a mixed Arabic/Latin paragraph, a task list, a table, a relative image, a Mermaid diagram and both inline and display math. Read and scroll both panes. A long document must keep the insertion point visible as the writer types and moves across wrapped lines.
2. Select part of the heading and replace it using Chinese Pinyin. Use the candidate keys to choose a word. Watch the candidate window follow the caret after a wrap and a window move. Confirm, undo and redo. Repeat the edit, click Find before confirming and check both fields and the preview.
3. Drag a selection through wrapped and bidirectional text, extend it with Shift, then delete an emoji joined by a zero-width joiner. Copy text to a second app and paste plain text back. Find the next and previous literal match, replace one match, then Replace All. One undo restores the entire batch.
4. Change an edge label in a diagram and a denominator in a formula. The corresponding drawing changes. Break the syntax during typing and then correct it. The affected block shows a diagnostic with its source location, the surrounding document stays readable, and the corrected block reappears.
5. Use New, Open, Save and Save As. A modified document offers Save, Discard and Cancel before replacement or close. Cancel a panel, simulate a failed write and retry. Reopen the saved file and compare its source, including Unicode and Markdown delimiters. Closing a clean document needs no prompt.

The proposed shell has a file toolbar, the two document panes and an inline Find and Replace bar. Standard shortcuts and toolbar buttons invoke the same Actions. Bold and Italic wrap a source selection as one edit. Escape closes Find after the input method releases it and returns focus to the source selection. Enter in the search field advances to a match, while Enter in the source inserts a newline.

## Framework and example ownership

| Location | Proposed responsibility |
| --- | --- |
| `oxiview-text` | Own document edits, selection and caret affinity, preedit, undo history, and the editor protocol. Extend the M4 layout with hit testing, visual caret movement and selection rectangles. Keep storage replaceable behind editing operations. Start with a UTF-8 string. [Text port](../../crates/oxiview-text/src/lib.rs) |
| `oxiview-core` | Bind a retained editable node to the model. Reuse M5 focus and Actions, M2 pointer capture, the layout pass and the Scene painter. Implement source and search fields through the same Builder with multiline and single-line policies. [Key dispatch](../../crates/oxiview-core/src/ui/keyboard.rs) · [Text nodes](../../crates/oxiview-core/src/ui/text.rs) |
| `oxiview-platform` and its adapters | Expose owned text-input events and editor-state updates, plain-text clipboard operations and single-file panel requests. macOS implements the native services. Headless injects input, clipboard contents, selections, cancellations and errors through the same port. Windows and Linux adapters follow at M14 and M15. [Window endpoint](../../crates/oxiview-platform/src/interface.rs) · [Platform order](../adr/0018-macos-first-headless-backend-hedge.md) |
| `oxiview-app` | Connect focused text input and platform replies to the window loop. Keep AppKit objects on the main thread and document state on the window thread. [Threading contract](../adr/0003-window-threading-model.md) |
| `examples/markdown-editor-oxi` | Own document paths and file I/O, Find and Replace, Markdown parsing, preview blocks and the two-pane interface. Organize these as document, preview and view modules with a separate replay entry point. Use precompiled `.oxi` for the shell and the same Rust Builders for editable fields. Keep Markdown and diagram dependencies with the example. [Editor shell](../../examples/markdown-editor-oxi/src/screen.oxi) · [Layer rules](../../xtask/layers.toml) · [Authoring contract](../adr/0037-optional-precompiled-oxi.md) |

## Editing and input contracts

- Store positions as checked UTF-8 byte offsets with selection anchor, active end and visual affinity. User cursor stops and character deletion respect extended grapheme clusters, including combining marks and joined emoji. Left and Right follow visual order in mixed-direction text. Up and Down preserve the desired horizontal position across wrapped lines. Hit testing, caret placement and selection rectangles use the same shaped paragraph as painting. Include the empty document and trailing empty line. [Unicode text segmentation](https://www.unicode.org/reports/tr29/) · [Unicode bidirectional layout](https://www.unicode.org/reports/tr9/)
- Give each focused editor session an identity. A key can produce several ordered input-method callbacks in one turn. The native adapter accounts for a consumed stroke once, including repeat and release. Unclaimed editing commands resolve through the focused Keymap. Tab traverses fields when the input method releases it. Printable text arrives through text commits. Test dead keys, candidate navigation with arrows and Enter, a late callback from the previous field and the first character typed immediately after clicking the new field. [Input ADR](../adr/0011-ime-and-action-keymap.md) · [AppKit text input](https://developer.apple.com/documentation/appkit/nstextinputclient)
- AppKit asks synchronous questions about selected and marked ranges, surrounding text and screen rectangles. Publish a revisioned view from the window thread. The main-thread adapter applies pending native callbacks to its view so a query in the same call sees the new marked range. Translate UTF-16 offsets and screen coordinates at the adapter boundary. Reject stale or invalid replacements without modifying the document. Exercise a non-BMP character before the range, a surrogate split, a request spanning lines and a window moving between displays. [Candidate geometry](https://developer.apple.com/documentation/appkit/nstextinputclient/firstrect%28forcharacterrange%3Aactualrange%3A%29)
- Group adjacent direct typing or deletion using the injected clock. Movement, a change of field, a selection replacement, paste and an IME commit delimit groups. Undo restores text and selection together. A new edit after undo drops the redo branch. Track the saved document state across undo, redo and asynchronous writes. The search field owns its own history.
- Caret and selection changes repaint the editor. Text changes remeasure affected paragraphs. Use clipped, scrollable document viewports in the example, including wheel input and keyboard caret reveal. General scrolling widgets, inertia and virtual lists belong to M7. Drive caret blink from window deadlines while visible and focused. A hidden or inactive editor allows the window to go idle. [Frame scheduling](../adr/0004-frame-scheduling.md)

## Preview and file behavior

Parse committed source into blocks and inline runs with source ranges. Render headings, paragraphs, emphasis and strong text, lists, quotes, links, thematic breaks, inline code and fenced code with native text and Scene primitives. Add task lists, tables and strikethrough. Resolve local images relative to the current document and show their alt text if loading fails. Preserve the original source through every save. [CommonMark](https://spec.commonmark.org/0.31.2/) · [GitHub Flavored Markdown](https://github.github.com/gfm/) · [Rust parser candidate](https://github.com/pulldown-cmark/pulldown-cmark)

Render fenced `mermaid` blocks as diagrams and `$…$` and `$$…$$` as inline and display math. The proposed fixture set includes a flowchart, a sequence diagram, a state diagram, inline subscripts, a stacked fraction, a radical and a matrix. Align inline math to the surrounding text baseline. A fenced code example containing dollar signs still displays literal code. Before selecting dependencies, compile a native renderer for each format on the pinned Rust toolchain, check the font and crate licenses, and draw the fixtures at two display scales. The upstream projects expose native vector commands and a Rust math layout respectively. Their fit with oxiview still needs this prototype. [Mermaid renderer candidate](https://github.com/1jehuang/mermaid-rs-renderer) · [Math renderer candidate](https://docs.rs/latex-rust/latest/latex_rust/)

Compile expensive preview blocks off the window thread. Identify results by document and source revision, reuse unchanged drawings and discard obsolete results after a newer edit or Open. Caret movement, selection and preedit reuse the published preview. The left pane accepts typing while a diagram is being laid out. Test slow completions in reverse order.

Read and write files away from the window thread. Preserve the source's line-ending convention and UTF-8 BOM when present. Save a specific revision by atomic replacement and serialize writes to the same destination. A write completing after another edit leaves the newer source marked modified. A cancelled Open or Save As preserves the current path, text and history. Exercise the existing window-close intent as well as the application's own New and Open Actions. [Window close handling](../../crates/oxiview-core/src/lib.rs) · [Native Open panel](https://developer.apple.com/documentation/appkit/nsopenpanel) · [Native Save panel](https://developer.apple.com/documentation/appkit/nssavepanel)

Clipboard and panel replies carry request identities. A failed clipboard write leaves the cut selection intact. An outstanding paste belongs to the field and revision that requested it. Exercise paste completing after the user switches from source to Find. The headless backend supplies these races on demand. Native review checks interoperability with a second app. [AppKit pasteboard](https://developer.apple.com/documentation/appkit/nspasteboard)

## Implementation order and green-check criteria

| Step | Required evidence |
| --- | --- |
| S1. Text geometry and editing core | Deterministic tests for grapheme deletion, visual navigation, wrapped selections, replacements, undo grouping and redo. Round-trip point to caret to painted geometry with pinned fonts. |
| S2. Focused native input | An AppKit probe and the headless backend exercise mark, replace, commit, cancel, rapid focus changes, candidate rectangles and stale callbacks. A person verifies actual Pinyin candidate choice and placement on macOS. |
| S3. Usable document editor | Build the two panes, single-line search, independent viewports, clipboard and file panels. Replay real file round-trips, unsaved changes, cancelled requests, failed writes and retry. |
| S4. Markdown rendering | Start the diagram and math feasibility work alongside S1. The finished example renders every agreed construct. Compare the preview against fixtures, including broken and corrected input, and check that typing keeps focus and the document stays mounted. |
| S5. Fuzzing | Run deterministic regression seeds in ordinary stable `cargo xtask verify`. Exercise isolated `cargo-fuzz` targets for Unicode edit sequences and native range/session events. Record the seed corpus, elapsed time, execution counts and crash count for each target. Check valid boundaries, unchanged state after a rejected edit, cancel restoration, undo/redo round-trips and absence of panics. Retain minimized failures as stable regression seeds. [Toolchain policy](../adr/0020-cargo-workspace-xtask-stable.md) |
| S6. Release acceptance | Drive the writer journey through a release headless replay and a native macOS window. Review CPU images at 1×, 1.5× and 2× covering preedit, selection, diagrams and math. An inactive editor produces zero additional presentations across 1,000 virtual opportunities. Compare M6 measurements and run local verification before recording completion. |

Proposed window-free workloads for `cargo xtask bench --m6` are replace-and-undo in a fixed 4 KiB mixed-script source, hit testing and selection geometry on 32 cached wrapped lines, and 16 preedit updates followed by one commit. Count preview parses separately in the replay to show that preedit generates zero parses and one commit publishes one revision. M6 completion requires its own native review, snapshot review and machine measurements. [Performance protocol](../conventions/testing-performance.md) · [Snapshot review](../conventions/testing-snapshots.md)

## Acceptance record

The example parses Markdown with pulldown-cmark. Fermaid lays out flowcharts, sequence and state diagrams in Rust, and resvg produces the diagram image. RaTeX supplies mathematical glyph positions, paths and rules, including the baseline for inline formulas. Native CPU tests cover each format, local images, changed and reused blocks, and source locations in erroneous Markdown. [Implementation and dependency review](../conventions/dependencies.md#m6-markdown-preview) · [Preview tests](../../examples/markdown-editor-oxi/src/preview/tests.rs)

The design preflight found stable Rust 1.97 on the owner's Mac and an older nightly. A direct build of the two libFuzzer targets with LLVM coverage works with the stable compiler. The isolated nightly tooling path supports sanitizer campaigns. Both targets share their oracles with ordinary stable regression tests. [Build and reproduction commands](../../fuzz/README.md)

The Mac14,10 coverage campaign started each target with its checked-in seed, an empty writable corpus and RNG seed 20261120. Both ran for 61 seconds with a 384-byte input limit. The editing model completed 938,643 executions, exercised 535 coverage points and retained 776 corpus inputs. The input-session target completed 1,102,806 executions, exercised 321 coverage points and retained 647 inputs. It includes the AppKit adapter's UTF-16 range conversion. Both completed with zero crash artifacts. [Coverage oracles](../../fuzz/scenarios.rs) · [Native range conversion](../../crates/oxiview-platform-macos/src/macos/utf16.rs)

With the owner's approval, the Mac also installed cargo-fuzz 0.13.2 and nightly 1.102.0 (6905905bb) for a separate AddressSanitizer campaign. Each target started from its checked-in seed and ran for 61 seconds. The editing target completed 309,226 executions and the input-session target completed 338,845, with zero sanitizer failures or crash artifacts. Framework builds and ordinary verification used the pinned stable Rust 1.97.

The release headless replay ran ten checks through the window loop and wrote seventeen CPU images. It changed an edge label and formula, checked the changed pixels, introduced source-located errors and recovered the earlier valid graphics after correction. A marked heading caused zero preview parses and its commit caused one. Reopening the saved UTF-8 document reproduced its BOM and CRLF bytes. An inactive window produced zero additional presentations across 1,000 virtual display opportunities. [Release scenario](../../examples/markdown-editor-oxi/src/replay/scenario.rs) · [File and focus regressions](../../examples/markdown-editor-oxi/src/app/tests.rs)

After the owner confirmed desktop readiness, the release AppKit probe passed against the real input client. It checked marked text and synchronous selected ranges, Unicode replacement and substring queries, candidate screen geometry across a window move, an ordered switch of editor sessions and a private pasteboard round trip. It exited automatically. [AppKit probe](../../crates/oxiview-platform-macos/src/macos/m6_input_probe.rs)

On 2026-11-20 the owner accepted the packaged editor in its native macOS window after checking Chinese Pinyin candidate placement, the preview update after confirmation, one-step undo, Save As and reopening the document. The owner also approved the three final multi-scale editor snapshots and recording only the M6 metrics on this Mac.

The reviewed 1×, 1.5× and 2× CPU images show the committed Chinese heading, underlined Pinyin with a selected segment, Arabic text, a diagram and two typeset formulas. The 1× and 1.5× images use the full local-to-device transform to align preview text at a fractional pane origin. Each accepted PNG matched the final reviewed candidate byte for byte. [Snapshot specimen](../../examples/markdown-editor-oxi/src/snapshots.rs) · [Accepted images](../../examples/markdown-editor-oxi/tests/snapshots/)

The scoped M6 recording added exactly three entries to the Mac14,10 baseline. All 28 earlier entries and the machine metadata retained their original values. The existing eight M2, four M3, four M4 and three M5 workloads passed their recorded thresholds in serial runs after compilation. [Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json) · [Benchmark protocol](../conventions/testing-performance.md#m6-editing-workloads)

| M6 workload | Recorded | Immediate comparison |
| --- | ---: | ---: |
| Replace and undo in 4 KiB | 201.5 ns | 197.6 ns |
| Cached selection geometry on 32 lines | 1.876 µs | 1.861 µs |
| Sixteen preedit updates and one commit | 1.571 µs | 1.593 µs |

The bundled release executable reproduced the complete replay. The separately invoked release integration test also verified that replay output preserves existing user files. [Release integration test](../../examples/markdown-editor-oxi/tests/release_replay.rs)
