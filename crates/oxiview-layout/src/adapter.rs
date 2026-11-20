use crate::*;
use oxiview_base::{Point, Size};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::Hash,
    rc::Rc,
    sync::Arc,
};
use taffy::{CacheTree, LayoutPartialTree as _, NodeId, TraversePartialTree};

/// Layout implementation that borrows the caller's topology.
///
/// The adapter retains resolved inputs and results under stable private keys.
/// A pass stages its input, cache, and geometry changes until it succeeds.
#[derive(Debug)]
pub struct TaffyLayout<I> {
    root: Option<I>,
    viewport: Size,
    ids: HashMap<I, NodeId>,
    nodes: HashMap<NodeId, Retained<I>>,
    next_identity: usize,
}

impl<I> Default for TaffyLayout<I> {
    fn default() -> Self {
        Self {
            root: None,
            viewport: Size::default(),
            ids: HashMap::new(),
            nodes: HashMap::new(),
            next_identity: 0,
        }
    }
}

impl<I: Copy + Eq + Hash> TaffyLayout<I> {
    /// Creates an adapter with an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Discards all retained results and their private identity mappings.
    pub fn clear_cache(&mut self) {
        *self = Self::default();
    }

    /// Number of live-node cache records retained by the last successful pass.
    pub fn cached_node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn committed_layout_boundary(&self, id: I) -> Option<bool> {
        let root = *self.ids.get(&self.root?)?;
        let key = *self.ids.get(&id)?;
        let node = self.nodes.get(&key)?;
        Some(
            key != root
                && (node.is_suppressed_boundary() || self.viewport_boundary(root, node).is_some()),
        )
    }

    fn viewport_boundary(
        &self,
        root: NodeId,
        node: &Retained<I>,
    ) -> Option<(taffy::LayoutInput, Geometry)> {
        // The viewport positions fixed-size absolute boxes without measuring
        // them or merging their overflow into the root's layout output. The
        // absolute box itself contains any absolute descendants.
        if node.geometry.hidden
            || node.geometry.hoisted_by != Some(root)
            || self.nodes.get(&root)?.inputs.style.position != Position::Static
            || node.inputs.style.position != Position::Absolute
            || !matches!(node.inputs.style.width, Length::Points(_))
            || !matches!(node.inputs.style.height, Length::Points(_))
        {
            return None;
        }
        let (input, output) = node.cache.layout.as_ref()?;
        output
            .oof_candidates
            .is_empty()
            .then_some((*input, node.geometry))
    }

    /// Validates the complete caller tree and publishes geometry for every node.
    ///
    /// Use this entry point for external trees without a mutation journal. Style,
    /// child order, structural parent and measurement revisions are compared to
    /// the last successful pass to identify reusable results.
    pub fn compute<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        self.compute_with_styles(tree, root, viewport, None, &mut HashMap::new())
    }

    fn compute_with_styles<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
        changes: Option<&HashMap<I, LayoutChange<I>>>,
        styles: &mut TaffyStyles,
    ) -> Result<LayoutStats, LayoutError> {
        if self.root.is_some_and(|previous| previous != root) {
            let mut fresh = Self::new();
            let mut fresh_styles = HashMap::new();
            let stats = fresh.run(tree, root, viewport, changes, &mut fresh_styles)?;
            *self = fresh;
            *styles = fresh_styles;
            return Ok(stats);
        }
        self.run(tree, root, viewport, changes, styles)
    }

    fn run<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
        changes: Option<&HashMap<I, LayoutChange<I>>>,
        styles: &mut TaffyStyles,
    ) -> Result<LayoutStats, LayoutError> {
        if ![viewport.width(), viewport.height()]
            .into_iter()
            .all(valid_extent)
        {
            return Err(LayoutError::InvalidViewport);
        }
        let viewport_changed = self.viewport != viewport;
        let track_order = changes.is_none();
        let changes = changes.filter(|_| self.root.is_some());
        let mut adapter = Adapter::new(self, tree, styles);
        {
            let _span = tracing::trace_span!("ui_layout_inputs").entered();
            if let Some(changes) = changes {
                adapter.sync_changes(changes, root, viewport_changed)?;
            } else {
                adapter.sync_tree(root, track_order)?;
            }
        }
        let root_key = adapter.key(root).ok_or(LayoutError::InvalidTree)?;
        let boundary = changes
            .filter(|_| !viewport_changed)
            .and_then(|changes| adapter.viewport_boundary(root_key, changes));
        adapter.invalidate(root_key, boundary.map(|(id, _, _)| id))?;
        if viewport_changed {
            // Taffy skips reporting viewport-relative hoists on a root cache
            // hit. Re-run the root so every placed box records its owner.
            adapter.updates.dirty.insert(root_key);
        }
        {
            let _span = tracing::info_span!("ui_layout").entered();
            if !adapter.updates.dirty.is_empty() {
                adapter.resolve_taffy_styles();
            }
            if adapter.updates.dirty.contains(&root_key)
                && adapter.style(root_key).display == taffy::Display::None
            {
                adapter.compute_child_layout(root_key, taffy::LayoutInput::HIDDEN);
            } else if adapter.updates.dirty.contains(&root_key) {
                taffy::compute_root_layout(
                    &mut adapter,
                    root_key,
                    taffy::Size {
                        width: taffy::AvailableSpace::Definite(viewport.width() as f32),
                        height: taffy::AvailableSpace::Definite(viewport.height() as f32),
                    },
                );
            } else if let Some((id, input, mut geometry)) =
                boundary.filter(|(id, _, _)| adapter.updates.dirty.contains(id))
            {
                let output = adapter.compute_child_layout(id, input);
                geometry.raw.scrollable_overflow_rect = output.scrollable_overflow_rect;
                adapter.updates.geometry.insert(id, geometry);
            } else if adapter.updates.dirty.is_empty() && changes.is_none() {
                // A validated, unchanged external tree gets every box from
                // the previously completed layout without running Taffy.
                adapter.stats.cache_hits += 1;
            }
            let mut hidden_boundaries: Vec<_> = adapter.hidden_boundaries.drain().collect();
            hidden_boundaries.sort_unstable_by_key(|id| usize::from(*id));
            for id in hidden_boundaries {
                if !adapter
                    .updates
                    .geometry
                    .get(&id)
                    .is_some_and(|geometry| geometry.hidden)
                {
                    taffy::LayoutPartialTree::compute_child_layout(
                        &mut adapter,
                        id,
                        taffy::LayoutInput::HIDDEN,
                    );
                }
            }
        }
        let stats = adapter.stats;
        let converted_styles = adapter.updates.styles.len();
        let changes = adapter.finish()?;
        changes.commit(self, styles);
        self.root = Some(root);
        self.viewport = viewport;
        tracing::trace!(
            input_visits = stats.visited_nodes,
            resolved_styles = stats.resolved_styles,
            converted_styles,
            layout_visits = stats.layout_visits,
            computed_nodes = stats.computed_nodes,
            cache_hits = stats.cache_hits,
            measured_nodes = stats.measured_nodes,
            "layout work"
        );
        Ok(stats)
    }
}

