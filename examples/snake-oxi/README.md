# Snake in .oxi

**English**

Play the same native [Snake game](../snake/README.md) with Rust code for
application logic and precompiled `.oxi` files for views and typed styles.
Both examples use the same game model, simulation clock, input actions, board
painter and text renderer.

```sh
cargo run --release -p snake-oxi
cargo run --release -p snake-oxi -- --headless --json
cargo test -p snake-oxi
```

Enter starts a game, arrows or WASD steer, Space pauses or resumes, and Escape
pauses. The mouse controls, pace choices and optional `--debug-overlay` also
work. The native window currently runs on macOS. The headless replay accepts
`--seed` and `--output` and writes six images and `report.json` under
`target/xtask/snake-oxi/` by default.

Build a local macOS application bundle with `./examples/snake-oxi/package-macos.sh`.
The result is `target/xtask/snake-oxi/Oxiview Snake Oxi.app`.

## Read the source

- The [entry point](src/main.rs) starts the shared game loop. The
  [crate root](src/lib.rs) imports the view and style modules.
  The [screen](src/screen.oxi)
  places the board and sidebar and binds the phase badge.
- The [board](src/board.oxi) attaches the shared painter and mounts the
  conditional overlay. Rust [message functions](src/messages.rs) format its
  title and explanation for each game phase.
- The [sidebar](src/sidebar.oxi) binds scores and reconciles recent runs by
  their stable keys.
- The [Rust controls](src/controls.rs) create scoped press state. The
  [button fragment](src/button_face.oxi) uses the local binding, mount point
  and pointer handler. A [style-only module](src/styles.oxi) defines the
  typed pressed appearance.

The [parity test](tests/parity.rs) drives the original Rust and `.oxi` screens
through the same headless replay and compares every output image and counter.
The [M3+1 plan](../../docs/plans/m3-plus-1-oxi.md) describes the compiler's
entry points and equivalence contract.
