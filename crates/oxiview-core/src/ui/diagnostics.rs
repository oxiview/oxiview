use oxiview_layout::LayoutStats;

/// Cumulative completed UI preparation work for one tree.
/// Reading a snapshot performs no layout, painting, timing, or scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiDiagnostics {
    /// Successful layout computations whose staged geometry was committed.
    pub layout_passes: u64,
    /// Uncached node-layout evaluations reported by successful layout passes.
    pub computed_layout_nodes: u64,
    /// Intrinsic measurement calls reported by successful layout passes.
    pub measured_nodes: u64,
    /// Node-layout cache hits reported by successful layout passes.
    pub layout_cache_hits: u64,
    /// Successfully built complete Scenes, independent of presentation outcome.
    pub scene_builds: u64,
    /// Paint calls that reused a previously built complete Scene.
    pub scene_cache_hits: u64,
}

impl UiDiagnostics {
    pub(super) fn record_layout(&mut self, stats: LayoutStats) {
        self.layout_passes = self.layout_passes.saturating_add(1);
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
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