// Taffy's converted Style contains pointer-backed values and is thread-local.
type TaffyStyles = HashMap<NodeId, Rc<taffy::Style>>;

/// Window-thread Taffy adapter with retained converted styles.
#[derive(Debug)]
pub struct IncrementalTaffyLayout<I> {
    layout: TaffyLayout<I>,
    styles: TaffyStyles,
}

impl<I> Default for IncrementalTaffyLayout<I> {
    fn default() -> Self {
        Self {
            layout: TaffyLayout::default(),
            styles: HashMap::new(),
        }
    }
}

impl<I: Copy + Eq + Hash> IncrementalTaffyLayout<I> {
    /// Discards all retained results, styles, and private identity mappings.
    pub fn clear_cache(&mut self) {
        *self = Self::default();
    }

    /// Number of live-node cache records retained by the last successful pass.
    pub fn cached_node_count(&self) -> usize {
        self.layout.cached_node_count()
    }

    /// Whether the last committed layout contains this node's identity.
    pub fn contains_cached_node(&self, id: I) -> bool {
        self.layout.ids.contains_key(&id)
    }

    /// Reports a node's boundary status under the last committed constraints.
    /// See [`LayoutEngine::layout_boundary`] for the meaning of the result.
    pub fn layout_boundary(&self, id: I) -> Option<bool> {
        self.layout.committed_layout_boundary(id)
    }

    /// Computes layout using mutations since this adapter's last successful pass.
    ///
    /// Maintain a valid rooted tree, report both changed child lists for a
    /// move, and report every changed input and removed node. Keep the journal
    /// on failure and discard it on success. An empty cache or newly selected
    /// root bootstraps with a full traversal. Subsequent passes publish only
    /// touched geometry. A viewport change also refreshes the root's projection.
    /// Use [TaffyLayout::compute] when the caller does not supply a complete
    /// journal or needs every box republished.
    pub fn compute<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
        changes: &HashMap<I, LayoutChange<I>>,
    ) -> Result<LayoutStats, LayoutError> {
        self.layout
            .compute_with_styles(tree, root, viewport, Some(changes), &mut self.styles)
    }
}

fn valid_extent(value: f64) -> bool {
    value.is_finite() && value >= 0.0 && value <= f64::from(f32::MAX)
}

fn convert_edges(v: taffy::Rect<f32>) -> Edges<f64> {
    Edges {
        left: f64::from(v.left),
        right: f64::from(v.right),
        top: f64::from(v.top),
        bottom: f64::from(v.bottom),
    }
}

#[derive(Debug)]
struct Inputs<I> {
    id: I,
    // The dependency used to propagate invalidation and project hoisted geometry.
    parent: Option<I>,
    // Only the scanning adapter needs to detect order changes without a journal.
    sibling_index: Option<usize>,
    style: Arc<LayoutInput>,
    revision: u64,
}

#[derive(Debug)]
struct Retained<I> {
    inputs: Inputs<I>,
    cache: NodeCache,
    geometry: Geometry,
}

impl<I> Retained<I> {
    fn is_suppressed_boundary(&self) -> bool {
        self.geometry.hidden && self.inputs.style.display == Display::None
    }
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    raw: taffy::Layout,
    hoisted_by: Option<NodeId>,
    hidden: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            raw: taffy::Layout::new(),
            hoisted_by: None,
            hidden: false,
        }
    }
}

// Keep the complete constraint in both caches. In particular, Taffy's
// size-only key ignores the parent's height, which can resolve percentages,
// and can merge available-space values visible to custom measurement.
#[derive(Clone, Debug, Default)]
struct NodeCache {
    layout: Option<(taffy::LayoutInput, taffy::LayoutOutput)>,
    sizes: VecDeque<(taffy::LayoutInput, taffy::LayoutOutput)>,
}

impl NodeCache {
    fn get(&self, input: &taffy::LayoutInput) -> Option<taffy::LayoutOutput> {
        let entry = match input.run_mode {
            taffy::RunMode::PerformLayout => self.layout.as_ref().filter(|(key, _)| key == input),
            taffy::RunMode::ComputeSize => self.sizes.iter().find(|(key, _)| key == input),
            taffy::RunMode::PerformHiddenLayout => None,
        };
        entry.map(|(_, output)| output.clone())
    }

    fn store(&mut self, input: taffy::LayoutInput, output: taffy::LayoutOutput) {
        match input.run_mode {
            taffy::RunMode::PerformLayout => self.layout = Some((input, output)),
            taffy::RunMode::ComputeSize => {
                const MAX_SIZES: usize = 9;
                if self.sizes.len() == MAX_SIZES {
                    self.sizes.pop_front();
                }
                self.sizes.push_back((input, output));
            }
            taffy::RunMode::PerformHiddenLayout => {}
        }
    }
}

struct Updates<I> {
    inputs: HashMap<NodeId, Inputs<I>>,
    added: HashMap<I, NodeId>,
    removed: HashSet<I>,
    next_identity: usize,
    dirty: HashSet<NodeId>,
    caches: HashMap<NodeId, NodeCache>,
    geometry: HashMap<NodeId, Geometry>,
    styles: TaffyStyles,
}

