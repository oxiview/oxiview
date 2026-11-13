# Dependency management

## Adding and upgrading dependencies

Record the following assessments before adding a dependency. Recheck when an upgrade changes sources, licenses, or the dependency tree:

- A selectable license branch is allowed by the root [deny.toml](../../deny.toml) and compatible with MIT / Apache-2.0 distribution. Do not duplicate the allowlist here.
- Active maintenance: inspect recent releases, commits, issue responses, and security advisories rather than download counts alone.
- Whether it is pure Rust; for non-Rust parts, assess build tools, system libraries, target support, and distribution cost. Prefer pure Rust without treating system bindings as violations.
- Whether the added transitive dependencies, duplicate versions, compilation time, and source footprint are worthwhile.
- Whether the workspace or standard library already provides equivalent functionality that can be reused.
- Whether it can be encapsulated within its owning context without exposing third-party types in public APIs.

Define versions only in `[workspace.dependencies]` in the root [Cargo.toml](../../Cargo.toml). Crates inherit them with `xxx.workspace = true` in dependencies / dev-dependencies / build-dependencies as appropriate; do not repeat versions. Foundation libraries follow [ADR-0022](../adr/0022-foundation-libraries.md).

Register restricted external dependencies with `packages` and `allow` in [layers.toml](../../xtask/layers.toml) `[[external]]` entries. Package names support exact matching and trailing `*`. Checks cover direct consumers declared by the workspace across normal / build / dev, optional, and target-specific dependencies, resolving renames by real package name. They do not restrict transitive consumers inside third-party libraries. Type-level anti-corruption layers still require code review; see [architecture](../architecture.md).

Run `cargo xtask deps` to review direction, licenses, advisories, duplicate versions, and sources. Duplicate versions currently produce warnings rather than a blanket ban. Pass `cargo xtask verify` again before committing. Advisory refresh failure is an environment error; do not present old results as success.

## Upstream patch queue

Keep patches in the root Cargo.toml `[patch.crates-io]` and prioritize upstream fixes. Use traceable pinned commits or version-controlled local patches, not machine-specific directories. Record each new patch below with its crate, reason, upstream issue/PR link, and explicit removal condition. On upgrades, check whether upstream has merged and released the fix. Once the condition is met, remove the patch and entry, then verify again.

The current deny policy rejects unapproved Git sources. Review and explicitly update source allow rules before using a Git patch; never bypass cargo-deny. This step adds no patches and changes no source policy.

| Crate | Reason | Upstream issue / PR | Removal condition |
| --- | --- | --- | --- |
| None | No upstream patches currently | Not applicable | Not applicable |

## When to write an ADR

For costly choices that lock in public interfaces, data formats, runtimes, platform bindings, or extensive implementation, apply the [three Git convention criteria](git-workflow.md). An ADR is required when the decision is hard to reverse, surprising without context, and involves real trade-offs. A routine local tooling dependency does not need an ADR merely because it is new; a complete review record is still required.

## Shared foundation dependencies (M0 S1)

The following exact pins implement ADR-0022. They are the current published
versions reviewed for S1; exact pins keep geometry and color behavior stable
until an explicit dependency review. All three are pure Rust with no native
build tools or system-library distribution requirements, and offer an
MIT license branch compatible with the workspace policy.

