---
status: in-progress
milestone: M1
---

# M1 first window: detailed plan

Goal: a native macOS window presenting through a `wgpu::Surface`, rendered on its own window thread while the main thread only pumps system events. Frames are produced on demand and aligned to the display's vsync; rounded rectangles take an SDF fast path. A headless platform backend implements the same platform interface as its second implementation (ADR-0018). The two M1 hard targets, input-to-pixel latency and idle CPU, get their first real measurements. M0's blit presentation path is deleted.

## Decisions

Confirmed before execution:

| Question | Decision |
| --- | --- |
| Vello | Deferred to M4, when text and arbitrary paths need it. M1 adds SDF for rounded rectangles; the M0 polygon compute path stays as the fallback for everything else. ADR-0012's M0 note is corrected accordingly. |
| Native surface handle | An owned handle type in `oxiview-base` (for example a `CAMetalLayer` pointer variant). Its only constructor is an `unsafe fn` whose contract the producing platform adapter upholds, following the raw-window-handle model. `oxiview-base` thereby gains one narrow unsafe exception: declaring that constructor, with no unsafe blocks or unsafe impls. `oxiview-render-wgpu` consumes the handle and converts it to wgpu's surface target internally; `oxiview-app` wires the two without unsafe. No third-party handle type in any public API. Recorded in a new ADR, with AGENTS.md and the unsafe conventions updated. |
| Hard targets | Both levels. Ordinary `verify` runs deterministic headless tests: zero frames while idle, and input-to-frame latency counted in virtual vsync intervals. `bench` (`verify --full`) opens a real macOS window and measures `input_to_pixel_ms` and `idle_cpu_percent`. |
| Live resize | Flicker-free on macOS in M1. During live resize the main thread waits, bounded by a timeout, for the window thread to commit a frame at the new size, presented with the Core Animation transaction. The main/window-thread synchronization protocol is fixed in M1 and must also be implementable on the headless backend. |
| Minimum macOS | macOS 14, for `CADisplayLink` on views and screens. `CVDisplayLink` is deprecated and not used. |
| Crate placement | Frame scheduling and the window-thread loop live in `oxiview-core`; wiring platform, renderer, and window thread lives in `oxiview-app`. Both are created as minimal skeletons; M2 adds the node tree to `oxiview-core`. |
| Window count | One window in the demo. The interface carries window identity and per-window event routing from the start; multiple windows and shared-thread mode are M9. |
| Input | Raw pointer and keyboard events delivered to the window thread. No hit testing (M2), no IME (M6), no key mapping (M5). |
| Headless rendering | The headless backend renders frames through the CPU renderer, so input-to-pixel tests run end to end without a GPU. |
| Execution | Same as M0: one step, one verification, one commit. |

## Scope

**Included**: the real `oxiview-platform` interface replacing M0's placeholder, `oxiview-platform-headless`, a rebuilt `oxiview-platform-macos`, per-window surfaces and the SDF path in `oxiview-render-wgpu`, frame scheduling in a minimal `oxiview-core`, wiring in a minimal `oxiview-app`, one demo example, and real measurement for two hard targets.

**Excluded**, deferred to the milestone named:

- Vello and vector text (M4).
- Hit testing, pointer dispatch to nodes, and the arena node tree (M2).
- Focus, key mapping, and themes (M5); IME (M6).
- Animation APIs (M8). M1 samples time at the predicted presentation timestamp but exposes no animation system.
- Multiple windows, shared-thread mode, menus, title-bar customization, and window materials (M9).
- Continuous window and layer rendering (M10). Their frame-policy variants exist and report unsupported.
- Damage regions and partial repaint. On-demand frames repaint the whole window.

## Risks

- wgpu may not expose the Metal drawable's presented time. If it does not, `input_to_pixel_ms` ends at the display link's target timestamp for the frame that consumed the input, and the report names that endpoint. A proxy is never reported as the real presentation time.
- `presentsWithTransaction` with a window-thread renderer is fragile: a slow window thread stalls the main thread during resize. The wait is bounded; on timeout the main thread proceeds and the event is traced.
- Real-window measurement needs a display session. Inside a sandbox without a GPU or WindowServer, `bench` reports an environment error (exit 2), never a pass.

## Steps

Each step passes `cargo xtask verify` on its own and is committed on its own.

### S1 Platform interface (`oxiview-platform`, `oxiview-base`)

