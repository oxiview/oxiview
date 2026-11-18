# oxiview-layout

Language: **English**

The owned layout port borrows its caller's retained tree. `TaffyLayout` implements
Flex, Grid, and Block layout without a second authoritative `TaffyTree`.
Style values, constraints, measurements, geometry, and errors crossing the port
use framework-owned types. The root counts as level 1 of the 64-level limit.

Private per-node caches persist across successful compute passes. Cache
fingerprints include layout inputs, parent identity, ordered child identities,
and `LayoutTree::content_revision`. Changed nodes invalidate their ancestors;
reparenting and changes to display/positioning conservatively invalidate the
affected subtree. Other branches can reuse Taffy's final-layout and measurement
entries under matching constraints. Dirty-tree validation still scans the
current caller topology, so cache reuse does not imply zero topology visits.

Caller identities must distinguish logical nodes and tree instances, or the
caller must use `clear_cache` before reusing a namespace. Measurement revisions
must change whenever callback replacement or captured content can change the
measurement result. The default revision is for immutable measurement content.
Core supplies generational IDs and checked measurement revisions.

Private Taffy identities remain stable across passes because cached out-of-flow
candidates contain those identities. They map to each pass's current entry
indices; cached hoisted children and saved geometry remain valid after reordering.
The cached child list is only an invalidation fingerprint; traversal always uses
the caller's authoritative child slices. Removed records are discarded after
the next successful pass. Failed passes publish neither new geometry nor caches.

Tests compare cached geometry against fresh computation after changes to
measurement data, percentages, constraints, padding/borders, layout families,
visibility, positioning, child order, reparenting, removal/reintroduction, and
120 deterministic generated edits. Measurement counters verify unchanged branch
reuse in Flex/Grid/Block layouts. These tests establish the covered cases, not
an arbitrary-depth guarantee or completed performance-baseline acceptance.
