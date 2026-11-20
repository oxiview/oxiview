---
status: accepted
---

# Rendering: owned Scene abstraction, wgpu + Vello backend, SDF fast path, and linear color space

The framework defines its own **Scene (Display List)** as the sole interface between node-tree painting and concrete renderers. The main backend runs Vello on wgpu for arbitrary vector paths, gradients, clipping, and vector text (ADR-0010). Common UI primitives such as rounded rectangles, borders, and shadows use an **SDF fast path**, computed analytically in shaders. Environments without compute shaders (older GPUs, virtual machines, remote desktops) fall back to vello_cpu / tiny-skia. One codebase covers Metal, D3D12, and Vulkan.

Color: blend internally in **linear space**, support Display P3 wide gamut, and reserve HDR (extended dynamic range) in the interface. Gradient interpolation color space is selectable.

## Considered Options

- **Native graphics backends per platform** (GPUI style): best latency and size, but requires three shader and resource-management implementations. Scene preserves the option to replace a backend for a specific platform later.
- **Skia**: most complete, but depends on C++, has a large footprint, and conflicts with the preference for pure Rust.

## Consequences

- The owned platform layer (ADR-0002) must hand native surfaces (CAMetalLayer, HWND/DXGI, wl_surface) to wgpu directly.
- Theme and style colors carry color-space semantics, defaulting to sRGB.

## Notes

- 2026-11-13 (M0): The CPU path adopted `vello_cpu` (pinned `=0.4.0`, confined to `oxiview-render-cpu`) and is locked by pixel snapshots. An offscreen wgpu renderer with a process-wide device arrived ahead of M1; it rasterizes the shared Scene encoding with its own compute shader, and Vello itself was deferred to M4, when text and arbitrary paths need it; M1 adds only the SDF fast path (decided while planning M1). GPU pixels are not required to match the CPU path byte for byte. Presentation blits pixels into the window until M1 introduces a `wgpu::Surface`.
- The first Snake integration used Scene images for all glyphs. M4 added Vello GPU for general scenes, retained raster glyphs in a device-owned atlas and kept the analytic SDF fast path for eligible frames. [M4 implementation](../plans/m4-text-and-snake.md) · [GPU backend selection](0038-vello-gpu-and-glyph-atlas.md) · [Native frame pacing](0036-fifo-drawable-readiness.md)
