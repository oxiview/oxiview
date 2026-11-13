---
status: done
milestone: M0
---

# M0 foundation: detailed plan

Goal: a Scene that application code builds once and keeps, two renderers behind it, and a native window on macOS showing the GPU rendering of that Scene. Pixel snapshots lock the CPU rendering, which is deterministic and needs no GPU. M1 adds the real platform port, the headless platform backend, thread separation, on-demand frame scheduling, vsync, and the SDF fast path without revisiting Scene.

## Decisions

Confirmed before execution:

| Question | Decision |
| --- | --- |
| CPU renderer | `vello_cpu`, inside `oxiview-render-cpu` only. No scene type wraps or exposes a vello type. See Risks. |
| wgpu timing | M0, offscreen only: adapter, device, texture, readback. No `wgpu::Surface`. |
| Scene model | Retained scene graph. Nodes are reusable subtrees, not single draw primitives, and not a display list rebuilt every frame. |
| Render context | Process-wide. One wgpu instance, adapter, and device shared by every window. Device-loss recovery is process-wide. |
| Milestone extent | Through a visible window. M0's roadmap text grows by "one window plus offscreen GPU rendering" and nothing else. |
| Window presentation | Offscreen render, then blit pixels to the window. The window is a viewer for one image. M1 replaces this with a real Surface. |
| Platform code | Create `oxiview-platform` and `oxiview-platform-macos` now, holding only what the one window needs. Thin on purpose; M1 fills the port. |
| wgpu backends | Compile Metal, Vulkan, and DX12 together. Verify on this Mac only; the other two are untested until their platforms exist. |
| Execution | Same as M-1: one step, one verification, one commit. |

## Scope

**Included**: `oxiview-base`, `oxiview-scene`, `oxiview-render-cpu`, `oxiview-render-wgpu`, a minimal `oxiview-platform` port, a minimal `oxiview-platform-macos` adapter, and one example that opens a window.

**Excluded**, all deferred to the milestone named:

- The full platform trait, the headless platform backend, main-thread/window-thread separation, on-demand frame scheduling, and vsync (M1, ADR-0018, ADR-0003, ADR-0004).
- The SDF fast path (M1; it needs shaders).
- Hit testing and pointer dispatch (M2).
- The arena node tree and its three dirty-flag levels (M2, ADR-0007). The retained Scene is a render-side structure, not that tree.
- Text and fonts (M4).
- Damage regions, tiling, multithreaded rasterization.

## Crates

`oxiview-base`, `oxiview-scene`, `oxiview-render-cpu`, `oxiview-render-wgpu`, `oxiview-platform`, and `oxiview-platform-macos` are already registered in `xtask/layers.toml`. Create each when its step needs it, through `cargo xtask new-crate`. No crate is created ahead of its step.

`vello_cpu` joins the `[[external]]` entry that already restricts `vello` and `wgpu`, allowed only in `oxiview-render-cpu`. `objc2` stays restricted to `oxiview-platform-macos`.

## Risks

`vello_cpu` is young. Its API is unstable, its output can drift between versions, gradients are lower quality than the GPU path, and its pixels will not match the wgpu renderer byte for byte. Pin the version exactly, keep every vello type inside `oxiview-render-cpu`, and record the version and the reason in the dependency conventions. Snapshots guard against silent drift. Bumping the version requires re-reviewing and re-accepting baselines, not a blind lockfile update.

## Steps

Each step passes `cargo xtask verify` on its own and is committed on its own.

### S1 Shared foundation (`oxiview-base`)

- Owned geometry and color: point, size, rectangle, corner radii, affine transform, sRGB and linear color with conversions both ways. `kurbo` and `peniko` arrive per ADR-0022, wrapped so neither type escapes the crate.
- `Clock`, `Scheduler`, and `Rng` traits, each with a manual implementation and a system implementation. Tests show that production paths reach only the manual implementations, so nothing reads system time, scheduling, or entropy directly (ADR-0016).
- `tracing` initialization helper.

Acceptance: `cargo xtask verify` passes. A test fails if a production module can name a system implementation.

### S2 Retained scene graph (`oxiview-scene`)

- A `Scene` addressed by stable node identity. Nodes form reusable subtrees: a subtree is defined once and placed many times under different transforms.
- Node kinds: filled and stroked rounded rectangle, solid color and linear gradient, affine transform, rectangle and rounded-rectangle clip, opacity group.
- Incremental edits: insert, remove, replace, and reparent a node, then render the result. A property test asserts that building a scene through edits equals building it in one pass.
- An encoded form with no renderer types in it, which both renderers consume. No third-party type in the public API.
- Cycles are rejected at edit time. A subtree reference must point at a subtree that already exists.

Acceptance: the property test passes, and the crate's public API exposes no `vello`, `kurbo`, or `peniko` type.

### S3 CPU renderer (`oxiview-render-cpu`)

- A `Renderer` trait and a `CpuRenderer` implementing it: Scene in, RGBA8 out, written into `oxiview-testing`'s `RgbaImage`.
- `vello_cpu` sits behind the anti-corruption layer. The crate documents the risks above and opts into `unsafe_code` only if vello's API forces it.
- Snapshot suite, baselined on this machine and accepted after human review: rounded rectangle, linear gradient, clip, opacity group, a reused subtree placed twice, and overlapping draw order.