| Dependency | Assessment and scope |
| --- | --- |
| `kurbo =0.13.1` | MIT OR Apache-2.0. [Release metadata](https://docs.rs/crate/kurbo/0.13.1) and [upstream commits](https://github.com/linebender/kurbo/commits/main/) show continued Linebender maintenance, including numerical test improvements and Rust toolchain updates in 2026. The selected default features add `arrayvec`, `smallvec`, and `polycool`; no native code or duplicate geometry version is required. Neither std nor the existing workspace provides affine and geometry primitives. Private wrappers in `oxiview-base` prevent upstream types from entering public signatures. |
| `peniko =0.6.1` | MIT OR Apache-2.0. [Release metadata](https://docs.rs/crate/peniko/0.6.1) and the [upstream project](https://github.com/linebender/peniko) show ongoing Linebender graphics maintenance. Its selected graph reuses `kurbo` and `smallvec`, adding `color` and `linebender_resource_handle`. S1 uses its re-exported typed color representation and conversions; std and the existing workspace have no equivalent color-space implementation. Color storage stays private, and no brush or renderer API is introduced. |
| `tracing =0.1.44` | MIT. [Release metadata](https://docs.rs/crate/tracing/0.1.44) and the [upstream project](https://github.com/tokio-rs/tracing) establish the maintained Tokio tracing ecosystem. Only `std` is enabled; attributes and log bridging are disabled. The selected graph adds `tracing-core`, `once_cell`, and `pin-project-lite`, without another proc-macro dependency. Existing workspace dependencies provide no tracing dispatcher. The helper accepts an explicitly supplied subscriber and owns its thread-local guard, avoiding a new subscriber implementation or dependency. |

`cargo tree -p oxiview-base` shows eight new transitive runtime packages shared
across these dependencies and no duplicate versions in this crate's graph.
`thiserror` reuses the existing workspace definition. The lockfile also records
optional resolution dependencies (`euclid`, `num-traits`, and `autocfg`) that are
not compiled by the selected feature graph. The small pure Rust graph is
appropriate for shared geometry, color conversion, and observability; it adds
no renderer or platform runtime.

Upstream issue-list requests were unavailable during this assessment, so recent
issue-response activity could not be independently confirmed. Advisory freshness
must be established by the mandatory `cargo xtask verify` dependency check; a
network failure refreshing the database is not evidence of an advisory pass.

## Retained scene property tests (M0 S2)

| Dependency | Assessment and scope |
| --- | --- |
| `proptest =1.12.0` | MIT OR Apache-2.0. The [published release](https://docs.rs/crate/proptest/1.12.0) is dated 2026-10-10; [upstream commits](https://github.com/proptest-rs/proptest/commits/main/) show maintenance of random generation, toolchain compatibility, and safety fixes during 2026. Pure Rust, with no native build tool or system-library requirement beyond Rust's platform support. Used only in `oxiview-scene` dev-dependencies; no upstream type enters the scene API. Neither std nor the existing workspace test harness provides generated strategies and automatic shrinking. |

Only `std` is enabled; default fork, timeout, and bit-set support are disabled.
The published dependency metadata lists `bitflags`, `chacha20`, `core_detect`,
`num-traits`, `rand`, `rand_xorshift`, and `unarray` as nonoptional dependencies;
`std` also supports regex strategies. Disabling fork/timeout avoids their
`rusty-fork` and `tempfile` machinery. This test-only footprint buys generated
edit sequences and minimized counterexamples without adding a production
runtime dependency. The version is pinned exactly at the workspace root.

Local resolution of 1.12.0 is currently blocked by crates.io DNS failure; the
cached index only contains older releases. Consequently the resolved transitive
footprint, duplicate versions, licenses, and fresh advisories remain unverified.
The lockfile must be resolved and `cargo xtask verify` must pass when registry
access is available; this assessment does not claim those checks passed.

## CPU rasterizer (M0 S3)

| Dependency | Assessment and scope |
| --- | --- |
| `vello_cpu =0.4.0` | Apache-2.0 OR MIT, compatible with the workspace license policy. Maintained by the [Linebender Vello project](https://github.com/linebender/vello). The cached 0.4.0 source changelog records the 2026-11-11 release, including compositing fixes and API changes. Pure Rust, including SIMD support, with no native build tools or system libraries. The existing workspace provides geometry, retained scenes, and pixel comparison, but no CPU rasterization implementation. Only `oxiview-render-cpu` may depend on it; all upstream types remain private. |

Default features are disabled; only `std` and `u8_pipeline` are enabled. Text,
PNG, multithreading, and the f32 pipeline are disabled. The observed
`cargo tree --offline -p oxiview-render-cpu` runtime graph adds `vello_common`,
`fearless_simd`, `fearless_simd_macros`, `bytemuck` and its derive macro,
`hashbrown`/`foldhash`, `guillotiere`/`euclid`/`num-traits` (with `autocfg` at
build time), and `log`. It reuses the workspace's `peniko`, `kurbo`, `color`,
`smallvec`, `thiserror`, and proc-macro dependencies. There are no duplicate
versions in the rasterizer's runtime graph. The test harness graph retains
its existing two `miniz_oxide` versions. Optional text packages such as `glifo`
and `skrifa` appear in lockfile resolution but are absent from the enabled build
feature graph. This footprint provides path rasterization and compositing
without implementing those algorithms inside the framework.

The API is unstable, output can change between versions, gradient quality is
below the GPU path, and pixels will not match the GPU renderer byte for byte.
Snapshots catch silent output drift. Bumping the version requires re-reviewing
and re-accepting baselines, including changes to transitive rasterizer packages.
The exact pin alone does not freeze the transitive graph; retain Cargo.lock.

S3 uses its own straight-RGBA8 image type and converts to the testing image in
snapshot tests. This deliberately revises the M0 plan's image-type wording:
`oxiview-testing` is dev-only and cannot appear in the production API. The
rasterizer has its own external allowlist entry; the GPU entry is unchanged.

The exact cached crate sources were inspected for path, paint, transform, clip,
opacity-layer, and pixmap APIs. Live registry fetching failed DNS resolution;
offline resolution and compilation succeeded. Fresh advisory status is assessed
by `cargo xtask deps`; cached source availability does not establish that status.

## Offscreen GPU renderer (M0 S4)

| Dependency | Assessment and scope |
| --- | --- |
| `wgpu =30.0.2` | MIT OR Apache-2.0. Maintained by the [wgpu project](https://github.com/gfx-rs/wgpu); the exact cached 30.0.2 release sources and manifests were inspected for adapter requests, device-loss callbacks, compute pipelines, and readback. Pure Rust implementation, with native API bindings and operating-system graphics drivers required at runtime. The workspace previously had geometry, retained scenes, and CPU rasterization, but no GPU renderer or GPU resource/device management. All wgpu types remain private to `oxiview-render-wgpu`. |

The exact version is defined only in workspace dependencies. Default features
are disabled; `std`, `metal`, `vulkan`, `dx12`, and `wgsl` are enabled together.
`webgpu` is disabled. No direct geometry, executor, or additional rendering
library dependency is added. A small safe standard-library executor waits for
native adapter/device requests. Geometry uses the existing foundation API and
is flattened to polygons for GPU coverage testing, not an SDF fast path.

Offline resolution added 78 lockfile packages, including target-specific and
optional resolution packages. The observed macOS build graph includes
`wgpu-core`, `wgpu-hal`, `wgpu-types`, `wgpu-naga-bridge`, `naga`, `naga-types`,
`half`/`zerocopy`, `parking_lot`/`lock_api`, shader diagnostic utilities,
`objc2` and its Metal/Foundation/QuartzCore bindings, and `raw-window-metal`.
`cargo tree --offline -p oxiview-render-wgpu -d` reports `syn` 2.0.119 and
3.0.10 in proc-macro dependencies. Windows/Vulkan resolution adds `ash`,
`gpu-allocator`, and Windows bindings; browser-related packages appearing in
the lockfile do not imply an enabled browser backend. This larger footprint
provides shader translation, native GPU abstraction, validation, synchronization,
and safe readback that neither std nor the existing workspace provides.
Transitive native bindings do not widen the direct-consumer external allowlist.

All three backend **features** compile together in the macOS build. Upstream
wgpu target gates exclude Vulkan and DX12 backend implementations on this Mac;
this is not evidence of compiling or running their Windows/Linux implementations.
Runtime verification is restricted to macOS through Metal, with no claim of a
byte-for-byte match with the CPU renderer. In the current execution environment,
Metal exposes no adapter, so GPU tests print their reason and skip. Shader
parsing/validation runs without a device. Actual pixel readback and simulated
loss/recovery still need a Mac execution environment that exposes an adapter.

This adapter owns a straight-RGBA8 image with the CPU image's public shape.
Unifying those types is deferred because the adapter dependency boundary forbids
an `oxiview-render-cpu` dependency. Process-wide device state is shared by both
clones and independent contexts; real loss callbacks and simulated destruction
invalidate the same state for replacement on next use.

Live documentation fetching was unavailable. `cargo xtask deps` passed dependency
direction checks but could not refresh RustSec because `github.com` failed DNS
resolution. Fresh advisories and the complete cargo-deny review are therefore
unverified; offline compilation is not an advisory pass.

## One-image AppKit viewer (M0 S5)

| Dependency | Assessment and scope |
| --- | --- |
| `objc2 =0.6.5` | MIT. Objective-C runtime bindings, restricted as a direct dependency to `oxiview-platform-macos`. Reuses the version already resolved by the Metal renderer. |
| `objc2-foundation =0.3.2` | MIT. Foundation bindings for main-thread markers, NSObject, notifications, strings, and geometry. Restricted to the same adapter. |
| `objc2-app-kit =0.3.2` | Zlib OR Apache-2.0 OR MIT; the MIT branch is compatible with workspace distribution. AppKit bindings for one window and one bitmap image view. Restricted to the same adapter. |

Exact versions are defined in the root workspace and inherited by the adapter
only on macOS. The [upstream project](https://github.com/madsmtm/objc2) and
[published AppKit metadata](https://docs.rs/crate/objc2-app-kit/0.3.2) identify
the maintained bindings project; exact cached sources and its runnable window
example were inspected for ownership, feature gates, and callback signatures.
Recent issue-response and commit activity were not independently confirmed.
These are native system bindings, not pure Rust implementations: compilation
requires the Apple SDK/linker, and presentation requires macOS AppKit,
Foundation, the Objective-C runtime, and a display session. No separate native
library is bundled or downloaded by this change.

AppKit defaults are disabled. Enabled features are `std`, `NSApplication`,
`NSRunningApplication` (activation policy), `NSResponder`, `NSWindow`,
`NSView`, `NSControl` (image-view superclass), `NSImageView`, `NSImage`,
`NSImageRep`, `NSBitmapImageRep`, and `NSGraphics` (bitmap color-space name).
Foundation defaults are disabled; explicit features are `std`, `NSObject`,
`NSNotification`, `NSString`, and `NSGeometry`. Their feature dependencies
also enable the Foundation support needed by these AppKit declarations.

The observed adapter tree reuses `objc2-encode`, `bitflags`,
`objc2-core-foundation`, and the existing `thiserror` proc-macro graph.
It has no duplicate versions; only `objc2-app-kit` was newly added to the
root lockfile. Disabling unused framework features avoids the default broad
AppKit binding surface. Neither std nor the existing framework crates provide
a native window. All Objective-C objects stay private to the adapter.

The port uses the existing `thiserror` dependency for its required top-level
error enum and has no renderer or Objective-C dependency. The example is a
workspace member under `examples/` that inherits the existing PNG encoder; it is
exempt from layer rules as an application, but the xtask ban still applies.

The window exists solely to show one frame and is replaced when M1 designs the
real platform trait. Straight RGBA8 bytes are copied into an AppKit-owned
non-premultiplied bitmap, retained by an NSImage in an NSImageView. There is no
GPU surface, Metal layer, or rendering dependency in the platform crates.

Dependency-direction checks passed. Fresh RustSec review remains blocked:
`cargo xtask deps` could not resolve `github.com` while refreshing the advisory
database and returned exit code 2. This is not a cargo-deny or advisory pass.