impl<I: Copy + Eq + Hash> Updates<I> {
    fn commit(self, layout: &mut TaffyLayout<I>, styles: &mut TaffyStyles) {
        for id in self.removed {
            if let Some(key) = layout.ids.remove(&id) {
                layout.nodes.remove(&key);
                styles.remove(&key);
            }
        }
        layout.ids.extend(self.added);
        styles.extend(self.styles);
        for (key, inputs) in self.inputs {
            if let Some(node) = layout.nodes.get_mut(&key) {
                node.inputs = inputs;
            } else {
                layout.nodes.insert(
                    key,
                    Retained {
                        inputs,
                        cache: NodeCache::default(),
                        geometry: Geometry::default(),
                    },
                );
            }
        }
        for key in self.dirty {
            if let Some(node) = layout.nodes.get_mut(&key) {
                node.cache = NodeCache::default();
            }
        }
        for (key, cache) in self.caches {
            if let Some(node) = layout.nodes.get_mut(&key) {
                node.cache = cache;
            }
        }
        for (key, geometry) in self.geometry {
            if let Some(node) = layout.nodes.get_mut(&key) {
                node.geometry = geometry;
            }
        }
        layout.next_identity = self.next_identity;
    }
}

struct Adapter<'a, T: LayoutTree> {
    retained: &'a TaffyLayout<T::NodeId>,
    styles: &'a TaffyStyles,
    tree: &'a mut T,
    updates: Updates<T::NodeId>,
    full_order: Vec<T::NodeId>,
    hidden_boundaries: HashSet<NodeId>,
    default_style: taffy::Style,
    stats: LayoutStats,
    error: Option<LayoutError>,
    depth: usize,
}

