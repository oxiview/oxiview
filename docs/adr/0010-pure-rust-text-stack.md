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
