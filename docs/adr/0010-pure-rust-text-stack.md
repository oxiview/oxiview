---
status: accepted
---

# Text: a pure Rust Parley-based stack, hybrid rasterization, and an owned editing core

The **text stack** is entirely Rust: fontique for font discovery and fallback, harfrust for shaping, Parley for paragraph layout and rich text, skrifa/swash for glyph outlines and rasterization, and ICU4X for line breaking, graphemes, and bidirectional text. Shaping, line breaks, and metrics remain identical across platforms, and issues can be fixed within the stack. This deliberately avoids GPUI's platform-native text-stack approach (CoreText / DirectWrite / FreeType).

Use **hybrid glyph rasterization**, choosing a path by each glyph's **final display size** (logical font size × scale × DPI). Small text is rasterized on the CPU into a GPU **glyph atlas**, with hinting and subpixel positioning for readable body text. Large text above a threshold, and text with complex transforms such as rotation or perspective, uses **vector paths**. This avoids pixelation at arbitrary scales and unbounded atlas growth. Scale animations can transition smoothly between paths; the threshold is configurable and the atlas uses LRU eviction. Handle color emoji (COLRv1 / bitmap) separately.

The framework's own **editing core** provides **editable text**: grapheme-level cursors, selection, bidirectional cursors, IME composition, and undo coalescing, exposed to widgets through an editor protocol. Single-line and multiline inputs build on it. Large-document use cases such as code editors can replace text storage, for example with a rope.

## Considered Options

- **Platform-native text stacks**: closer to system rendering, but metrics, line breaks, and fallback vary across platforms and are harder to fix consistently.
- **cosmic-text**: simpler, but weaker rich-text and layout capabilities than Parley.
- **All-vector rendering**: ideal scaling, but inadequate clarity and performance for small text.

## Consequences

- Query system font directories and user font settings directly so fallback chains approximate system behavior.
- Upstream crates such as Parley are evolving; pin versions and maintain a patch queue.
- The editing core is among the largest and riskiest parts of the framework. Validate it early in prototyping, after layout and before complex widgets.

## M4 implementation

Parley shapes text in logical coordinates. UI text painting supplies the
local-to-device affine to the text port. Small outlines use Swash hinting and
quarter-pixel phases at the final display scale. The default cutoff is 64
device pixels per em, configurable through `TextSystem::with_vector_threshold`.
Outlines above that cutoff and outlines with rotation, shear, reflection, or
nonuniform scaling become unhinted Scene glyphs. Synthesized bold and oblique
outlines use the Scene's filled Bézier path. Bitmap-only faces retain their
available strikes and the advance of unpainted spaces. COLR glyphs reference
their font's paint graph at all sizes.
The `glyph_images` API remains available for callers that need Swash raster
images.

The CPU adapter draws unhinted outlines through Skrifa and reads COLR
paint tables through Skrifa's Fontations dependency. Vello CPU rasterizes glyph
clip coverage and supplies the blend operators. Each color glyph blends and
interpolates premultiplied linear-light values in viewport-clipped tiles. The
adapter applies glyph opacity once and passes these float tiles to the linear
compositor. The finished frame is encoded as straight sRGB. Keeping the original color stops and
geometry handles empty and single-stop lines, hard stops, and variable stop
offsets and alpha. Headless pixel tests cover transformed gradients, sweep
orientation, alpha, compositing, palette mapping, and retained clip and opacity.
[OpenType COLR](https://learn.microsoft.com/en-us/typography/opentype/spec/colr) ·
[OpenType CPAL interpolation](https://learn.microsoft.com/en-us/typography/opentype/spec/cpal#interpolation-of-colors)

[M4 text and Snake](../plans/m4-text-and-snake.md)
