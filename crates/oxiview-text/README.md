# oxiview-text

Language: **English**

Owned text-context APIs for Unicode shaping, font fallback, paragraph wrapping,
and DPI-aware glyph rasterization. Parley/fontique/Harfrust provide logical
layout; Swash rasterizes glyphs into immutable shared foundation images. Core converts the returned
`GlyphImage { rect, image }` records into Scene image primitives; the text
crate has no dependency on the Scene drawing language. The native
GPU renderer samples glyph pixels from the Scene's encoded command buffer,
with the same transform, clip, and group-opacity semantics as other primitives.
The CPU renderer supports the same image primitive for headless runs.

Keep one `TextSystem` per window. `layout(text, &style, max_width)` returns an
owned `TextLayout`; `size()` is logical geometry for intrinsic measurement.
`glyph_images(&layout, local_origin, display_scale)` returns owned positioned glyph images
at the actual display scale. A scale change needs new glyph primitives, while
the logical layout can be reused. `TextStyle` supports family, weight, size,
color, line height, and direction-aware paragraph alignment.

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

The current implementation emits glyph images. It does not yet implement the
dedicated GPU texture atlas, large-text vector path, arbitrary transformed vector glyphs, complete COLRv1
painting, or a Vello GPU backend migration from ADR-0010/0012. This library and
the Snake demonstration establish working font text without claiming those
remaining M4 capabilities or editable-text behavior from later milestones.
