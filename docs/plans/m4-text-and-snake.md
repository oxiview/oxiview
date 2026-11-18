---
status: in-progress
milestone: M4
---

# M4 real text and the Snake demo

The owner requested a complete playable Snake demo using M3's expression syntax
and reactive components together with real M4 text. Implementation proceeds
under [ADR-0010](../adr/0010-pure-rust-text-stack.md); a visual game is the
integration target, not a replacement for the remaining text/rendering scope.

## Text and renderer boundary

- `oxiview-text` owns `TextSystem`, `TextStyle`, `TextLayout`, and positioned
  `GlyphImage` values. Parley/fontique shape, wrap, align, and resolve fonts;
  Swash rasterizes glyphs at their actual display scale.
- Immutable RGBA image data and pixel dimensions are shared owned values in
  `oxiview-base`. Text does not depend on Scene or rendering adapters. Core
  converts glyph-image records to Scene image primitives.
- Text nodes measure intrinsic content under layout constraints. Changes to
  text invalidate the relevant layout and paint work. Unchanged text reuses
  cached layout/raster data; DPI changes keep logical geometry while
  rerasterizing glyphs. Rasterization uses the absolute content origin's
  physical pixel phase, then converts glyph rectangles back to node-local
  coordinates. Whole-pixel moves reuse glyph images; fractional moves update
  their phase instead of bilinear-translating an incorrectly aligned bitmap.
- The bounded glyph cache defaults to 4,096 entries and 8 MiB. Zero-coverage
  whitespace masks do not create image primitives. Spaces retain their shaped
  advances without uploading transparent 1x1 raster artifacts.
- Both CPU and GPU adapters support shared image primitives with scaling,
  clipping, and opacity. The native path uploads glyph/image data and composites
  it on the GPU; it does not CPU-rasterize the entire application window.
- Dependency versions, licenses, and pinned font fixtures are documented in
  [dependency conventions](../conventions/dependencies.md) and the text crate's
  fixture manifest.

## Playable integration target

The [Snake example](../../examples/snake/README.md) includes mouse controls,
arrow/WASD input, start/pause/resume/restart, selectable pace, food, growth,
collision, full-board victory, score/level/session best, and three keyed recent
runs. All labels, scores, status messages, and direction arrows use real text.
The application uses ordinary Rust game state plus Signals, function components,
props, `view!`, Show, keyed For, and owned node handlers.

The board updates on a fixed simulation interval rather than an always-running
display loop. Ready, paused, and finished states cancel the next tick. Window
activation/visibility loss pauses gameplay; restoration waits for explicit
resume. A long clock gap pauses rather than fast-forwarding an unseen run.

Snake's optional `--debug-overlay` counts application submissions. At Classic
pace the initial step is 125 ms. The [continuous animation demo](../../examples/animation-fps/README.md)
exercises the same native presenter under a new frame request after each
submission and measures actual displayed FPS from Metal feedback.

## Automated acceptance

Run from the repository root:

```sh
cargo test -p snake
cargo run --release -p snake -- --headless --json
cargo xtask verify
```

The replay drives actual pointer and keyboard events, collects three foods,
checks scoring/growth, pauses through 1,000 virtual frame opportunities,
checks display-gap and activation recovery, reaches game over, and restarts
while checking keyed history identity and a single screen mount. It writes
ready/playing/paused/game-over/compact images and an explicit pass/fail manifest
under `target/xtask/snake/`. A new run invalidates the old manifest before any
images are replaced.

Separate real headless-window tests exercise scheduler deadlines and verify
zero additional frames/presentations/board updates during extended idle states.
Font fixtures cover shaping, combining marks, fallback, wrapping, weights,
DPI, and cache reuse. Image adapter tests cover CPU/GPU alpha, clipping, scale,
and restoration of surrounding drawing state. Whole-game CPU/GPU comparisons
exercise the actual application Scene rather than only synthetic rectangles.

## Review and remaining work

Build `examples/snake/package-macos.sh` to produce the local development bundle
at `target/xtask/snake/Oxiview Snake.app`. Native visual review requires an
unlocked display session; do not label headless rendering as that review.
Review font clarity, controls, resizing, fullscreen, interruption recovery,
and idle behavior after opening the app.

The delivered text path targets readable small and medium UI text through
cached glyph raster images. Large-text vector glyph paths, complete COLRv1
coverage, full Vello backend migration, and the complete rich-text API remain
open M4 scope. Editable text/IME, focus/Action mapping, widgets, and animation
remain later milestones. Snapshot baseline acceptance and performance recording
follow their existing human-review gates; generated replay images are not
automatically accepted baselines.
