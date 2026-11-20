use oxiview_layout::LayoutStats;

/// Cumulative UI preparation and pointer dispatch work for one tree.
/// Reading a snapshot performs no layout, painting, timing, or scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiDiagnostics {
    /// Successful layout computations whose staged geometry was committed.
    pub layout_passes: u64,
    /// Nodes inspected while synchronizing inputs in successful layout passes.
    pub layout_input_visits: u64,
    /// Style projections requested by successful layout passes.
    pub resolved_styles: u64,
    /// Layout algorithm entry calls, including cache hits and hidden nodes.
    pub layout_node_visits: u64,
    /// Uncached node-layout evaluations reported by successful layout passes.
    pub computed_layout_nodes: u64,
    /// Intrinsic measurement calls reported by successful layout passes.
    pub measured_nodes: u64,
    /// Node-layout cache hits reported by successful layout passes.
    pub layout_cache_hits: u64,
    /// Complete Scenes successfully assembled from the UI tree.
    pub scene_builds: u64,
    /// Successful paint batches that reused Scene structure for draw edits.
    pub scene_updates: u64,
    /// Paint calls that returned a cached Scene with no pending edits.
    pub scene_cache_hits: u64,
    /// Nodes emitted during complete Scene assembly or checked for sparse draw edits.
    /// Only successful paint batches contribute.
    pub paint_node_visits: u64,
    /// Node painting preparations used by successfully built complete Scenes.
    /// Reassembling a Scene from retained node painting does not count.
    pub painted_nodes: u64,
    /// Node-owned pointer callbacks invoked, including boundary and cancel callbacks.
    /// Route observers and batch report callbacks are excluded.
    pub dispatched_handlers: u64,
}

impl UiDiagnostics {
    pub(super) fn record_layout(&mut self, stats: LayoutStats) {
        self.layout_passes = self.layout_passes.saturating_add(1);
        self.layout_input_visits = self
            .layout_input_visits
            .saturating_add(count(stats.visited_nodes));
        self.resolved_styles = self
            .resolved_styles
            .saturating_add(count(stats.resolved_styles));
        self.layout_node_visits = self
            .layout_node_visits
            .saturating_add(count(stats.layout_visits));
        self.computed_layout_nodes = self
            .computed_layout_nodes
            .saturating_add(count(stats.computed_nodes));
        self.measured_nodes = self
            .measured_nodes
            .saturating_add(count(stats.measured_nodes));
        self.layout_cache_hits = self
            .layout_cache_hits
            .saturating_add(count(stats.cache_hits));
    }

    pub(super) fn record_scene_build(&mut self, visits: usize, painted_nodes: usize) {
        self.scene_builds = self.scene_builds.saturating_add(1);
        self.paint_node_visits = self.paint_node_visits.saturating_add(count(visits));
        self.painted_nodes = self.painted_nodes.saturating_add(count(painted_nodes));
    }

    pub(super) fn record_scene_update(&mut self, visits: usize) {
        self.scene_updates = self.scene_updates.saturating_add(1);
        self.paint_node_visits = self.paint_node_visits.saturating_add(count(visits));
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