Acceptance: changing one pixel in the renderer fails the snapshot self-test. `cargo xtask snapshot --review` lists the new baselines for confirmation before `--accept`.

### S4 Offscreen GPU renderer (`oxiview-render-wgpu`)

- A process-wide `RenderContext`: one wgpu instance, adapter, and device, created on first use and reused.
- Offscreen render to a texture and readback to RGBA8. No surface, no window handle.
- Device-loss recovery: drop the device and rebuild it at process scope. A test simulates loss and checks that the next render succeeds.
- Compile the Metal, Vulkan, and DX12 backends. Tests skip cleanly when no adapter is available and say so in their output.

Acceptance: on this Mac the readback is non-empty and the device survives a simulated loss. No assertion that the bytes equal the CPU renderer.

### S5 Minimal window (`oxiview-platform`, `oxiview-platform-macos`)

- The port exposes exactly one capability: open one window and present an RGBA8 frame by blitting it. No event loop trait, no input, no DPI, no vsync.
- The macOS adapter implements it with the `objc2` family, confined to that crate. The example wires the port to `oxiview-render-wgpu`.
- The example builds a Scene, renders it offscreen, and shows it. It is the milestone's runnable demo.

Acceptance: `cargo run -p` the example opens a window on macOS displaying the rendered scene. The port crate compiles without `objc2`.

### S6 Close-out

- Mark M0 done in the roadmap and set this plan's status to done.
- Add a dated note to ADR-0012 recording that M0 adopted `vello_cpu` for the CPU path and offscreen wgpu ahead of M1.
- Record remaining items: the blit presentation path to be replaced by a Surface in M1, Vulkan and DX12 compiled but unverified, and the ADR-0013 wgpu-in-public-API conflict still open before M10.

Acceptance: a fresh clone passes `cargo xtask verify` with every check marked pass.

## Definition of done

1. A fresh checkout passes `cargo xtask verify`.
2. The example renders a Scene to a PNG with no window and no GPU.
3. The snapshot suite catches a one-pixel regression.
4. No production path reads system time, scheduling, or entropy directly.
5. On macOS, the example opens a window showing the GPU rendering of the Scene.

## Execution record

Code was implemented by Codex step by step; the coordinator wrote each step's brief, reviewed the code, re-ran verification outside the Codex sandbox, and reviewed snapshot baselines before acceptance.

| Step | Actual deliverables | Deviations and reasons |
| --- | --- | --- |
| S1 | `oxiview-base`: owned geometry and sRGB/linear color, `kurbo`/`peniko` wrapped inside the crate, `Clock`/`Scheduler`/`Rng` with manual and system implementations, tracing helper. | System implementations are `cfg(test)`-gated. The boundary test inspects real Cargo artifacts: the production rlib contains no system implementation names, the test executable contains all three. Scheduling is infallible. |
| S2 | `oxiview-scene`: retained Scene with stable node identity, reusable subtrees, edit operations with cycle rejection before mutation, renderer-neutral encoding. | The property test covers shared subtrees, transforms, clips, opacity groups, and gradients; a 5,000-node build test has a 500 ms limit. |
| S3 | `oxiview-render-cpu`: `Renderer` trait and `CpuRenderer` over `vello_cpu =0.4.0`; six snapshot baselines (rounded rectangle, linear gradient, clip, opacity, reused subtree, overlap). | The renderer owns its RGBA8 image type because `oxiview-testing` is dev-only; unifying image types is deferred. Baselines were accepted after human pixel review. |
| S4 | `oxiview-render-wgpu`: process-wide instance/adapter/device, offscreen render and readback, process-wide device-loss recovery; Metal, Vulkan, and DX12 compile together. | No `vello` dependency in M0: a compute shader rasterizes flattened polygons from the shared encoding; Vello arrives with the M1 SDF path. GPU tests skipped inside the Codex sandbox (no adapter) and were re-run outside it, where readback and recovery pass on this Mac. |
| S5 | `oxiview-platform` one-image port, `oxiview-platform-macos` AppKit adapter (`objc2` confined there), `examples/window` demo with `--headless` CPU PNG output. | The example was first produced as a standalone package; the coordinator made it a workspace member exempt from layer rules so verify lints and tests it. The native window was confirmed open on screen (640×420 content) outside the sandbox. |
| S6 | Roadmap, plan status, ADR-0012 note, remaining items. | Steps were verified individually but committed together in one commit, because S1–S5 shared uncommitted root manifest, lockfile, and convention edits. |

## Remaining items

- Presentation blits pixels into the window. M1 replaces this with a `wgpu::Surface`; the blit path is expected to be deleted, not extended.
- Vulkan and DX12 compile but have not been run. Verifying them waits for the Windows and Linux milestones.
- [ADR-0013](../adr/0013-external-content-compositing.md) still conflicts with [ADR-0021](../adr/0021-crate-layout-strategic-ddd.md) on exposing wgpu publicly. Unresolved; decide before M10.
- The platform port created here is intentionally incomplete. M1 designs the real trait. Nothing in this port should be treated as settled API.
- `oxiview-render-cpu` and `oxiview-render-wgpu` each own an RGBA8 image type; unify them when the real platform port consumes frames in M1.
