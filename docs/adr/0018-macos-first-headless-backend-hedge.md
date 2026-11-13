---
status: accepted
---

# Platform order: macOS first, Windows / Linux later, with a headless platform backend as a safeguard

The first platform backend is macOS (AppKit + Metal). Complete functionality on macOS, including multiple windows, compositing, developer tools, accessibility, and distribution, before porting to Windows and Linux. This keeps upper-layer progress continuous without the cost of parallel work on three platforms.

The risk is contaminating platform abstractions with macOS semantics. Mitigate it by maintaining a **headless platform backend** from M1 as the interface's second implementation. It simulates windows, input, DPI changes, and vsync, and supports CI and event replay (ADR-0016). Every platform-interface method must be implementable and tested on the headless backend; otherwise, treat it as platform-biased and revise it.

## Considered Options

- **Two platforms in M1**: validates abstractions earliest, but delays upper-layer features.
- **A second platform before text editing**: validates IME differences earlier, but interrupts continuous feature development.

## Consequences

- Windows issues (TSF IME, modal resize loops, fractional DPI) and Linux issues (Wayland input protocols, compositor differences) emerge later. Reserve room for interface rework during ports.
- Accessibility and distribution in M12/M13 cover only macOS; other platform adapters belong to the porting milestones.
