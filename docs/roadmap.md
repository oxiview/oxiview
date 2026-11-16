# Milestone roadmap

Progress from small to large. Every milestone delivers a runnable demo, tests, and metrics integrated into verification, without carrying unfinished work across milestones. Detailed plans are developed one milestone at a time in `docs/plans/`.

## Guiding principles

1. **Every milestone is runnable**: demo application + tests + metrics.
2. **Finalize abstractions only after a second implementation**: Scene (CPU / wgpu), platform (macOS / headless, ADR-0018), layout (Taffy / test double). Every interface with only one implementation is marked unstable.
3. **Crate layering and dependency direction are fixed from M-1**, enforced by `cargo xtask` (ADR-0021).
4. **Local verification only**: interpret every reference to integration into CI as integration into `cargo xtask verify` (ADR-0019).
5. **Build robustness first**: injectable clocks, scheduling, and randomness; no panics in rendering paths; end-to-end tracing; fuzz text and input code; recover from GPU device loss.
6. **Reserve concrete extension points**: deferred capabilities such as accessibility semantics, continuous frame policies, and shared-thread mode have space in the types from the start, protected by tests.

## Milestones

| # | Name | Key scope | Related ADRs |
|---|---|---|---|
| M-1 | [Engineering baseline](plans/m-1-engineering-baseline.md) ✅ | **Choices**: technology, design, build system, directory constraints (ADRs 0019–0023); **conventions**: AGENTS.md (unsafe/panic/errors, API stability and versions, dependencies, commits and branches); **skills**: ADR and glossary maintenance, scaffolding (crates/widgets/capabilities), review checklist, verification; **test harness**: snapshots, benchmarks, performance; **agent harness**: automated `cargo xtask` verification; workspace skeleton + dependency rules and checks | 0001, 0019–0023 |
| M0 | [Foundation](plans/m0-foundation.md) ✅ | Retained Scene; CPU renderer (vello_cpu) with pixel snapshots; offscreen wgpu renderer with a process-wide device; injectable clocks, scheduling, and randomness; tracing; one macOS window presenting by blit | 0012, 0016 |
| M1 | [First window](plans/m1-first-window.md) — close-out in progress | Platform interface; macOS + headless platform backends; main/window thread separation; wgpu surface; on-demand frame scheduling and vsync; SDF fast path. Implementation and visual acceptance delivered; performance reproducibility and full verification pending. | 0002, 0003, 0004, 0018, 0026 |
| M2 | Node tree and layout | Arena node tree; three levels of dirty flags; layout protocol + Taffy; typed styles; hit testing and pointer events | 0007, 0008, 0009 |
| M3 | Reactivity and components | Signals, scopes, Show/For; Builder API; function components; local executor + background thread pool | 0005, 0006 |
| M4 | Static text | Parley text stack; Vello GPU path; hybrid glyph atlas and vectors; rich text; DPI | 0010 |
| M5 | Focus, keyboard, and themes | Focus and Tab order; Action/Keymap; state variants; tokens and theme hot reload; system preference signals | 0008, 0011 |
| M6 | Text editing | Editing core; IME preedit; selection; undo; fuzzing | 0010, 0011 |
| M7 | Basic widgets and scrolling | Basic widgets; scrolling and inertia; virtual lists and tables | 0001 |
| M8 | Animation | Springs/easing; interruption; compositing fast path; timeline orchestration | 0015 |
| M9 | Multiple windows and system integration | Multiple windows and shared-thread mode; menus, dialogs, clipboard, drag and drop; title bars and materials | 0003, 0017 |
| M10 | Compositing and external content | Compositing layers; continuous rendering at window/layer level; 3D textures; native child views | 0004, 0013 |
| M11 | Developer tools | Inspector; signal dependency graph; code hot patching; event recording/replay | 0016 |
| M12 | Accessibility | AccessKit and macOS adapter | 0014 |
| M13 | Distribution | Packaging, signing, automatic updates; application lifecycle | 0017 |
| M14 | Windows port | Win32 + D3D12; modal resize loop; DPI; TSF; complete accessibility and distribution | 0002, 0018 |
| M15 | Linux port | Wayland first, X11 compatibility; complete accessibility and distribution; full platform matrix | 0002, 0018 |
