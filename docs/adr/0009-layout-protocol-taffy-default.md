---
status: accepted
---

# Layout: an owned layout protocol trait with Taffy as the default implementation

The framework defines its own **layout protocol**: a trait covering size constraints, measurement, positioning, caching, and invalidation. The default implementation uses Taffy for Flex, Grid, and Block layout, with text participating through measure callbacks. Above the protocol, the framework adds Stack, absolute positioning, virtual lists, and virtual tables, and permits custom layouts. Isolating Taffy allows full or partial replacement when performance or semantics are insufficient, such as single-pass constraint layout or very large virtualization, without changing widget code.

## Consequences

- Widgets depend only on the layout protocol and cannot reference Taffy types directly.
- Layout caching integrates with ADR-0007's layout dirty flags and boundaries; the protocol must expose whether a node's size is independent of its content.
