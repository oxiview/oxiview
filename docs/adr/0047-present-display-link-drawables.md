---
status: accepted
---

# Present the display link's drawable from its final render command

The M7 viewer renders on the window thread and scrolls on demand. Its earlier windowed FIFO path waited for a wgpu surface texture, submitted the scene and then asked wgpu to present. In a native trace the next viewer frame after each of 57 empty refresh slots had finished its recorded GPU work at least 14.47 ms before that slot. Rendering time alone could not explain the observed timing. [M7 evidence](../plans/m7-widgets-and-scrolling.md#native-scrolling-on-the-built-in-120-hz-display)

## Decision

The synchronized viewer uses the drawable supplied by a window-associated `CAMetalDisplayLink`. The platform adapter tracks the update's CPU deadline, estimated presentation time and layer generation. It can retain one unclaimed live update for a pending request. A frame event claims it once. The link pauses once input and invalidation settle. Resize, display transitions, occlusion and teardown retire the old handoff. The synchronous resize transaction and the default on-demand path for other windows continue to use the existing surface API. [On-demand scheduling](0004-frame-scheduling.md) · [Surface acquisition](0036-fifo-drawable-readiness.md) · [Apple's display-link contract](https://developer.apple.com/documentation/quartzcore/cametaldisplaylinkdelegate/metaldisplaylink(_:needsupdate:))

The composition root lends the platform-owned drawable to the renderer. The render adapter checks the native resource type, wgpu device, pixel format and extent. The existing SDF or Vello pipeline renders the scene. A Metal output pass on the same queue writes the drawable and requests presentation on that pass's command buffer. The platform checks the lease again before committing. The renderer retains its submission guard until the command is committed or the lease is discarded. Input and generic UI code use project-owned types. [Platform lease](../../crates/oxiview-platform-macos/src/macos/metal_frames.rs) · [Output pass and pixel tests](../../crates/oxiview-render-wgpu/src/metal_output.rs)

The Rust window thread has no Core Animation run loop. The render adapter wraps wgpu surface configuration in an explicit Core Animation transaction and disables implicit animations while wgpu sets the Metal layer's color space and drawable size. It commits the transaction when configuration returns. The same scope covers resize. [Surface configuration](../../crates/oxiview-render-wgpu/src/surface.rs) · [Apple's transaction documentation](https://developer.apple.com/documentation/quartzcore/catransaction)

## Consequences

The render adapter now directly uses the pinned Metal bindings for the final pass. Its color conversion must agree with the wgpu path for opaque and translucent pixels in each supported surface format. The adapter tests exercise distinct textures and compare the resulting bytes. AppKit remains confined to the platform adapter. The caller opts into this scheduling policy at the shared viewer entry point, which both the normal application and the native workload use. [Dependency rules](../../xtask/layers.toml) · [Viewer entry point](../../examples/log-viewer-oxi/src/lib.rs)
