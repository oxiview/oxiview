---
status: done
milestone: M4
---

# M4 real text and the Snake demo

The owner requested a complete playable Snake demo using M3's expression syntax
and reactive components together with real M4 text. Implementation proceeds
under [ADR-0010](../adr/0010-pure-rust-text-stack.md); a visual game is the
integration target, not a replacement for the remaining text/rendering scope.

## Text and renderer boundary

- `oxiview-text` owns paragraph styles, immutable rich text and positioned
  glyph draws. Parley and fontique shape, wrap, align and resolve fonts.
  Swash rasterizes hinted small text at its actual display scale. Large
  outlines carry their font and placement into the renderer.
- Immutable RGBA image data and pixel dimensions are shared owned values in
  `oxiview-base`. Text does not depend on Scene or rendering adapters. Core
  converts text draws into Scene primitives.
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
- The native presenter and the offscreen GPU renderer share Vello and a
  device-owned glyph atlas. Repeated draws can sample a resident image across
  frame submissions. [GPU backend and atlas](../adr/0038-vello-gpu-and-glyph-atlas.md)
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
cargo xtask bench --m4
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

## Display and performance review

The owner signed off Snake's steering, pause, restart, scoring and keyed
history during [M3 acceptance](m3-reactivity-and-components.md#acceptance-record).
M4's native review covers the appearance of the final text renderer at the
display scale, including small labels, large glyphs, and text after a resize
or fullscreen transition.

| M4 deliverable | Observable acceptance |
| --- | --- |
| Rich text | Unicode spans support typeface, size, weight, italic, foreground, underline, strikethrough, character and word spacing, and line height. Geometry-affecting edits remeasure a mounted node. Decoration and color edits repaint it. |
| Text baselines | Flex and Grid align the first lines of differently sized labels, including wrapped text and a per-item alignment override. [M2 layout commitment](m2-node-tree-and-layout.md#typed-property-subset-and-units) |
| Hybrid glyph rendering | Small labels keep hinted glyph images. Large or rotated outlines stay sharp at their final display size. The CPU and GPU adapters preserve placement, clipping and opacity. [ADR-0010](../adr/0010-pure-rust-text-stack.md) |
| Color glyphs | A pinned COLRv1 font exercises gradients, transforms, clips and compositing through the real text pipeline. Embedded bitmap emoji retain their colors. |
| GPU rendering and residency | The native and offscreen adapters execute the Vello path on the shared device. A repeated frame reuses uploaded glyphs. Eviction and device recreation preserve the rendered result. [ADR-0012](../adr/0012-scene-abstraction-wgpu-vello.md) |

For performance review, use fixed fonts and the existing median-of-three
protocol to measure paragraph shaping and wrapping, styled spans, and cold
and warm small-glyph raster work. Report the workload, scale and cache state
with each number. The workspace default is a 10% relative regression threshold.
[Measurement conventions](../conventions/testing-performance.md#measurement-and-parsing)

Build `examples/snake/package-macos.sh` to produce the local development bundle
at `target/xtask/snake/Oxiview Snake.app`. Native visual review requires an
unlocked display session; do not label headless rendering as that review.
Review font clarity, controls, resizing, fullscreen, interruption recovery,
and idle behavior after opening the app.

The deterministic display specimen uses pinned fonts to render raster body
text, outline digits aligned with a smaller label, styled Latin and Arabic
text, and two COLRv1 glyphs. It produces separate CPU snapshot candidates at
1×, 1.5× and 2×. The owner reviews those images and the measured text
benchmarks before recording baselines. [Display specimen](../../crates/oxiview-app/tests/m4_text_display.rs)

## Acceptance record

On 2026-11-20 the release replay for both the Rust and `.oxi` versions reached 30 points
with one screen mount and zero rejected input commands. In the ready screen
on the development Mac, all 266 painted glyphs were raster images at 1×.
At 2× the same screen contained 259 raster images and seven vector glyphs.
The comparison includes ink from the outline glyphs in its text mask.
All eight CPU/GPU scenes passed. At most 0.46% of the compared text pixels
exceeded a four-code channel delta, against the existing 4% limit.
[Replay counters](../../examples/snake/src/replay.rs) · [CPU and GPU comparisons](../../examples/snake/tests/rendering.rs)

The owner accepted eight pixel baselines on 2026-11-20. Three cover the new
text specimen and three cover M2 translucency at 1×, 1.5× and 2×. The other
two capture the CPU gradient and the rounded edge after the linear-light
correction. All eight stored PNGs match the reviewed candidates byte for byte.
[Application snapshots](../../crates/oxiview-app/tests/snapshots) · [CPU renderer snapshots](../../crates/oxiview-render-cpu/tests/snapshots)

Local `cargo xtask verify` passed all seven checks after the baselines were
accepted. The workspace ran 443 passing tests with zero failures. The existing
`view!` documentation example is ignored.

The M2 and M3 scoped comparisons passed all twelve workloads. With the
owner's approval, the scoped M4 recording added four metrics to the Mac14,10
baseline. The immediate window-free comparison passed all four at the 10%
regression threshold. [Machine baseline](../../perf/baselines/macos-8a09e0b60732277a.json) · [Benchmark protocol](../conventions/testing-performance.md#m4-text-benchmarks)

| Workload | Recorded | Immediate comparison |
| --- | ---: | ---: |
| Shape and wrap the plain paragraph | 25.43 µs | 24.91 µs |
| Shape and wrap the styled paragraph | 26.36 µs | 27.10 µs |
| Rasterize the score label at 2× with a cold cache | 70.28 µs | 71.65 µs |
| Prepare the same label with warm glyph images | 1.799 µs | 1.778 µs |

On 2026-11-20 the owner reviewed the release build of Snake in its native
macOS window and confirmed that the text was clear. This completed the M4
display review.
