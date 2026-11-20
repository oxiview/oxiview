---
status: accepted
---

# Use Vello GPU and a device-owned glyph atlas

M4 implements the Vello backend in [ADR-0012](0012-scene-abstraction-wgpu-vello.md)
with the upstream Sparse Strips renderer, `vello_gpu` 0.4.0. Its CPU stage
processes vector paths and schedules strips. Its wgpu stage rasterizes and
composites them on the same device that owns the native surface. Offscreen
rendering calls the same adapter. The SDF fast path handles eligible UI
scenes, including the original window benchmark.

The classic Vello compute renderer also supports wgpu 30. In version 0.12
its built-in COLR painter maps several OpenType blend modes to source-over.
Sparse Strips shares Glifo with Vello CPU and can sample an external texture
region directly. These facilities let both adapters paint color glyphs and
let the GPU keep small raster glyphs resident across frames.
[Vello GPU](https://docs.rs/vello_gpu/0.4.0/vello_gpu/) · [Classic Vello painter](https://docs.rs/vello/0.12.0/src/vello/scene.rs.html)

The device owns an 8 MiB, 1,024 × 1,024 RGBA16F glyph atlas with a ceiling
of 4,096 entries. Immutable source images provide stable identities. A frame
protects all the entries it will sample before allocating new ones. The
allocator evicts least-recently-used entries outside that set. Images that
exceed the atlas dimensions or cannot fit alongside the protected entries
use a texture for that submission. This preserves paint order and bounds the
memory retained between frames. A replacement device starts with a fresh atlas.
[Atlas implementation](../../crates/oxiview-render-wgpu/src/atlas.rs) · [Glyph policy](0010-pure-rust-text-stack.md)

Vector glyphs share the shaped font's backing allocation. When COLR rendering
requires normalized font data, the GPU adapter charges the original allocation,
any adjusted bytes and variation coordinates against a separate 32 MiB
budget with a ceiling of 128 entries. Oversized entries use transient storage.
[Font preparation](../../crates/oxiview-render-wgpu/src/glyph.rs)

Both adapters decode sRGB inputs before filtering and blending. The CPU
reference uses Vello's float compositor and a local patch to encode each
finished strip as straight sRGB RGBA8. Quantizing a premultiplied strip
before conversion loses color precision in dark fills and at transparent
edges. Keeping that precision through readback lets the same Snake test
compare CPU and GPU glyph regions at its original four-code tolerance.
[Patch provenance and removal condition](../conventions/dependencies.md#upstream-patch-queue) · [Snake renderer test](../../examples/snake/tests/rendering.rs)
