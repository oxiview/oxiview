---
status: accepted
---

# Animation: interruptible springs and easing, with a compositing fast path for transform / opacity

Any style property can transition through **springs** or **easing curves**. Animations are **interruptible at any time**: new targets continue from the current value and velocity without jumps. Animations changing only transform / opacity use a **compositing fast path**, updating layer parameters without style recalculation or layout. The framework also provides timeline orchestration (parallel, sequential, staggered) and gesture-driven motion (tracking drags and handing release velocity to springs). Sample animations at ADR-0004's predicted presentation timestamp and respect the system reduced-motion preference (ADR-0017).

## Consequences

- State-variant transitions (ADR-0008) animate by default, with a global opt-out.
- A running animation keeps the window in an active frame-demand state. Return promptly to zero idle frames when it ends; the idle CPU metric in CI must cover this.
