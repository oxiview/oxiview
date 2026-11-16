---
status: in-progress
milestone: M1
---

# M1 first window: detailed plan

Goal: a native macOS window presenting through a `wgpu::Surface`, rendered on its own window thread while the main thread only pumps system events. Frames are produced on demand and aligned to the display's vsync; rounded rectangles take an SDF fast path. A headless platform backend implements the same platform interface as its second implementation (ADR-0018). The two M1 hard targets, input-to-pixel latency and idle CPU, get their first real measurements. M0's blit presentation path is deleted.

Current status (2026-11-16): implementation and human visual acceptance are
delivered. S6 baseline reproducibility and S7 full verification remain incomplete;
the performance comparison failures below prevent marking M1 done. M2 has not
started.

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

### Accepted measurement adjustment

[ADR-0026](../adr/0026-m1-native-latency-gates.md) records the owner's adjustment
after real presentation measurements: windowed phase-swept input-to-submit p95
must fit one measured refresh interval; actual input-to-pixel p95 must remain
within 115% of the human-recorded optimized baseline on the same display interval.
Idle CPU must be at or below 0.1%, with no submissions, presentations or
display-link frame events. Fullscreen pixel baselines remain deferred to M10.
These are the M1 close-out gates; they do not establish one-frame end-to-end
latency for all workloads.

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

- M2: arena node tree, dirty flags, layout, hit testing and pointer dispatch.
- M3/M4: reactivity and components; Parley text and the Vello GPU path. M1 keeps
  the polygon compute fallback and SDF rectangle path.
- M9: multiple windows, shared-thread mode, and reclaimable native surface-handle
  ownership. M1 intentionally retains one Metal layer per window for process
  lifetime under ADR-0025.
- M10: fullscreen pixel-latency baselines, continuous rendering, and further
  compositor/presentation latency work. Continuous input can exceed one refresh
  interval even with full display cadence. On the internal 120 Hz display,
  windowed/restored paint-to-submit p95 can also exceed 8.33 ms with the overlay;
  the optional fullscreen diagnostic does not pass every budget check.
- Revisit the Metal hosting-delegate workaround when wgpu's occlusion handling
  supports this fullscreen path without false timeouts. Check actual occlusion
  and window lifecycle behavior before removing the workaround.
- Partial repaint and damage-region tracking remain deferred; M1 repaints the
  complete window on demand. The 100,000-row scrolling metric remains pending
  until M7; Windows/Linux native backends remain M14/M15.

## Execution record

| Step | Delivery | Validation |
| --- | --- | --- |
| S1 | `3a886f9`: platform interface and owned native surface handle | Local verify passed; threading and ownership contracts documented. |
| S2 | `b41ee54`: deterministic headless backend | Local verify passed; platform interface, virtual input/vsync and resize tests. |
| S3 | `faf4109`: on-demand scheduler and window-thread loop | Local verify passed; one frame for input, no frames during idle, skipped-frame retry. |
| S4 | `131a904`: native surfaces, SDF path and shared RGBA8 image | Local verify passed; CPU/GPU comparisons, snapshots and recovery tests. |
| S5 | `46865da`: AppKit backend and application wiring | Local verify passed; native surface demo replaces the M0 blit path. |
| S6 implementation | `9078375`: native metrics, presentation diagnostics and sparse rendering optimization | Local verify passed; native measurements and sustained presentation checks. |
| S6 diagnostic correction | `857e3d7`: acknowledge the exact input timestamp delivered by AppKit; record the accepted latency gates | Local and fresh-clone ordinary verify passed; native measurements complete, baseline reproducibility still pending. |

On 2026-11-16, the owner confirmed the final demo's visual checks: complete
content, pointer-following rectangle, no stretching or flicker while dragging
window edges, and no persistent white screen after entering/exiting fullscreen.
Yellow overlay latency bars were explicitly evaluated separately.

### S6/S7 close-out checks — 2026-11-16

- Fixed diagnostic event attribution: AppKit can quantize an injected event's
  timestamp by one nanosecond during queue delivery. Diagnostic event numbers
  now correlate a worker receipt with the view callback's exact capture value.
  Capture still starts before queuing, and frame attribution still requires
  exact timestamp equality. Repeated native latency and cadence runs exercise
  this path; no timestamp tolerance or latency threshold was relaxed.
- `cargo xtask verify`: all seven checks passed locally. A fresh local clone,
  with the candidate changes applied and no copied `target` cache, also passed
  all seven checks.
- The owner explicitly approved `cargo xtask bench --record` and immediate
  comparison. Recording succeeded on the internal 120 Hz display. The candidate
  baseline remains uncommitted because subsequent comparison did not reproduce
  every metric within its existing tolerance. Do not record again just to clear
  these failures.
- `cargo xtask verify --full`: exit 1. The seven ordinary checks passed; bench
  failed for idle CPU and two sparse-render metrics. Pixel/submit latency,
  zero idle work, and existing image-comparison metrics passed. M7 scrolling
  remains pending, not passing.

| Metric | Recorded candidate | Latest full verification | Result |
| --- | --- | --- | --- |
| Input-to-submit p95 | 4.573 ms | 4.606 ms | Pass: one interval is 8.333 ms; relative comparison also passes. |
| Input-to-pixel p95 | 30.262 ms | 29.105 ms | Pass: actual presentation, ceiling 34.801 ms. |
| Idle process CPU | 0.02458% | 0.03257% | Hard target 0.1% passes; baseline +10% fails. |
| Idle submitted/presented frames and display-link events | 0 / 0 / 0 | 0 / 0 / 0 | Pass. |
| GPU sparse scene, 1 primitive | 4.130 ms | 7.758 ms | Baseline +10% fails. |
| GPU sparse scene, 128 primitives | 2.842 ms | 3.944 ms | Baseline +10% fails. |
| GPU sparse scene, 1,000 primitives | 7.465 ms | 6.234 ms | Pass. |

The same build produced materially different GPU medians on repeated runs.
Quieter agent activity, a one-second GPU warm-up, and a longer idle settling
period did not resolve the variation; the experimental source changes were
reverted. The Mac was on battery power and the owner could not connect power
during this session. Power and background GPU load remain environment factors
to isolate, not a proven explanation for every failure. Baselines and thresholds
were not changed after the failed comparison.

The supplemental `--fullscreen-check --debug-overlay --json` reported roughly
120 displayed frames/s in all three stages and a confirmed transition frame.
Cadence passed, but paint-to-submit p95 was 9.81 ms in windowed/restored stages
and continuous input-to-submit p95 was 17.78–18.75 ms. Its exit status was 1;
this is recorded separately from the accepted windowed phase-swept baseline.

Next: repeat the unchanged baseline comparison under stable power and desktop
load, investigate any remaining failures, and require a passing
`cargo xtask verify --full` before committing the baseline and marking S6/S7/M1
complete. The final committed checkout must also pass ordinary verify.
