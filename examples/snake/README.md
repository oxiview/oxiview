# Snake

**English**

A native game exercising oxiview's retained tree, Signals, function components,
optional `view!` syntax, scoped bindings, keyed lists, and real font rendering.
The model is independent of UI nodes and uses injected randomness and a fixed
simulation clock. Text is shaped with Parley and rasterized into cached glyph
images; the native renderer composites those images on the GPU.

## Run

From the repository root, on macOS 14+ with a visible Metal-capable display:

```sh
cargo run --release -p snake
```

Build a local application bundle for direct opening:

```sh
./examples/snake/package-macos.sh
```

The output is `target/xtask/snake/Oxiview Snake.app`. This is a local development
bundle; distribution signing/notarization remains in its later milestone.

## Live diagnostics

Enable the optional top-right overlay at startup:

```sh
cargo run --release -p snake -- --debug-overlay
open -n "target/xtask/snake/Oxiview Snake.app" --args --debug-overlay
```

APP FPS counts successful application frame submissions over the last second.
The panel also shows the backend's display cadence,
CPU event-processing time, CPU paint-to-submission time, live node and draw
counts, cumulative layout/measurement/cache statistics, and logical viewport
size/DPI. Paint-to-submit starts when application painting begins, after any
ordinary drawable wait. At Classic pace the initial simulation step is 125 ms,
so steady play typically submits about eight frames per second. To measure
actual Metal display times under continuous motion, use the
[continuous animation demo](../animation-fps/README.md).

The layer is disabled by default, draws above the Scene, does not participate in
hit testing, and never changes the game's retained layout. Labels update on
application frames with a 200 ms formatting limit. After activity stops, one
final diagnostic frame shows zero FPS, then no more refreshes are scheduled;
that frame is excluded from application FPS. Hidden/inactive windows cancel
the diagnostic deadline. Use the overlay for inspection, with it disabled for
comparable performance measurements.

`--headless --debug-overlay` exports the panel for visual checks; static images
do not measure live FPS or display cadence.

## Controls

| Input | Action |
| --- | --- |
| Enter | Start a ready game; otherwise start a new run |
| Space | Start, pause, resume, or replay according to the current state |
| Escape | Pause an active game |
| Arrow keys or WASD | Steer while playing |
| Primary green button | Start / Pause / Resume / Play again |
| Direction buttons | Steer while playing |
| New game | Restart immediately, retaining the session best score |
| Chill / Classic / Swift | Choose pace before starting or after finishing |

Each food is worth ten points. The snake grows by one cell and gets gently
faster every five foods. Reversals are rejected and at most two turns are
buffered, so rapid perpendicular turns do not accidentally reverse direction.
Filling the board wins. Best score and the three most recent completed runs
are session data; the example does not write user preferences or saved games.

Losing window activation or visibility pauses gameplay. Returning to the window
waits for explicit resume. A long display-clock gap also pauses the game instead
of fast-forwarding an unseen run. Ready, paused, and finished states schedule no
periodic game work.

## Headless acceptance

No visible display is required for the CPU replay:

```sh
cargo run --release -p snake -- --headless --json
cargo test -p snake
```

`--seed N` selects a reproducible food stream. `--output DIRECTORY` selects the
headless artifact directory. Defaults write to `target/xtask/snake/`:

- `ready.png` and `ready@2x.png`: actual font rendering at two display scales.
- `playing@2x.png`: a run driven by keyboard inputs that collected three foods.
- `paused.png`, `game-over.png`, and `compact.png`: lifecycle and resize states.
- `report.json`: executed checks, glyph-image evidence, component mounts,
  history updates, rejected-command counts, and retired automatic bindings.

The manifest is marked incomplete before work starts and failed if a replay
fails. A previous successful report cannot certify a partially overwritten run.
These images are review artifacts, not automatically accepted golden snapshots.

The replay exercises real controls and bindings. Additional window-loop tests
use the actual headless platform and virtual clock to verify zero idle frames,
timed steps, pause/resume, activation loss, and task wake behavior. Model tests
cover collision, tail movement, full-board victory, food-source errors, and
arbitrary turn sequences. Existing local project checks remain authoritative:

```sh
cargo xtask verify
```

## Native review

After unlocking the display, check normal and high-DPI text; start with mouse
and keyboard; collect food and turn quickly; pause/resume; change pace between
runs; resize; leave and return to the window; and verify restart/history. Keep
the window idle to confirm no periodic activity. Native fullscreen, display
transitions, and benchmark-baseline acceptance are separate checks.

See [M3 implementation](../../docs/plans/m3-reactivity-and-components.md) and
[M4 text and Snake](../../docs/plans/m4-text-and-snake.md) for framework scope
and remaining milestone work.