impl<T: LayoutTree> Adapter<'_, T> {
    fn new<'a>(
        retained: &'a TaffyLayout<T::NodeId>,
        tree: &'a mut T,
        styles: &'a TaffyStyles,
    ) -> Adapter<'a, T> {
        Adapter {
            retained,
            styles,
            tree,
            updates: Updates {
                inputs: HashMap::new(),
                added: HashMap::new(),
                removed: HashSet::new(),
                next_identity: retained.next_identity,
                dirty: HashSet::new(),
                caches: HashMap::new(),
                geometry: HashMap::new(),
                styles: HashMap::new(),
            },
            full_order: Vec::new(),
            hidden_boundaries: HashSet::new(),
            default_style: taffy::Style::default(),
            stats: LayoutStats::default(),
            error: None,
            depth: 0,
        }
    }

    fn sync_tree(&mut self, root: T::NodeId, track_order: bool) -> Result<(), LayoutError> {
        let mut seen = HashSet::new();
        let mut stack = vec![(root, None, None, 1)];
        while let Some((id, parent, sibling_index, depth)) = stack.pop() {
            if depth > MAX_LAYOUT_DEPTH {
                return Err(LayoutError::ExcessiveDepth);
            }
            if !seen.insert(id) {
                return Err(LayoutError::InvalidTree);
            }
            self.sync_node(id, parent, sibling_index, true, false)?;
            self.full_order.push(id);
            stack.extend(
                self.tree
                    .children(id)
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(index, &child)| {
                        (child, Some(id), track_order.then_some(index), depth + 1)
                    }),
            );
        }
        self.updates
            .removed
            .extend(self.retained.ids.keys().filter(|id| !seen.contains(id)));
        Ok(())
    }

    fn sync_changes(
        &mut self,
        changes: &HashMap<T::NodeId, LayoutChange<T::NodeId>>,
        root: T::NodeId,
        viewport_changed: bool,
    ) -> Result<(), LayoutError> {
        if !self.tree.contains_node(root) {
            return Err(LayoutError::InvalidTree);
        }
        for (&id, &change) in changes {
            match change {
                LayoutChange::Update {
                    parent,
                    style,
                    children,
                } => {
                    if (id == root) != parent.is_none() {
                        return Err(LayoutError::InvalidTree);
                    }
                    self.sync_node(
                        id,
                        parent,
                        None,
                        style || (viewport_changed && id == root),
                        children,
                    )?;
                }
                LayoutChange::Remove => {
                    self.updates.removed.insert(id);
                }
            }
        }
        if viewport_changed && !changes.contains_key(&root) {
            self.sync_node(root, None, None, true, false)?;
        }
        let mut attached = HashSet::new();
        let mut ancestors = Vec::new();
        for (&key, inputs) in &self.updates.inputs {
            let Some(parent) = inputs.parent else {
                continue;
            };
            let previous = self
                .retained
                .nodes
                .get(&key)
                .and_then(|node| node.inputs.parent);
            if previous == Some(parent) {
                continue;
            }
            if let Some(previous) = previous
                && !matches!(
                    changes.get(&previous),
                    Some(LayoutChange::Update { children: true, .. } | LayoutChange::Remove)
                )
            {
                return Err(LayoutError::InvalidTree);
            }
            // A disconnected cycle can have locally consistent child lists.
            ancestors.clear();
            ancestors.push(inputs.id);
            let mut ancestor = parent;
            while ancestor != root {
                if ancestors.contains(&ancestor) {
                    return Err(LayoutError::InvalidTree);
                }
                if ancestors.len() >= MAX_LAYOUT_DEPTH - 1 {
                    return Err(LayoutError::ExcessiveDepth);
                }
                ancestors.push(ancestor);
                ancestor = self
                    .key(ancestor)
                    .and_then(|key| self.inputs(key))
                    .and_then(|node| node.parent)
                    .ok_or(LayoutError::InvalidTree)?;
            }
            attached.insert(inputs.id);
        }
        for (&id, &change) in changes {
            if let LayoutChange::Update { children: true, .. } = change {
                let mut seen = HashSet::new();
                for &child in self.tree.children(id) {
                    let child_inputs = self.key(child).and_then(|key| self.inputs(key));
                    if !seen.insert(child)
                        || !self.tree.contains_node(child)
                        || child_inputs.is_none_or(|node| node.parent != Some(id))
                    {
                        return Err(LayoutError::InvalidTree);
                    }
                    attached.remove(&child);
                }
            }
        }
        if !attached.is_empty() {
            return Err(LayoutError::InvalidTree);
        }
        Ok(())
    }

    fn sync_node(
        &mut self,
        id: T::NodeId,
        parent: Option<T::NodeId>,
        sibling_index: Option<usize>,
        style_changed: bool,
        children_changed: bool,
    ) -> Result<(), LayoutError> {
        self.stats.visited_nodes += 1;
        if !self.tree.contains_node(id) {
            return Err(LayoutError::InvalidTree);
        }
        let old_key = self.retained.ids.get(&id).copied();
        let old = old_key
            .and_then(|key| self.retained.nodes.get(&key))
            .map(|node| &node.inputs);
        let input = if style_changed || old.is_none() {
            self.stats.resolved_styles += 1;
            let input = self.tree.layout_input(id);
            input.validate()?;
            Some(input)
        } else {
            None
        };
        let revision = self.tree.content_revision(id);
        if !children_changed
            && old.is_some_and(|old| {
                old.parent == parent
                    && old.sibling_index == sibling_index
                    && old.revision == revision
                    && input.as_ref().is_none_or(|input| *input == *old.style)
            })
        {
            return Ok(());
        }
        let style = match (input, old) {
            (Some(input), Some(old)) if input == *old.style => Arc::clone(&old.style),
            (Some(input), _) => Arc::new(input),
            (None, Some(old)) => Arc::clone(&old.style),
            (None, None) => return Err(LayoutError::InvalidTree),
        };
        let key = if let Some(key) = old_key {
            key
        } else {
            let next = self.updates.next_identity;
            self.updates.next_identity = next.checked_add(1).ok_or(LayoutError::CacheExhausted)?;
            let key = NodeId::from(next);
            self.updates.added.insert(id, key);
            key
        };
        self.updates.inputs.insert(
            key,
            Inputs {
                id,
                parent,
                sibling_index,
                style,
                revision,
            },
        );
        Ok(())
    }

    fn viewport_boundary(
        &self,
        root: NodeId,
        changes: &HashMap<T::NodeId, LayoutChange<T::NodeId>>,
    ) -> Option<(NodeId, taffy::LayoutInput, Geometry)> {
        // Replay the final layout only when the journal leaves the boundary
        // and its incoming constraints unchanged.
        if changes.values().any(|change| {
            !matches!(
                change,
                LayoutChange::Update {
                    children: false,
                    ..
                }
            )
        }) {
            return None;
        }
        let mut boundary = None;
        for (&key, inputs) in &self.updates.inputs {
            let old = &self.retained.nodes.get(&key)?.inputs;
            if old.parent != inputs.parent
                || old.style.position != inputs.style.position
                || old.style.display != inputs.style.display
            {
                return None;
            }
            let mut current = inputs.parent;
            let mut found = None;
            for _ in 0..MAX_LAYOUT_DEPTH {
                let ancestor = self.key(current?)?;
                if ancestor == root || self.updates.inputs.contains_key(&ancestor) {
                    // A changed ancestor can affect the replay constraints.
                    return None;
                }
                let node = self.retained.nodes.get(&ancestor)?;
                if node.geometry.hidden {
                    return None;
                }
                if let Some((input, geometry)) = self.retained.viewport_boundary(root, node) {
                    found = Some((ancestor, input, geometry));
                    break;
                }
                current = node.inputs.parent;
            }
            let found = found?;
            if boundary.is_some_and(|(id, _, _)| id != found.0) {
                return None;
            }
            boundary = Some(found);
        }
        boundary
    }

    fn invalidate(&mut self, root: NodeId, boundary: Option<NodeId>) -> Result<(), LayoutError> {
        let mut reset_subtrees = Vec::new();
        let mut paths = Vec::with_capacity(self.updates.inputs.len());
        for &id in &self.updates.removed {
            if let Some(parent) = self
                .retained
                .ids
                .get(&id)
                .and_then(|key| self.retained.nodes.get(key))
                .and_then(|old| old.inputs.parent)
                .and_then(|parent| self.key(parent))
            {
                paths.push(parent);
            }
        }
        for (&key, inputs) in &self.updates.inputs {
            if let Some(old) = self.retained.nodes.get(&key) {
                if old.inputs.sibling_index != inputs.sibling_index
                    && old.inputs.parent == inputs.parent
                    && old.inputs.revision == inputs.revision
                    && Arc::ptr_eq(&old.inputs.style, &inputs.style)
                {
                    // A reorder changes the parent's algorithm inputs only.
                    let parent = inputs.parent.and_then(|id| self.key(id));
                    paths.push(parent.ok_or(LayoutError::InvalidTree)?);
                    continue;
                }
                if old.inputs.parent != inputs.parent
                    || old.inputs.style.display != inputs.style.display
                    || old.inputs.style.position != inputs.style.position
                {
                    reset_subtrees.push(key);
                }
                if old.inputs.parent != inputs.parent
                    && let Some(old_parent) = old.inputs.parent.and_then(|id| self.key(id))
                {
                    paths.push(old_parent);
                }
            }
            paths.push(key);
        }
        for key in paths {
            let mut current = key;
            for depth in 1..=MAX_LAYOUT_DEPTH {
                if !self.updates.dirty.insert(current)
                    || current == root
                    || Some(current) == boundary
                {
                    break;
                }
                let inputs = self.inputs(current).ok_or(LayoutError::InvalidTree)?;
                let parent = inputs
                    .parent
                    .and_then(|id| self.key(id))
                    .ok_or(LayoutError::InvalidTree)?;
                // A still-suppressed box contributes nothing to its parent.
                if inputs.style.display == Display::None
                    && self.retained.nodes.get(&current).is_some_and(|old| {
                        old.is_suppressed_boundary() && old.inputs.parent == inputs.parent
                    })
                {
                    self.hidden_boundaries.insert(current);
                    break;
                }
                if depth == MAX_LAYOUT_DEPTH {
                    return Err(LayoutError::ExcessiveDepth);
                }
                current = parent;
            }
        }
        // Moves and display/position transitions can change the context for
        // descendants without changing their own inputs. Keep their styles.
        let mut reset = HashSet::new();
        while let Some(key) = reset_subtrees.pop() {
            if reset.insert(key) {
                self.updates.dirty.insert(key);
                let node = self
                    .node(key)
                    .filter(|&node| self.tree.contains_node(node))
                    .ok_or(LayoutError::InvalidTree)?;
                for id in self.tree.children(node) {
                    reset_subtrees.push(self.key(*id).ok_or(LayoutError::InvalidTree)?);
                }
            }
        }
        Ok(())
    }

    fn key(&self, id: T::NodeId) -> Option<NodeId> {
        self.updates
            .added
            .get(&id)
            .or_else(|| {
                if self.updates.removed.contains(&id) {
                    None
                } else {
                    self.retained.ids.get(&id)
                }
            })
            .copied()
    }

    fn inputs(&self, key: NodeId) -> Option<&Inputs<T::NodeId>> {
        self.updates
            .inputs
            .get(&key)
            .or_else(|| self.retained.nodes.get(&key).map(|node| &node.inputs))
    }

    fn resolve_taffy_styles(&mut self) {
        for (&key, inputs) in &self.updates.inputs {
            let unchanged = self
                .retained
                .nodes
                .get(&key)
                .is_some_and(|old| Arc::ptr_eq(&old.inputs.style, &inputs.style));
            if !unchanged || !self.styles.contains_key(&key) {
                self.updates
                    .styles
                    .insert(key, Rc::new(convert_style(&inputs.style)));
            }
        }
        // The external adapter keeps its result cache portable. Its converted
        // styles are pass-local, and a clean pass needs none.
        if self.styles.is_empty() {
            for (&key, old) in &self.retained.nodes {
                if !self.updates.inputs.contains_key(&key)
                    && !self.updates.removed.contains(&old.inputs.id)
                {
                    self.updates
                        .styles
                        .insert(key, Rc::new(convert_style(&old.inputs.style)));
                }
            }
        }
    }

    fn converted_style(&self, id: NodeId) -> Option<&Rc<taffy::Style>> {
        self.updates
            .styles
            .get(&id)
            .or_else(|| self.styles.get(&id))
    }

    fn style(&self, id: NodeId) -> &taffy::Style {
        self.converted_style(id)
            .map_or(&self.default_style, Rc::as_ref)
    }

    fn node(&self, id: NodeId) -> Option<T::NodeId> {
        self.inputs(id).map(|inputs| inputs.id)
    }

    fn geometry(&self, id: NodeId) -> Option<&Geometry> {
        self.updates
            .geometry
            .get(&id)
            .or_else(|| self.retained.nodes.get(&id).map(|node| &node.geometry))
    }

    fn layout_box(&self, key: NodeId) -> Result<(T::NodeId, LayoutBox), LayoutError> {
        let inputs = self.inputs(key).ok_or(LayoutError::InvalidTree)?;
        if !self.tree.contains_node(inputs.id) {
            return Err(LayoutError::InvalidTree);
        }
        let geometry = self.geometry(key).ok_or(LayoutError::InvalidGeometry)?;
        let raw = geometry.raw;
        let mut x = f64::from(raw.location.x);
        let mut y = f64::from(raw.location.y);
        if let Some(owner) = geometry.hoisted_by {
            // An out-of-flow box is positioned relative to its containing
            // block. Subtract the intervening static ancestors' translations
            // to publish geometry relative to its structural parent.
            let mut current = inputs
                .parent
                .and_then(|id| self.key(id))
                .ok_or(LayoutError::InvalidTree)?;
            let mut depth = 1;
            while current != owner {
                if depth >= MAX_LAYOUT_DEPTH {
                    return Err(LayoutError::InvalidTree);
                }
                let parent_geometry = self.geometry(current).ok_or(LayoutError::InvalidGeometry)?;
                x -= f64::from(parent_geometry.raw.location.x);
                y -= f64::from(parent_geometry.raw.location.y);
                current = self
                    .inputs(current)
                    .and_then(|node| node.parent)
                    .and_then(|id| self.key(id))
                    .ok_or(LayoutError::InvalidTree)?;
                depth += 1;
            }
        }
        let overflow = raw.scrollable_overflow_rect;
        let content_width = f64::from(overflow.right - overflow.left);
        let content_height = f64::from(overflow.bottom - overflow.top);
        if !x.is_finite()
            || !y.is_finite()
            || ![
                raw.size.width,
                raw.size.height,
                raw.padding.left,
                raw.padding.right,
                raw.padding.top,
                raw.padding.bottom,
                raw.border.left,
                raw.border.right,
                raw.border.top,
                raw.border.bottom,
            ]
            .into_iter()
            .map(f64::from)
            .all(valid_extent)
            || ![content_width, content_height]
                .into_iter()
                .all(valid_extent)
        {
            return Err(LayoutError::InvalidGeometry);
        }
        Ok((
            inputs.id,
            LayoutBox {
                location: Point::new(x, y),
                size: Size::new(f64::from(raw.size.width), f64::from(raw.size.height)),
                content_size: Size::new(content_width, content_height),
                padding: convert_edges(raw.padding),
                border: convert_edges(raw.border),
            },
        ))
    }

    fn finish(mut self) -> Result<Updates<T::NodeId>, LayoutError> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let mut results = Vec::new();
        if self.full_order.is_empty() {
            results.reserve(self.updates.geometry.len());
            for &key in self.updates.geometry.keys() {
                results.push(self.layout_box(key)?);
            }
        } else {
            results.reserve(self.full_order.len());
            for &id in &self.full_order {
                let key = self.key(id).ok_or(LayoutError::InvalidTree)?;
                results.push(self.layout_box(key)?);
            }
        }
        for (id, geometry) in results {
            self.tree.set_layout(id, geometry);
        }
        Ok(self.updates)
    }
}

