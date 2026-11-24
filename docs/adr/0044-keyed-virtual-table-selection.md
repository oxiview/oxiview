---
status: accepted
---

# Keep virtual table selections by row and column key

A reader can extend a rectangular selection beyond the edge of the M7 table and copy it after scrolling. Virtualization releases the Views for cells that leave the mounted range. Sorting can also place a different record at the same grid position. The owner chose to retain the whole selection through scrolling and column resizing, and to collapse it to the original active cell when sorting or filtering. [M7 reader journey](../plans/m7-widgets-and-scrolling.md#reader-journey) · [Retained node identity](0032-owned-generational-ui-node-identity.md)

The table stores the anchor and active end as pairs of stable row and column keys. Both refer to the ordered data model. When Copy runs, resolve the rectangle against a single revision and obtain the cells' text in displayed order. The collection keeps keyboard focus while individual row and cell Views enter or leave the viewport.

After changing the sort or filter, look up the active pair in the new order. If both keys are visible in that result, make the pair the whole selection. If either key has left the result, clear the selection. For example, extending from record 17's severity to record 25's message makes the latter cell active. Sorting by severity keeps record 25's message selected at its new position. Filtering out record 25 clears the selection and disables Copy. In this rule, a key may belong to the filtered result while being far outside the viewport.

Screen-coordinate selection can move a copy operation to different records when the order changes. Keeping all the original keys would preserve membership across sorting but could yield disjoint areas, which would need a new clipboard representation. The chosen policy preserves a single rectangle and one identifiable destination for the reader's next keyboard action. The M5 focus-recovery rule still applies if the collection itself is removed or disabled. [Focus recovery](0041-focus-recovery-in-tab-order.md)
