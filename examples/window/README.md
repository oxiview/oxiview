# M0 window example

From the repository root:

```sh
cargo run -p window
cargo run -p window -- --headless
```

The first command requires macOS, a display session, and a Metal adapter. It
renders a retained scene offscreen with the GPU renderer, then presents its
straight-RGBA8 pixels through the temporary platform port. Close the window to
return from presentation and exit. The window is fixed-size; display-scale
handling and the real platform API belong to M1.

The headless command renders the same scene at 640 × 420 with the CPU renderer
and writes `target/xtask/window.png` at the repository root. It does not initialize
the GPU or AppKit. The scene contains a gradient rounded rectangle and a
partly transparent overlapping amber rounded rectangle on a dark background.

The example is a workspace member, so `cargo xtask verify` formats, lints, and
tests it. As an application it is exempt from layer rules in `xtask/layers.toml`.
