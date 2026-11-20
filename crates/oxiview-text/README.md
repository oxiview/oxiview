# oxiview-text

Language: **English**

Parley, fontique and Harfrust shape and wrap Unicode text in logical units.
Keep one `TextSystem` per window. Call `layout` for a plain paragraph or
`layout_rich` for styled spans. The returned `TextLayout` exposes logical
size, first-line baseline and counts of lines and glyphs. `rewrap` reuses
shaping at a different width.

`RichText::new` takes the full paragraph and a list of `TextSpan` values.
Each span specifies a nonempty UTF-8 byte range and per-span style. Spans can
arrive out of order and must have disjoint ranges. Unstyled text inherits
the paragraph's family, size, weight, foreground and line height. Spans can
override the line-height multiplier, add logical letter or word spacing,
and enable underline or strikethrough in the span's foreground. The
`rich_text` view and `bind_rich_text` apply the same API to a retained node.
Flex and Grid rows can align the first lines of differently sized labels
with `Alignment::Baseline`.
[Rich text types](src/rich.rs) · [Text view](../oxiview-core/src/ui/text.rs)

`glyph_draws` takes a local origin and the complete local-to-device affine.
It selects hinted images for small glyphs and vectors above 64 device pixels
per em or under complex transforms. Set the cutoff with
`with_vector_threshold`. Color glyphs retain their OpenType paint graph or
embedded bitmap strike. `TextLayout::decorations` returns logical rule
rectangles for the positioned runs. Core paints underlines, glyphs, then
strikethroughs in the Scene. Both renderers apply the resolved transforms,
clips and group opacity. The `glyph_images` method also supports callers that
explicitly need Swash raster images.
[GPU backend and atlas](../../docs/adr/0038-vello-gpu-and-glyph-atlas.md)

The default cache retains at most 8 MiB of glyph pixels and 4,096 entries using
LRU eviction. Pixel storage already referenced by a Scene remains alive after
cache eviction. `set_cache_budget` and `cache_stats` expose owned controls and
counters. A font is rasterized at its final display size with grayscale
antialiasing, hinting, and quarter-pixel phase keys. Native bitmap/color-outline
glyphs retain their RGB; monochrome glyphs receive the style color.

`without_system_fonts` and `register_font` support deterministic font databases.
The [test fixtures](tests/fonts/README.md) include unmodified, licensed Noto
fonts. Tests cover combining marks, ligatures, Arabic fallback, wrapping,
variable weights, DPI changes, cache eviction, and invalid inputs. The macOS
system-font test additionally covers Chinese and a color emoji without opening
a window.

The original color fixture exercises color gradients, clipping, blending and
embedded strikes. The application-level specimen exercises styled Latin and
Arabic text at three display scales. [Font fixtures](tests/fonts/README.md) ·
[M4 display tests](../oxiview-app/tests/m4_text_display.rs)

## Editable text

`Editor` tracks UTF-8 source, selection affinity, undo history, and an optional
native input session. `source` and `committed_revision` feed preview and file
consumers. `presentation` includes inline preedit and has its own revision.
Retained fields use `TextSystem::layout_editable` for paragraph shaping,
selection rectangles, visual navigation, and glyph drawing. Updating a layout
reuses unchanged paragraph shapes. [Editor model](src/edit.rs) ·
[Retained editor](../oxiview-core/src/ui/editable.rs)

M6 reuses the already-resolved ICU4X `icu_segmenter =2.3.0` as a direct text-port
dependency. Extended grapheme and dictionary word boundaries support editing
before a font or layout exists, while Parley supplies the geometry for shaped
text. The pure Rust segmenter requires Rust 1.88 and uses the workspace's
existing Unicode-3.0 license branch. The crate inherits the exact pin with
default features disabled and compiled data enabled. The lockfile reuses the
version brought in by Parley and adds no new ICU package. The APIs expose byte
offsets and owned geometry. [Published manifest](https://docs.rs/crate/icu_segmenter/2.3.0/source/Cargo.toml) ·
[ICU4X release history](https://github.com/unicode-org/icu4x/releases)