struct ChildIter<'a, T: LayoutTree> {
    children: std::slice::Iter<'a, T::NodeId>,
    adapter: &'a Adapter<'a, T>,
}

impl<T: LayoutTree> Iterator for ChildIter<'_, T> {
    type Item = NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.children.next().map(|&id| {
            self.adapter
                .key(id)
                .unwrap_or_else(|| NodeId::from(usize::MAX))
        })
    }
}

impl<T: LayoutTree> taffy::TraversePartialTree for Adapter<'_, T> {
    type ChildIter<'a>
        = ChildIter<'a, T>
    where
        Self: 'a;

    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        let children = self.node(id).map_or(&[][..], |id| self.tree.children(id));
        ChildIter {
            children: children.iter(),
            adapter: self,
        }
    }

    fn child_count(&self, id: NodeId) -> usize {
        self.node(id).map_or(0, |id| self.tree.children(id).len())
    }

    fn get_child_id(&self, id: NodeId, index: usize) -> NodeId {
        self.node(id)
            .and_then(|id| self.tree.children(id).get(index))
            .and_then(|id| self.key(*id))
            .unwrap_or_else(|| NodeId::from(usize::MAX))
    }
}

impl<T: LayoutTree> taffy::TraverseTree for Adapter<'_, T> {}

