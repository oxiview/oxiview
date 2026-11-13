---
status: accepted
---

# Compositing: texture layers sharing a GPU device and embedded native child views

External content enters in two ways:

1. **Texture layers** (preferred): the framework exposes its wgpu device to applications. Applications use that same device to render 3D scenes, video frames, or custom content into textures. Each texture participates in UI composition as a compositing layer, supporting clipping, transforms, UI overlays, and continuous layer rendering (ADR-0004).
2. **Embedded native child views**: system components that cannot render into textures, such as system maps or DRM-protected video players, embed as native views (NSView / child HWND / wl_subsurface). The framework synchronizes position, size, visibility, and clipping.

## Consequences

- Native child views have the classic airspace limitation: they always sit above self-drawn content, cannot be covered by UI or blended translucently, and cannot participate in transform animations. Document this explicitly in APIs and documentation. For example, a menu above such a view needs a separate popup window.
- Texture layers and a shared device require applications to use the same wgpu version, making it part of the public API.
