---
status: accepted
---

# Recover focus at the next eligible Tab stop

A focused control can be hidden, disabled or removed while the rest of the window stays interactive. On 2026-11-20 the owner chose immediate recovery at a neighbor in the original traversal order. An eligible return target supplied by the application takes priority, as when a player closes Snake's inline Settings. [M5 design](../plans/m5-focus-keyboard-and-themes.md#focus-and-interaction-states)

Starting at the old position, take the first subsequent Tab stop that survives and is still eligible. If none follows it, take the nearest preceding eligible stop. If there are no eligible stops, the window has no focused node. Perform recovery as part of applying the change, before another input can be dispatched. A move that preserves the eligibility of the focused node keeps its identity. [Retained node identity](0032-owned-generational-ui-node-identity.md) · [Deferred handler mutations](0029-deferred-event-handler-mutations.md)

For stops A, B and C in traversal order, removing focused B moves focus to C. Disabling focused C moves focus to B if B is still eligible. Suppressing all three leaves focus empty. Normal Tab traversal may wrap at the end of a window. Recovery at the end goes back to the preceding control.

Clearing focus until the next Tab would avoid choosing a new target on the player's behalf, at the cost of an extra navigation input. Restarting at the first stop would also discard the player's place. Immediate recovery keeps a visible position for keyboard navigation. An activation that was armed on the previous target is canceled, so its eventual key release cannot activate the neighbor.