impl<T: LayoutTree> taffy::LayoutPartialTree for Adapter<'_, T> {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_core_container_style(&self, id: NodeId) -> Self::CoreContainerStyle<'_> {
        self.style(id)
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &taffy::Layout) {
        if self.inputs(id).is_none() {
            self.error.get_or_insert(LayoutError::InvalidTree);
            return;
        }
        self.updates.geometry.insert(
            id,
            Geometry {
                raw: *layout,
                ..Geometry::default()
            },
        );
    }

    fn resolve_calc_value(&self, _value: *const (), _basis: f32) -> f32 {
        0.0
    }

    fn compute_child_layout(
        &mut self,
        id: NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        self.stats.layout_visits += 1;
        if self.error.is_some() {
            return taffy::LayoutOutput::HIDDEN;
        }
        if self.depth >= MAX_LAYOUT_DEPTH {
            self.error = Some(LayoutError::ExcessiveDepth);
            return taffy::LayoutOutput::HIDDEN;
        }
        let Some(node) = self.node(id).filter(|&node| self.tree.contains_node(node)) else {
            self.error = Some(LayoutError::InvalidTree);
            return taffy::LayoutOutput::HIDDEN;
        };
        self.depth += 1;
        let display = self.style(id).display;
        let output = if inputs.run_mode == taffy::RunMode::PerformHiddenLayout
            || display == taffy::Display::None
        {
            self.hide(id)
        } else {
            taffy::compute_cached_layout(self, id, inputs, |tree, id, inputs| {
                tree.stats.computed_nodes += 1;
                let mut output = if tree.child_count(id) == 0 {
                    tree.compute_leaf(id, node, inputs)
                } else {
                    match display {
                        taffy::Display::Flex => taffy::compute_flexbox_layout(tree, id, inputs),
                        taffy::Display::Grid => taffy::compute_grid_layout(tree, id, inputs),
                        taffy::Display::Block | taffy::Display::FlowRoot => {
                            taffy::compute_block_layout(tree, id, inputs, None)
                        }
                        taffy::Display::None => taffy::LayoutOutput::HIDDEN,
                    }
                };
                if inputs.run_mode == taffy::RunMode::PerformLayout {
                    taffy::compute_oof_layout(tree, id, &mut output);
                }
                output
            })
        };
        self.depth -= 1;
        output
    }
}

impl<T: LayoutTree> Adapter<'_, T> {
    fn hide(&mut self, id: NodeId) -> taffy::LayoutOutput {
        let completed = self
            .updates
            .geometry
            .get(&id)
            .is_some_and(|geometry| geometry.hidden);
        let reused = !self.updates.dirty.contains(&id)
            && self
                .retained
                .nodes
                .get(&id)
                .is_some_and(|node| node.geometry.hidden);
        if completed || reused {
            if let Some(geometry) = self.updates.geometry.get_mut(&id) {
                geometry.hidden = true;
            }
            return taffy::LayoutOutput::HIDDEN;
        }
        self.stats.computed_nodes += 1;
        let output = taffy::compute_hidden_layout(self, id);
        if let Some(geometry) = self.updates.geometry.get_mut(&id) {
            geometry.hidden = true;
        }
        output
    }

    fn compute_leaf(
        &mut self,
        id: NodeId,
        node: T::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        let style = self.converted_style(id).cloned();
        let style = style.as_ref().map_or(&self.default_style, Rc::as_ref);
        taffy::compute_leaf_layout(
            inputs,
            style,
            |_, _| 0.0,
            |known, available| {
                if [
                    known.width,
                    known.height,
                    available.width.into_option(),
                    available.height.into_option(),
                ]
                .into_iter()
                .flatten()
                .any(|axis| !axis.is_finite())
                {
                    self.error = Some(LayoutError::InvalidGeometry);
                    return taffy::Size::ZERO;
                }
                self.stats.measured_nodes += 1;
                let measured = self.tree.measure(
                    node,
                    Dimensions {
                        width: known.width.map(f64::from),
                        height: known.height.map(f64::from),
                    },
                    Dimensions {
                        width: available_space(available.width),
                        height: available_space(available.height),
                    },
                );
                if ![measured.width, measured.height]
                    .into_iter()
                    .all(valid_extent)
                {
                    self.error = Some(LayoutError::InvalidGeometry);
                    return taffy::Size::ZERO;
                }
                taffy::Size {
                    width: measured.width as f32,
                    height: measured.height as f32,
                }
            },
        )
    }
}

