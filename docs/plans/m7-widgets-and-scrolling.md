---
status: done
milestone: M7
---

# M7 basic widgets and scrolling

The owner selected a native log viewer as the acceptance application. A reader can filter 100,000 records, switch between a list and a table, inspect an entry and follow new arrivals. Both views read the same records. The native scrolling targets are stable 120 FPS and average hitch time at or below 1.2 ms/s. [Cadence and hitch decision](../adr/0046-native-scroll-cadence-and-hitches.md) [Roadmap](../roadmap.md) · [Original performance target](../adr/0001-goals-and-non-goals.md) · [M6 acceptance](m6-text-editing-and-markdown.md#acceptance-record)

## Owner decisions

| Decision | What the reader can observe |
| --- | --- |
| Use a standalone native log viewer. | The same filtered records appear in list and table presentations. |
| Supply a reproducible 100,000-record sample and open real local logs. | The application starts with a complete dataset and can follow a file chosen through the native Open panel as another process appends to it. |
| Support variable heights in the list and equal heights in the table. | Long messages wrap and can expand inline in the list. Table rows stay compact and the selected row exposes its full message in a detail pane. |
| Pause Follow when the reader scrolls back. | Incoming records accumulate behind a notice showing how many new matching records have arrived. Activating the notice returns to the latest record and resumes following. |
| Support row and cell operations in the table. | The reader can sort by a column, drag its width, select or copy a whole row, navigate individual cells and copy a rectangular range. |
| Collapse a range to its active cell on sort or filter. | The original record and column remain selected at their new position. Filtering out the record clears the selection. Scrolling and dragging a column width preserve the full rectangle. [Table selection contract](../adr/0044-keyed-virtual-table-selection.md) |
| Pass unused scroll distance to the enclosing viewport. | At the end of a nested detail pane the outer view continues the gesture. The outermost view stops at its content boundary. [Nested scroll contract](../adr/0045-nested-scroll-distance.md) |

## Reader journey

1. Launch with the seeded dataset. Type a search using the M6 single-line field, combine it with severity checkboxes, and clear the search. During input-method composition the current results stay in place until the search text commits.
2. Scroll to the middle, select a record and expand a wrapped message. Resize the window, let neighboring rows remeasure and switch to the table. The selected record still identifies the same message. Navigate between records with the keyboard and use the detail pane to read the entire entry.
3. Scroll both axes of a wide table. Sort by severity, drag a column boundary and select the numbered row header to copy the original message. Click a data cell and extend its selection with Shift and the arrow keys across a viewport boundary. Copy the range and inspect the rows and columns in the system clipboard. Drag the scrollbar thumb and return to the range to check its highlight.
4. Open a local text log and append complete and partial lines from a second writer. With Follow on, the newest matching entry stays visible. Scroll up, append more data and check that the record at the top stays at the same position. Activate the new-record notice to see the latest entry. Switch files while the first still has a read pending.

The control set proposed for this journey is a push button for Open and Copy, severity checkboxes, a Follow switch, a single-choice List/Table control, a single-line search field and draggable scrollbars. Each control has an application use in the demo. The list and table each provide one Tab stop. Arrow and page keys move the active record, while Tab moves between the collection, filters and other controls. Retain the selected key even when its row scrolls outside the mounted range. [M5 focus recovery](../adr/0041-focus-recovery-in-tab-order.md)

## Following and selecting records

While Follow is on, arrivals matching the current filter move the viewport to the newest record in arrival order. Scrolling back pauses viewport movement while the file reader continues. Count distinct matching arrivals since the pause. Extending the last unterminated record keeps its identity and contributes one arrival across its successive reads. Returning to the bottom by ordinary scrolling keeps the current pause. A new-record notice or explicitly enabling Follow resumes it.

The proposed interaction when sorting is to pause Follow and keep the chosen order. Resuming Follow restores arrival order and reveals the newest matching record. With an empty result the filters remain usable, and a subsequent matching arrival can become visible. Use the stable key and the offset into the top row to anchor a paused viewport as earlier estimated heights resolve.

The table has a row header and data cells. Selecting the row header makes Copy return the original log message. Selecting a data cell makes it the active cell. Arrow keys navigate, Shift+arrow extends a rectangle and a pointer drag can extend it across viewport edges by scrolling. Keep the anchor and active end in the table model. Resolve text for offscreen cells from the data source so Copy spans unmounted rows and columns. Encode values in displayed row and column order as tab-separated text with newline-separated records. Quote fields containing a tab, newline or quote and double embedded quotes. Reuse the M6 system clipboard and the headless clipboard for the same Action. [Clipboard port](../../crates/oxiview-platform/src/editor.rs)

Capture the ordered source revision at the time of Copy. Appends beyond the selected range preserve the rectangle. If a data update would change which records lie between its ends, apply the same collapse rule as sorting. Replay an offscreen selection through an append, a resize, a sort and a filter that removes the active row. Compare actual clipboard text, including multiline and quoted fields, after each action. [Keyed selection](../adr/0044-keyed-virtual-table-selection.md)

## Reading a local file

Use the M6 single-file Open panel. Read and index the file off the window thread and publish batches tagged with their source revision. Treat each plain-text line as a record. Preserve the original message when a timestamp or level cannot be identified. Keep a final unterminated line visible and update that record in place when more bytes arrive. Handle a UTF-8 character split across successive reads. A cancelled or failed Open preserves the current source and reading position. [Desktop service port](../../crates/oxiview-platform/src/editor.rs)

Watch the chosen path for appends, truncation and replacement. Continue from the consumed offset on an append. A truncation or replacement starts a distinct source generation so a selection from the old file cannot attach to an unrelated line at the same offset. Ignore stale reads after a source switch and surface a read error with its path. The workspace already confines the native watcher dependency to the composition root. Reuse that integration there and keep log parsing and file-following policy in the example. [Dependency allowlist](../../xtask/layers.toml) · [Existing file watcher](../../crates/oxiview-app/src/theme_file.rs)

## Framework and application

| Owner | Responsibility |
| --- | --- |
| `oxiview-widgets` | Build self-drawn controls from retained Views. Supply named theme types, keyboard Actions, enabled state, focus feedback and node semantics. Compose the list and table from the same scrolling and visible-range machinery. The widgets context is already registered in the dependency rules. [Layer rules](../../xtask/layers.toml) |
| `oxiview-platform`, macOS and headless | Carry logical scroll distance, pointer position, capture time, and the distinction between discrete wheel steps, direct gestures and momentum. Preserve the existing wheel entry point for callers and deterministic injection. [Window events](../../crates/oxiview-platform/src/interface.rs) |
| `oxiview-core` | Own each scroll viewport's offset, clipping, coordinate conversion, input routing and frame requests. Use the same geometry for painting, hit testing, hover, focus reveal and editor candidate rectangles. Reuse M2 pointer capture for the scrollbar thumb and M5 Actions for keyboard operations. [Retained painting and hit testing](../../crates/oxiview-core/src/ui/paint.rs) · [Editing geometry](../../crates/oxiview-core/src/ui/editable.rs) |
| `oxiview-layout` | Expose range and extent calculations through owned types. Keep row and column indexing independent of Taffy. A visible range limits how many row Views participate in retained layout. [Layout protocol](../adr/0009-layout-protocol-taffy-default.md) |
| `examples/log-viewer-oxi` | Keep record IDs, log parsing, search, severity filters, the follow setting and view selection in the example. The `.oxi` screen and Rust Actions exercise the same widget APIs available to other applications. [Run the viewer](../../examples/log-viewer-oxi/README.md) |
| `xtask` | Exercise the headless workloads and connect the reserved native scrolling metric to actual display feedback. Preserve prior machine measurements and compatibility with historical reports. [Performance protocol](../conventions/testing-performance.md) |

M6's Markdown editor already receives logical wheel distances and routes them to its two panes. Its source editor also reveals the caret. M7 can exercise a focused M6 text field inside a moving viewport to catch regressions in candidate placement and pointer selection. [M6 wheel handling](../../examples/markdown-editor-oxi/src/app.rs) · [M6 native session contract](../adr/0043-native-text-input-session-snapshots.md)

## Scroll and identity contracts

- A viewport clips its contents and passes unused distance per axis to the next enclosing viewport. When a child consumes 10 of a 24-unit update, its parent receives 14. The collection opts into `stationary_hover_updates` so hovering a row follows the visible geometry during scrolling. Use the existing capture and hover notification ordering. [Nested scrolling](../adr/0045-nested-scroll-distance.md) · [Stationary pointer hover](../adr/0030-stationary-pointer-hover-updates.md)
- Treat native momentum as part of the originating gesture. For an input source that delegates inertia to the framework, advance from the injected frame clock, stop on interruption or at the extent, and request frames only while motion changes the offset. A discrete wheel step moves by its delivered distance. Test a fresh gesture, a scrollbar drag, a hidden window and a reduced-motion change during motion. M8 owns general animation. [On-demand scheduling](../adr/0004-frame-scheduling.md)
- Keep record identity separate from the transient `NodeId` used to paint it. Scrolling a selected record offscreen can release its row View. Returning to that record restores its selected appearance. A captured pointer release or queued action for an old row cannot activate a different record.
- Limit mounted rows and cells to the visible range plus a bounded margin. Compute a fixed-height range directly from the offset and viewport. Use an indexed extent for variable-height content so measuring a visible row or jumping far down the list does not require laying out every earlier item. Preserve the first visible key and its offset within the row when heights above it change.
- A table keeps its column headers aligned with horizontal movement and visible during vertical scrolling. Intersect the column range with the horizontal viewport before building cells. Keyboard navigation to an offscreen item must reveal the item before the reader's next action.

## Implementation order

1. Mount a first reusable control in the registered `oxiview-widgets` crate and drive it by pointer and keyboard through ordinary Views. Use that control for the M7 widget recipe in the repository scaffolding guide. In parallel, extend the platform scroll events and build a nested viewport fixture with visible content and clickable children.
2. Implement the fixed and measured extent indexes, then mount a bounded window of stable row keys. Cover a jump to the last item, a width change that rewraps text, and updates to unloaded rows. Use the same geometry for scrollbars and for revealing a selected row.
3. Add table headers, column widths, two-axis cell virtualization and rectangular selection. Exercise the actual widget APIs in the `.oxi` application. Integrate filtering, file batches and Follow with the same collection revisions used by keyboard navigation and Copy.
4. Run the reader journey in a release headless replay, including a real temporary log and an injected clipboard and Open panel. Exercise the same application in a native window with the owner's trackpad, keyboard and system clipboard. Run the performance protocol and local verification, then record the results in this plan.

## Verification design

| Check | Evidence required for completion |
| --- | --- |
| Control behavior | Exercise pointer and keyboard activation, disabled controls, semantic role/value/Actions, focus traversal and theme overrides through ordinary retained Views. |
| Deterministic scrolling | Replay discrete wheel steps, a direct gesture with momentum, interruption, nested boundaries and thumb dragging. Compare visible content with pointer hits after fractional scrolling. Once settled, an inactive viewer produces zero submissions and presentations across 1,000 virtual display opportunities. |
| Virtualization | Use the same 100,000 record IDs in both views. Record mounted rows and cells, row construction, measurement and disposal while moving line by line and jumping to distant records. Exercise an empty result, one item, a partial last row and changes above the reading position. |
| Visual review | Produce CPU snapshots at 1×, 1.5× and 2× covering focus/selection, clipped rows, the scrollbar and the table header. Review the actual images before accepting the first baselines. [Snapshot conventions](../conventions/testing-snapshots.md) |
| Native performance | Calibrate display cadence in a release window. Drive continuous scrolling over the seeded list and verify that the displayed rows advance. Count missed refresh slots from Metal presentation feedback across the full measured interval. Keep each drawable ID, scroll position and presentation timestamp. Report resolution, scale, refresh interval, sample duration, expected slots, displayed frames and dropped slots. Incomplete feedback invalidates a run. [Native measurements](../../examples/window/src/measurement.rs) · [Presentation diagnostics](../adr/0036-fifo-drawable-readiness.md) |
| Sign-off | Record the owner's macOS review, the release headless reader journey and local `cargo xtask verify`. Publish the native scrolling measurements and track the remaining targets after delivery. |

`cargo xtask bench --m7` measures variable-height row lookup after a measurement update, one-row advances in a mounted 100,000-row list viewport, and diagonal movement in a wide virtual table. The collection fixtures bound mounted rows by the visible range plus its configured margin, and mounted cells by the product of the mounted row and column counts. They count construction and disposal as the viewport moves. A second dataset size with the same viewport checks that increasing the record count alone does not increase the mounted row count. [Collection tests](../../crates/oxiview-widgets/src/collection/tests.rs)

The native hard target uses the production viewer with its seeded data, text and ordinary rendering path. A two-second warm-up precedes ten seconds of continuous 12-point scrolling at the calibrated display cadence. Count all expected slots, including those at the start and end. A zero presentation timestamp or a missing slot fails the check. Missing callbacks or a changing refresh interval invalidate the sample. The native probe and owner review run after desktop readiness confirmation. [Capture and validation](../conventions/testing-performance.md#m7-virtualization-and-native-scrolling)

Historical baselines with a pending value remain readable. The native validator derives occupied slots from the raw feedback and checks the full sample, including zero timestamps, incompatible refresh intervals, missing callbacks and disagreement with the producer's summary. A first measured baseline requires a valid zero-drop result and the owner's approval. [Native validator](../../xtask/src/bench/scroll.rs) · [Measurement tests](../../xtask/src/bench/scroll_tests.rs)

## Acceptance record

M7 was accepted for delivery on 2026-11-24. The owner adopted the November 23 release report as the current performance record. The native review, three approved snapshots, 11 release replay checks, three collection benchmarks and seven local verification checks complete the delivery evidence. The two native scrolling goals carry forward as [performance follow-up](#native-scrolling-follow-up).

On 2026-11-21 the owner accepted the release build of Oxiview Logs in its native macOS window. The 100,000-row scrolling check uses Metal presentation timestamps. [Native scrolling protocol](../conventions/testing-performance.md#m7-virtualization-and-native-scrolling)

The owner accepted the expanded list at 1×, table selection at 1.5× and paused Follow at 2×. On the M2 Pro, the first window-free baselines were 165.4 ns for a variable extent update and lookup, 24.75 µs for advancing the virtual list by one row and 259.3 µs for moving diagonally through the table. All three passed a separate comparison. [Viewer snapshots](../../examples/log-viewer-oxi/tests/snapshots/) · [Machine baselines](../../perf/baselines/macos-8a09e0b60732277a.json)

The packaged release passed all 11 headless replay checks, and `cargo xtask verify` passed all seven local checks. The replay exercised a 100,000-row sample, fractional scrolling and interrupted momentum, variable-height expansion, two-axis selection and offscreen TSV Copy. A real append, a partial UTF-8 line, truncation, same-content file replacement and source switching ran through the file worker. Search used the M6 input session and waited for commit. The settled viewer submitted and presented zero frames over 1,000 virtual opportunities. The release CLI integration also checked that existing output files survived a new run. [Reader journey](../../examples/log-viewer-oxi/src/replay.rs) · [CLI integration test](../../examples/log-viewer-oxi/tests/release_replay.rs)

A release diagnostic at 2240×1440 physical pixels kept 18–19 list row Views mounted as the viewport moved through the 100,000-record source. Profiling identified repeated clipping of adjacent glyph draws. The Vello adapter now reuses the common clip prefix and the Scene encoder keeps a single clip stack during traversal. Thirteen sampled production frames retained identical GPU pixel hashes in the before/after comparison. [Adapter regression tests](../../crates/oxiview-render-wgpu/src/tests.rs) · [Scene encoding](../../crates/oxiview-scene/src/encoding.rs)

The viewer also drew fully transparent containers and consecutive opaque backgrounds covering the window. The GPU adapter now skips transparent solid fills and strokes, uses an attachment clear for leading full-window backgrounds and preserves rectangular clips through Vello. In two pairs of window-free release runs taken in opposite orders, the two original median times for Vello preparation and queue submission were 2.08 and 1.77 ms. The two optimized medians were 0.91 and 0.88 ms. Each run measured 1,200 frames at the same 2× resolution. All 13 sampled GPU pixel hashes matched the earlier release. [Vello adapter](../../crates/oxiview-render-wgpu/src/vector.rs)

The 256-control Tab workload initially exceeded the M5 baseline. Reusing the traversal order reduced its median to 205.4 ns against the original 1,607 ns baseline. Structural edits, focus policy, visibility and disabled state invalidate the cached order. All 22 M2–M6 scoped workloads passed their comparisons. The M6 Markdown release replay also passed after its wheel handler gained phased input. [Focus tests](../../crates/oxiview-core/tests/m5_focus.rs) · [M6 release replay](../../examples/markdown-editor-oxi/tests/release_replay.rs)

### Native scrolling on the built-in 120 Hz display

On the Mac14,10 with an M2 Pro, Oxiview Logs used windowed FIFO for both the application and its measurement. The release window measured 1120×720 points at 2×. After two seconds of warm-up, each submitted list frame advanced 12 points. The ten-second sample contained 1,200 expected refresh slots. Both runs below used AC power and an idle desktop. Each raw capture and its independent xtask verdict are under `target/xtask/log-viewer-oxi/`. [Viewer entry point](../../examples/log-viewer-oxi/src/lib.rs) · [Slot validator](../../xtask/src/bench/scroll.rs)

| Local capture | Occupied slots | Empty slots | Missing feedback | Zero timestamps |
| --- | ---: | ---: | ---: | ---: |
| `native-1795225742446882000` | 1,167 | 33 | 0 | 0 |
| `native-1795225818644440000` | 1,152 | 48 | 0 | 0 |

A separate Instruments Metal System Trace on battery recorded 57 empty slots. The display track showed no new composited surface at 51 of them. At the other six the compositor swapped a surface without a new viewer frame. Matching the displayed viewer frame numbers to the GPU execution track showed that, for every empty slot, the next displayed frame had finished its recorded GPU work at least 14.47 ms before that slot. The median lead was 26.33 ms. An isolated experiment used the drawable and deadline supplied by `CAMetalDisplayLink`. It submitted 1,200 frames and received 16 zero presentation timestamps. For those 16 frames, GPU completion preceded the estimated display time by at least 31.9 ms. The traces establish where the updates stopped appearing. The cause of each individual miss remains unresolved. [Apple display-link contract](https://developer.apple.com/documentation/quartzcore/cametaldisplaylinkdelegate/metaldisplaylink(_:needsupdate:)) · [Metal GPU completion time](https://developer.apple.com/documentation/metal/mtlcommandbuffer/gpuendtime)

The native baseline was still pending at the end of the November 21 campaign.

### November 23 measurement campaign

The owner selected stable 120 FPS and at most 1.2 ms of hitch time per second of active scrolling. Both the ordinary application and the probe now use the display link's actual drawable. The render command that writes the final pixels also requests presentation. The probe records OS display targets, the built-in display ID, the rendered leading row's geometry, and a fingerprint of the positioned glyphs of a fully visible row number. [Targets](../adr/0046-native-scroll-cadence-and-hitches.md) · [Presentation path](../adr/0047-present-display-link-drawables.md)

On the reference M2 Pro, the release with the bounded pending drawable was measured three times in sequence. The captures used the same executable (SHA-256 `aba85a7fd95b8d956fa9e6b611e328be022f28601b59751025f3a9a8f2cfbff5`). The first run filled all 1,200 slots. The independently validated recording run did so again and established the approved zero-drop baseline. The following comparison filled 1,199 slots. The capture directories under `target/xtask/` end in `1795406180330024000-85838`, `1795406194334976000-86165`, and `1795406208211710000-86614`. [Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json)

The earlier FIFO release measured 3.33 ms/s of app-attributed Apple hitch time. A subsequent display-link release measured 1.67 ms/s in a separate run, above the 1.2 ms/s target. In a diagnostic control, three 100 ms pauses of the window thread each produced a visible gap. Apple's detector reported a single hitch outside those pauses. The observer therefore requires independent Metal and display coverage before accepting a low Apple ratio. [Hitch check](../../xtask/src/bench/apple.rs)

The packaged release with explicit Metal layer configuration transactions had SHA-256 `17fd0c257cc524dcf785de5ea9bc41d38c4a17750ab30d6e5f4f45deecbc47a8`. Three consecutive untraced runs filled 1,189, 1,172 and 1,173 of the 1,200 slots. Two Apple recordings of the same package followed. In the first, two correlated app hitches totaled 37.50 ms over ten seconds, giving a lower bound of 3.75 ms/s. The second contained no correlated hitch event, but Metal showed 16 empty slots and 1,184 positive presentation times. These runs leave both targets open. The native capture directories under `target/xtask/` end in `1795412117632788000-2196`, `1795412132639950000-3227` and `1795412146688386000-3927`. The Apple traces end in `1795412221837519000-7034` and `1795412267032026000-8021`. [Measurement protocol](../conventions/testing-performance.md#m7-virtualization-and-native-scrolling)

In each of those Apple traces, two positive Metal presentations had an Apple Display swap, queue event and VSync, but no matching Hitches frame lifetime. The missing pairs were exactly 600 active slots apart. A separate AppKit control rendered a moving bar in a 2240×1440 Metal drawable on every display-link update. It submitted all 1,200 target frames in each run. At the two supported frame-latency settings, 20 and 39 of the corresponding drawables received a zero presentation time. Metal reported GPU completion at least 6.79 ms before the CPU deadline on all 59 of them. This control isolates the cost of drawing the log rows from other causes of missed presentation. [Apple display-link callback](https://developer.apple.com/documentation/quartzcore/cametaldisplaylinkdelegate/metaldisplaylink(_:needsupdate:))

An app-scoped XCTest diagnostic also measured the viewer, a deliberately late AppKit transaction and three deliberate window-thread stalls. `XCTHitchMetric(application:)` returned 0 ms/s for all three. The scored iterations occupied 1,200, 1,142 and 1,141 slots respectively. Its aggregate could not distinguish the controls from the ordinary scroll on this machine. [Apple's XCTest measurement example](https://developer.apple.com/videos/play/wwdc2020/10077/)

After rebuilding the release from the main checkout, three consecutive untraced runs displayed **1,192, 1,160 and 1,186 of 1,200 slots**. The signed executable had SHA-256 `44362d8e8ed2af6e7c804da6f7fbb80afcc3e3f997f6fe8cdaeec4e902efee00`. The OS clock covered 1,200, 1,199 and 1,200 nominal slots respectively. Every positive presentation in those runs matched its assigned display tick. The native capture directories under `target/xtask/` are `bench-m7-native-scroll-1795426784232889000-92009`, `bench-m7-native-scroll-1795426798428353000-92754` and `bench-m7-native-scroll-1795426812565114000-93490`.

Two subsequent recordings of that executable each had 8.33 ms of confirmed Apple hitch time during the ten-second scroll. The first displayed only 751 slots and had one additional hitch with unresolved attribution. The second displayed 1,162 slots and lacked two Apple Hitches frame lifetimes. An additional target-scoped table from Apple's Core Animation modeler disagreed with four known positive comparison events in that recording. The observed 0.83 ms/s in each trace is a lower bound. [Published measurement data](../performance/runs/1795426783-32f6c6d/m7-scroll.json)

### Native scrolling follow-up

| Goal | November 23 result | Evidence needed to close |
| --- | --- | --- |
| Stable 120 FPS | The three untraced runs reached 119.2, 116.0 and 118.6 FPS. They missed 8, 40 and 14 of 1,200 slots. | Reproduce 1,200 advancing presentations in 1,200 slots in an independent release comparison. |
| Average Apple hitch time ≤ 1.2 ms/s | Both ten-second traces confirmed at least 0.83 ms/s. Incomplete attribution leaves the total unresolved. | Establish the ratio across the full scroll with Apple events corroborated by Metal presentation and display records. |

Both items are post-delivery performance work. The scoped benchmarks continue to report the measured verdict for each goal. [Measurement protocol](../adr/0046-native-scroll-cadence-and-hitches.md)
