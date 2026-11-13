---
status: accepted
---

# Accessibility: defer implementation while reserving semantic extension points in the node tree

Accessibility integration (NSAccessibility / UI Automation / AT-SPI) is **deferred beyond early milestones**. Initial resources prioritize rendering, text, and input.

## Consequences

- Risk: adding accessibility later is usually costly because every widget needs semantics, focus, and actions, requiring rework. Reduce that cost by reserving semantic fields (role, label, value, supported Actions) in the node tree (ADR-0007) and widget traits from the start, while deferring platform integration.
- Evaluate AccessKit first when implementation begins. The owned platform layer (ADR-0002) requires corresponding AccessKit adapters.
- Revisit and supersede this decision with a new ADR before an official release for external users.