impl<T: LayoutTree> CacheTree for Adapter<'_, T> {
    fn cache_get(&mut self, id: NodeId, input: &taffy::LayoutInput) -> Option<taffy::LayoutOutput> {
        let staged = self.updates.caches.get(&id);
        let cache = if staged.is_some() || self.updates.dirty.contains(&id) {
            staged
        } else {
            self.retained.nodes.get(&id).map(|node| &node.cache)
        };
        let result = cache.and_then(|cache| cache.get(input));
        if result.is_some() {
            self.stats.cache_hits += 1;
        }
        result
    }

    fn cache_store(&mut self, id: NodeId, input: &taffy::LayoutInput, output: taffy::LayoutOutput) {
        let cache = self.updates.caches.entry(id).or_insert_with(|| {
            if self.updates.dirty.contains(&id) {
                NodeCache::default()
            } else {
                self.retained
                    .nodes
                    .get(&id)
                    .map_or_else(NodeCache::default, |node| node.cache.clone())
            }
        });
        cache.store(*input, output);
    }

    fn cache_clear(&mut self, id: NodeId) {
        self.updates.caches.insert(id, NodeCache::default());
    }
}

impl<T: LayoutTree> taffy::LayoutContainingBlock for Adapter<'_, T> {
    type OofItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, id: NodeId) -> Self::OofItemStyle<'_> {
        self.style(id)
    }

    fn clear_hoisted_children(&mut self, _id: NodeId) {
        // Each raw layout write resets its owner. Record the new owner when
        // Taffy places that box; cached descendants retain their previous one.
    }

    fn add_hoisted_children(&mut self, id: NodeId, hoisted: &[NodeId]) {
        for child in hoisted {
            if let Some(geometry) = self.updates.geometry.get_mut(child) {
                geometry.hoisted_by = Some(id);
            } else {
                self.error.get_or_insert(LayoutError::InvalidTree);
            }
        }
    }
}

impl<T: LayoutTree> taffy::LayoutFlexboxContainer for Adapter<'_, T> {
    type FlexboxContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.style(id)
    }

    fn get_flexbox_child_style(&self, id: NodeId) -> Self::FlexboxItemStyle<'_> {
        self.style(id)
    }
}

impl<T: LayoutTree> taffy::LayoutGridContainer for Adapter<'_, T> {
    type GridContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: NodeId) -> Self::GridContainerStyle<'_> {
        self.style(id)
    }

    fn get_grid_child_style(&self, id: NodeId) -> Self::GridItemStyle<'_> {
        self.style(id)
    }
}

impl<T: LayoutTree> taffy::LayoutBlockContainer for Adapter<'_, T> {
    type BlockContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, id: NodeId) -> Self::BlockContainerStyle<'_> {
        self.style(id)
    }

    fn get_block_child_style(&self, id: NodeId) -> Self::BlockItemStyle<'_> {
        self.style(id)
    }
}

