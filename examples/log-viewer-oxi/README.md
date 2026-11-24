# Oxiview Logs

[English](README.md)

Open the native macOS viewer with a reproducible stream of 100,000 records, or open an existing text log by path. The window follows a file selected through Open as another process appends to it.

```sh
cargo run --release -p log-viewer-oxi
cargo run --release -p log-viewer-oxi -- path/to/application.log
```

The sample starts at the first record. A newly opened file starts in Follow mode. Scrolling back pauses following and keeps the current record in place. **Resume follow** returns to the newest matching record in arrival order. The notice counts matching arrivals, including when a writer completes one line in several writes.

| Reader action | Interaction |
| --- | --- |
| Find an entry | Press Command+F and type a literal substring. Select severity checkboxes to narrow the results. Choosing an IME candidate commits the search. |
| Inspect a message | Click its list row to see the original text in the detail pane. Click the disclosure arrow or press Enter on the active row to expand it inline. |
| Navigate the table | Select Table and click a data cell. Use the arrows, Home, End or Page Up/Down to move. Shift extends a rectangular selection, including beyond the viewport. |
| Copy | Select the numbered row header and press Command+C for the original log line. A data-cell selection copies tab-separated values in displayed order, quoting tabs, newlines and embedded quotes. |
| Change columns | Click a header to sort. Click again to reverse it. Drag its right edge to resize it. Sorting or filtering collapses a rectangle to its active cell if it still exists. |
| Follow a different file | Press Command+O. Cancelling or failing to open the new file keeps the current source. Replacing or truncating the watched file starts a new generation of records. |

Plain-text files use one record per line. The viewer recognizes a leading timestamp, level and optional `component/thread:` prefix, such as `08:41:00.250 [WARN] storage/io: retrying checkpoint`. Other text stays readable in full and uses the Info level for filtering.

## Replay and measurement

```sh
cargo run --release -p log-viewer-oxi -- --headless --json
cargo xtask bench --m7
```

The headless replay writes `replay.json` and screenshots in a unique subdirectory of `target/xtask/log-viewer-oxi/`. Use `--output DIRECTORY` to choose its parent. The replay drives the native application's event loop with the headless platform and CPU presenter. The three scoped benchmarks measure viewport work without opening a window.

After confirming desktop readiness, use `cargo xtask bench --m7-native` for displayed 120 FPS and `cargo xtask bench --m7-hitches` for Apple hitch time on the 100,000-row scroll. The capture warms up for two seconds and measures ten seconds of continuous list movement. The capture directory named in `target/xtask/bench-m7-native-results.json` holds `viewer.json` with raw drawable IDs, painted row geometry, offsets and Metal timestamps. To build a local macOS application bundle, run `examples/log-viewer-oxi/package-macos.sh`. [M7 design and acceptance](../../docs/plans/m7-widgets-and-scrolling.md) · [Measurement protocol](../../docs/conventions/testing-performance.md#m7-virtualization-and-native-scrolling)