- Replace M0's one-image port. Define the event loop entry, window creation, window identity, raw pointer and keyboard events, scale factor and its changes, the vsync clock source, resize with the synchronization protocol above, and the main-to-window-thread channel.
- Frame policy enum with on-demand implemented and both continuous variants reserved, returning an unsupported error.
- Owned native surface handle type in `oxiview-base` with its safety contract. Write the ADR for it, update the ADR index, and amend AGENTS.md and the unsafe conventions for the `oxiview-base` exception.
- M0's one-image port stays alongside the new interface, marked for removal, so the M0 window keeps building until S5 replaces it.
- Capability query per ADR-0002, with no capability yet beyond what the window needs.

Acceptance: the interface compiles with no platform dependency. Every method has a doc comment stating its threading requirement.

### S2 Headless backend (`oxiview-platform-headless`)

- Implement every S1 interface method: simulated windows, injected input, scale-factor changes, virtual vsync, and the resize protocol.
- Fully deterministic through `oxiview-base`'s injectable clock, scheduler, and randomness (ADR-0016).

Acceptance: tests exercise every interface method on the headless backend. Any method that cannot be implemented headlessly is treated as platform-biased and revised in S1's interface.

### S3 Frame scheduling (`oxiview-core`)

- Platform-agnostic on-demand scheduler: a frame is requested only by invalidation, a pending timed request, or a system repaint request. No idle loop.
- Frame time is the predicted presentation timestamp from the vsync source.
- The window-thread loop: receive events, update, request a frame, render, present.

Acceptance: on the headless backend, an idle window produces zero frames across many virtual vsync intervals; one injected input produces exactly one frame on the next interval.

### S4 Surface and SDF (`oxiview-render-wgpu`, `oxiview-render-cpu`)

- A per-window surface created from the owned handle, sharing the process-wide device. Surface loss and device loss both recover.
- SDF shader for filled and stroked rounded rectangles with solid and linear-gradient brushes. Everything else keeps the polygon path.
- Unify the two RGBA8 image types into one owned type.

Acceptance: offscreen SDF output is compared with the CPU renderer within a stated tolerance, with no byte-for-byte requirement. Device-loss recovery still passes. CPU snapshots are unchanged.

### S5 macOS backend (`oxiview-platform-macos`, `oxiview-app`)

- AppKit event loop on the main thread, one window thread, a `CAMetalLayer`-backed view, `CADisplayLink` vsync, backing scale factor and its changes.
- Flicker-free live resize using the S1 protocol.
- `oxiview-app` wires the platform backend, renderer, and scheduler.
- Delete M0's one-image port and blit presentation path, and replace the M0 window example with the M1 demo.

Acceptance: `cargo run -p` the demo opens a window, and dragging its edge shows neither stretching nor flicker, confirmed by a human. An idle window presents no frames, verified by trace counters.

### S6 Hard targets (`xtask`, `oxiview-testing`)

- Real measurement sources for `input_to_pixel_ms` and `idle_cpu_percent` in `bench`: input injected in process through AppKit's event queue, measured to the presentation endpoint named in Risks; idle CPU from process resource usage over a fixed idle interval after settling, plus a zero idle-frame count.
- Upgrade the baseline protocol so these two metrics accept measured values, with unit and direction validation, hard-target checks, and self-tests. `list_100k_scroll_dropped_frames` stays pending until M7.
- Headless latency and idle-frame tests run in ordinary `verify`.

Acceptance: self-tests fail on a fabricated regression for each metric. Recording a baseline still requires human confirmation.

### S7 Close-out

- Demo: a rectangle follows the pointer; it is also the latency measurement scene.
- Mark M1 done in the roadmap and set this plan's status to done, with an execution record.
- Add a dated note to ADR-0003 and ADR-0004 recording what M1 implemented and what stays reserved.
- Record remaining items.

Acceptance: a fresh clone passes `cargo xtask verify` with every check marked pass, and `cargo xtask verify --full` passes on this Mac.

## Definition of done

1. A fresh checkout passes `cargo xtask verify`.
2. On macOS the demo presents through a wgpu surface on a window thread, with no blit path left in the codebase.
3. Live resize on macOS neither stretches nor flickers.
4. The headless backend implements every platform interface method, tested.
5. An idle window produces zero frames, on both backends.
6. `input_to_pixel_ms` and `idle_cpu_percent` have real measurements checked by `bench`.

## Remaining items

Filled in at close-out.