fn available_space(v: taffy::AvailableSpace) -> AvailableSpace {
    match v {
        taffy::AvailableSpace::Definite(v) => AvailableSpace::Definite(f64::from(v)),
        taffy::AvailableSpace::MinContent => AvailableSpace::MinContent,
        taffy::AvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
}
fn dimension(v: Length) -> taffy::Dimension {
    match v {
        Length::Auto => taffy::Dimension::auto(),
        Length::Points(v) => taffy::Dimension::length(v as f32),
        Length::Percent(v) => taffy::Dimension::percent(v as f32),
    }
}
fn length_auto(v: Length) -> taffy::LengthPercentageAuto {
    match v {
        Length::Auto => taffy::LengthPercentageAuto::auto(),
        Length::Points(v) => taffy::LengthPercentageAuto::length(v as f32),
        Length::Percent(v) => taffy::LengthPercentageAuto::percent(v as f32),
    }
}
fn length(v: Length) -> taffy::LengthPercentage {
    match v {
        Length::Auto => taffy::LengthPercentage::length(0.0),
        Length::Points(v) => taffy::LengthPercentage::length(v as f32),
        Length::Percent(v) => taffy::LengthPercentage::percent(v as f32),
    }
}
fn edges<A: Copy, B>(v: Edges<A>, convert: impl Fn(A) -> B) -> taffy::Rect<B> {
    taffy::Rect {
        left: convert(v.left),
        right: convert(v.right),
        top: convert(v.top),
        bottom: convert(v.bottom),
    }
}
fn align(v: Alignment) -> taffy::AlignItems {
    match v {
        Alignment::Start => taffy::AlignItems::START,
        Alignment::End => taffy::AlignItems::END,
        Alignment::Center => taffy::AlignItems::CENTER,
        Alignment::Stretch => taffy::AlignItems::STRETCH,
    }
}
fn content(v: ContentAlignment) -> taffy::AlignContent {
    match v {
        ContentAlignment::Normal => taffy::AlignContent::NORMAL,
        ContentAlignment::Start => taffy::AlignContent::START,
        ContentAlignment::End => taffy::AlignContent::END,
        ContentAlignment::Center => taffy::AlignContent::CENTER,
        ContentAlignment::Stretch => taffy::AlignContent::STRETCH,
        ContentAlignment::SpaceBetween => taffy::AlignContent::SPACE_BETWEEN,
        ContentAlignment::SpaceAround => taffy::AlignContent::SPACE_AROUND,
        ContentAlignment::SpaceEvenly => taffy::AlignContent::SPACE_EVENLY,
    }
}
fn track(v: TrackSize) -> taffy::TrackSizingFunction {
    use taffy::style_helpers::{auto, fr, length, minmax, percent};
    match v {
        TrackSize::Auto => auto(),
        TrackSize::Points(v) => length(v),
        TrackSize::Percent(v) => percent(v),
        TrackSize::Fr(v) => fr(v),
        TrackSize::MinMax(min, max) => {
            let min = match min {
                TrackBreadth::Auto => auto(),
                TrackBreadth::Points(v) => length(v),
                TrackBreadth::Percent(v) => percent(v),
                TrackBreadth::Fr(_) => auto(),
            };
            let max = match max {
                TrackBreadth::Auto => auto(),
                TrackBreadth::Points(v) => length(v),
                TrackBreadth::Percent(v) => percent(v),
                TrackBreadth::Fr(v) => fr(v),
            };
            minmax(min, max)
        }
    }
}
fn placement(v: GridPlacement) -> taffy::GridPlacement {
    match v {
        GridPlacement::Auto => taffy::GridPlacement::Auto,
        GridPlacement::Line(v) => taffy::style_helpers::line(v),
        GridPlacement::Span(v) => taffy::style_helpers::span(v),
    }
}
fn convert_style(v: &LayoutInput) -> taffy::Style {
    taffy::Style {
        display: match v.display {
            Display::Flex => taffy::Display::Flex,
            Display::Grid => taffy::Display::Grid,
            Display::Block => taffy::Display::Block,
            Display::None => taffy::Display::None,
        },
        position: match v.position {
            Position::Static => taffy::Position::Static,
            Position::Relative => taffy::Position::Relative,
            Position::Absolute => taffy::Position::Absolute,
        },
        size: taffy::Size {
            width: dimension(v.width),
            height: dimension(v.height),
        },
        min_size: taffy::Size {
            width: length_auto(v.min_width),
            height: length_auto(v.min_height),
        },
        max_size: taffy::Size {
            width: length_auto(v.max_width),
            height: length_auto(v.max_height),
        },
        aspect_ratio: v.aspect_ratio.map(|v| v as f32),
        inset: edges(v.inset, length_auto),
        margin: edges(v.margin, length_auto),
        padding: edges(v.padding, length),
        border: taffy::Rect {
            left: taffy::LengthPercentage::length(v.border_width as f32),
            right: taffy::LengthPercentage::length(v.border_width as f32),
            top: taffy::LengthPercentage::length(v.border_width as f32),
            bottom: taffy::LengthPercentage::length(v.border_width as f32),
        },
        flex_direction: match v.flex_direction {
            FlexDirection::Row => taffy::FlexDirection::Row,
            FlexDirection::Column => taffy::FlexDirection::Column,
            FlexDirection::RowReverse => taffy::FlexDirection::RowReverse,
            FlexDirection::ColumnReverse => taffy::FlexDirection::ColumnReverse,
        },
        flex_wrap: match v.flex_wrap {
            FlexWrap::NoWrap => taffy::FlexWrap::NoWrap,
            FlexWrap::Wrap => taffy::FlexWrap::Wrap,
            FlexWrap::WrapReverse => taffy::FlexWrap::WrapReverse,
        },
        flex_grow: v.flex_grow as f32,
        flex_shrink: v.flex_shrink as f32,
        flex_basis: dimension(v.flex_basis),
        gap: taffy::Size {
            width: length(v.column_gap),
            height: length(v.row_gap),
        },
        align_items: align(v.align_items),
        align_self: v.align_self.map(align),
        justify_items: align(v.justify_items),
        justify_self: v.justify_self.map(align),
        justify_content: content(v.justify_content),
        align_content: content(v.align_content),
        grid_template_columns: v
            .grid_template_columns
            .iter()
            .copied()
            .map(|v| taffy::GridTemplateComponent::Single(track(v)))
            .collect(),
        grid_template_rows: v
            .grid_template_rows
            .iter()
            .copied()
            .map(|v| taffy::GridTemplateComponent::Single(track(v)))
            .collect(),
        grid_auto_columns: v.grid_auto_columns.iter().copied().map(track).collect(),
        grid_auto_rows: v.grid_auto_rows.iter().copied().map(track).collect(),
        grid_auto_flow: match v.grid_auto_flow {
            GridAutoFlow::Row => taffy::GridAutoFlow::Row,
            GridAutoFlow::Column => taffy::GridAutoFlow::Column,
        },
        grid_column: taffy::Line {
            start: placement(v.grid_column_start),
            end: placement(v.grid_column_end),
        },
        grid_row: taffy::Line {
            start: placement(v.grid_row_start),
            end: placement(v.grid_row_end),
        },
        ..taffy::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_id_exhaustion_discards_staged_allocations_for_both_adapters()
    -> Result<(), LayoutError> {
        #[derive(Default)]
        struct Tree {
            children: Vec<usize>,
            published: Vec<usize>,
        }

        impl LayoutTree for Tree {
            type NodeId = usize;

            fn children(&self, id: usize) -> &[usize] {
                if id == 0 { &self.children } else { &[] }
            }

            fn layout_input(&self, _id: usize) -> LayoutInput {
                LayoutInput::default()
            }

            fn set_layout(&mut self, id: usize, _layout: LayoutBox) {
                self.published.push(id);
            }
        }

        let viewport = Size::new(200.0, 100.0);
        for sparse in [false, true] {
            let compute = |engine: &mut IncrementalTaffyLayout<_>, tree: &mut Tree, changes: &_| {
                if sparse {
                    engine.compute(tree, 0, viewport, changes)
                } else {
                    engine.layout.compute(tree, 0, viewport)
                }
            };
            let mut engine = IncrementalTaffyLayout::default();
            let mut tree = Tree::default();
            compute(&mut engine, &mut tree, &HashMap::new())?;
            engine.layout.next_identity = usize::MAX - 1;
            tree.published.clear();
            tree.children = vec![1, 2];
            let mut changes = HashMap::from([0, 1, 2].map(|id| {
                (
                    id,
                    LayoutChange::Update {
                        parent: (id != 0).then_some(0),
                        style: true,
                        children: true,
                    },
                )
            }));
            assert!(matches!(
                compute(&mut engine, &mut tree, &changes),
                Err(LayoutError::CacheExhausted)
            ));
            assert!(tree.published.is_empty());
            assert_eq!(engine.cached_node_count(), 1);
            assert_eq!(engine.layout.next_identity, usize::MAX - 1);

            tree.children.pop();
            changes.remove(&2);
            compute(&mut engine, &mut tree, &changes)?;
            assert_eq!(
                engine.layout.ids.get(&1),
                Some(&NodeId::from(usize::MAX - 1))
            );
            assert_eq!(engine.layout.next_identity, usize::MAX);
            assert!(tree.published.contains(&1));
        }
        Ok(())
    }
}
