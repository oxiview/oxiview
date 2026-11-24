---
status: accepted
---

# Pass unused scroll distance to enclosing viewports

A scroll gesture can start over a detail pane inside a larger scrollable view. The inner pane can reach its end partway through a single input update. The owner chose to continue scrolling the enclosing viewport and to stop at the outermost boundary. [M7 design](../plans/m7-widgets-and-scrolling.md#scroll-and-identity-contracts)

For each axis, the innermost eligible viewport clamps its proposed offset to the content extent. Subtract its actual movement from the delivered distance and offer the remainder to the next eligible ancestor. For example, if the inner pane can move 10 more logical units and receives 24, it consumes 10 and its parent receives 14. The outermost viewport ends at its last valid content position. A horizontal remainder can reach a different eligible ancestor from a vertical remainder.

Direct input and momentum follow the same consumption rule. A phased gesture and its momentum share an originating viewport path. A fresh gesture chooses its path from a fresh pointer hit. The input source identifies who supplies momentum. Native momentum updates are already movement. A source that delegates deceleration to the framework advances it on the window's frame clock. This gives one producer for each movement and lets headless replay the same distances. [Frame scheduling](0004-frame-scheduling.md) · [Capture and hover](0028-pointer-capture-ownership-and-lifecycle.md)

Keeping an entire gesture inside the inner pane would require the reader to start another gesture to continue scrolling the page. Applying the full distance at both levels would move the page before the inner pane had consumed its share. Elastic overscroll would introduce visible positions beyond the content extent and a return motion at the outer boundary. Each choice changes hit testing, scrollbar position and what the reader sees during momentum.
