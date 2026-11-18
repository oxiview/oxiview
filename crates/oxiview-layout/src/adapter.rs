use crate::*;
use oxiview_base::{Point, Size};
use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};
use taffy::{CacheTree, TraversePartialTree};

/// Layout implementation borrowing the caller's topology for each computation.
///
/// The retained tree remains authoritative. Private caches retain style/topology
/// fingerprints, measurement revisions, and Taffy results across computations.
/// Every pass validates the caller's tree; cached children are not used as an
/// alternate traversal source. Failed passes publish neither geometry nor caches.
#[derive(Debug)]
pub struct TaffyLayout<I> {
    retained: HashMap<I, CachedNode<I>>,
    next_identity: usize,
}
impl<I> Default for TaffyLayout<I> {
    fn default() -> Self {
        Self {
            retained: HashMap::new(),
            next_identity: 0,
        }
    }
}
impl<I: Copy + Eq + Hash> TaffyLayout<I> {
    /// Creates an adapter without allocating a second retained tree.
    pub fn new() -> Self {
        Self::default()
    }
    /// Discards all retained results and their private identity mappings.
    pub fn clear_cache(&mut self) {
        self.retained.clear();
        self.next_identity = 0;
    }
    /// Number of live-node cache records retained by the last successful pass.
    pub fn cached_node_count(&self) -> usize {
        self.retained.len()
    }
    /// Computes and atomically publishes parent-relative geometry.
    pub fn compute<T: LayoutTree<NodeId = I>>(
        &mut self,
        tree: &mut T,
        root: I,
        viewport: Size,
    ) -> Result<LayoutStats, LayoutError> {
        let _span = tracing::info_span!("ui_layout").entered();
        if ![viewport.width(), viewport.height()]
            .into_iter()
            .all(valid_extent)
        {
            return Err(LayoutError::InvalidViewport);
        }
        let mut entries = Vec::new();
        let mut indices = HashMap::new();
        let mut entry_indices = HashMap::new();
        let mut next_identity = self.next_identity;
        let mut seen = HashSet::new();
        let mut stack = vec![(root, None, 1usize)];
        while let Some((id, parent, depth)) = stack.pop() {
            if depth > MAX_LAYOUT_DEPTH {
                return Err(LayoutError::ExcessiveDepth);
            }
            if !seen.insert(id) || !tree.contains_node(id) {
                return Err(LayoutError::InvalidTree);
            }
            let input = tree.layout_input(id);
            input.validate()?;
            let index = entries.len();
            let identity = if let Some(saved) = self.retained.get(&id) {
                saved.identity
            } else {
                let identity = next_identity;
                next_identity = next_identity
                    .checked_add(1)
                    .ok_or(LayoutError::CacheExhausted)?;
                identity
            };
            indices.insert(id, identity);
            entry_indices.insert(identity, index);
            let children = tree.children(id).to_vec();
            let revision = tree.content_revision(id);
            entries.push(Entry {
                id,
                identity,
                parent,
                style: convert_style(&input),
                input,
                children,
                revision,
                layout: taffy::Layout::new(),
                cache: taffy::Cache::new(),
                hoisted: Vec::new(),
            });
            for child in tree.children(id).iter().rev() {
                stack.push((*child, Some(index), depth + 1));
            }
        }
        // Fingerprints validate the cached inputs; traversal still uses the
        // caller's child slices. Private Taffy IDs remain stable because cached
        // LayoutOutput values can contain out-of-flow candidate identities.
        let mut dirty = Vec::with_capacity(entries.len());
        let mut reset_subtree = Vec::with_capacity(entries.len());
        for entry in &entries {
            let parent = entry
                .parent
                .and_then(|index| entries.get(index).map(|parent| parent.id));
            let saved = self.retained.get(&entry.id);
            dirty.push(saved.is_none_or(|saved| {
                saved.input != entry.input
                    || saved.children != entry.children
                    || saved.parent != parent
                    || saved.revision != entry.revision
            }));
            reset_subtree.push(saved.is_some_and(|saved| {
                saved.parent != parent
                    || saved.input.display != entry.input.display
                    || saved.input.position != entry.input.position
            }));
        }
        for index in 0..entries.len() {
            let inherited = entries
                .get(index)
                .and_then(|entry| entry.parent)
                .and_then(|parent| reset_subtree.get(parent))
                .copied()
                .unwrap_or(false);
            if inherited {
                if let Some(reset) = reset_subtree.get_mut(index) {
                    *reset = true;
                }
                if let Some(dirty) = dirty.get_mut(index) {
                    *dirty = true;
                }
            }
        }
        for index in (0..entries.len()).rev() {
            if dirty.get(index).copied().unwrap_or(true)
                && let Some(parent) = entries.get(index).and_then(|entry| entry.parent)
                && let Some(dirty) = dirty.get_mut(parent)
            {
                *dirty = true;
            }
        }
        for (index, entry) in entries.iter_mut().enumerate() {
            if let Some(saved) = self.retained.get(&entry.id) {
                entry.layout = saved.layout;
                if !dirty.get(index).copied().unwrap_or(true) {
                    entry.cache = saved.cache.clone();
                    entry.hoisted = saved.hoisted.clone();
                }
            }
        }
        let root_identity = *indices.get(&root).ok_or(LayoutError::InvalidTree)?;
        let mut adapter = Adapter {
            tree,
            entries,
            indices,
            entry_indices,
            default_style: taffy::Style::default(),
            stats: LayoutStats::default(),
            invalid_geometry: false,
        };
        adapter.stats.visited_nodes = adapter.entries.len();
        taffy::compute_root_layout(
            &mut adapter,
            taffy::NodeId::from(root_identity),
            taffy::Size {
                width: taffy::AvailableSpace::Definite(viewport.width() as f32),
                height: taffy::AvailableSpace::Definite(viewport.height() as f32),
            },
        );
        if adapter.invalid_geometry {
            return Err(LayoutError::InvalidGeometry);
        }
        let mut layout_parents: Vec<Option<usize>> =
            adapter.entries.iter().map(|entry| entry.parent).collect();
        for (index, entry) in adapter.entries.iter().enumerate() {
            for child in &entry.hoisted {
                let child_index = adapter
                    .entry_indices
                    .get(&usize::from(*child))
                    .copied()
                    .ok_or(LayoutError::InvalidTree)?;
                let Some(parent) = layout_parents.get_mut(child_index) else {
                    return Err(LayoutError::InvalidTree);
                };
                *parent = Some(index);
            }
        }
        let mut absolute = vec![Point::default(); adapter.entries.len()];
        let mut results = Vec::with_capacity(adapter.entries.len());
        for (index, entry) in adapter.entries.iter().enumerate() {
            let layout = entry.layout;
            let values = [
                layout.location.x,
                layout.location.y,
                layout.size.width,
                layout.size.height,
                layout.padding.left,
                layout.padding.right,
                layout.padding.top,
                layout.padding.bottom,
                layout.border.left,
                layout.border.right,
                layout.border.top,
                layout.border.bottom,
            ];
            if !values.into_iter().all(f32::is_finite)
                || layout.size.width < 0.0
                || layout.size.height < 0.0
            {
                return Err(LayoutError::InvalidGeometry);
            }
            let layout_parent = layout_parents.get(index).copied().flatten();
            let offset = match layout_parent {
                Some(parent) => *absolute.get(parent).ok_or(LayoutError::InvalidTree)?,
                None => Point::default(),
            };
            let position = Point::new(
                offset.x() + f64::from(layout.location.x),
                offset.y() + f64::from(layout.location.y),
            );
            let Some(slot) = absolute.get_mut(index) else {
                return Err(LayoutError::InvalidTree);
            };
            *slot = position;
            let parent_origin = match entry.parent {
                Some(parent) => *absolute.get(parent).ok_or(LayoutError::InvalidTree)?,
                None => Point::default(),
            };
            let overflow = layout.scrollable_overflow_rect;
            let geometry = LayoutBox {
                location: Point::new(
                    position.x() - parent_origin.x(),
                    position.y() - parent_origin.y(),
                ),
                size: Size::new(f64::from(layout.size.width), f64::from(layout.size.height)),
                content_size: Size::new(
                    f64::from(overflow.right - overflow.left),
                    f64::from(overflow.bottom - overflow.top),
                ),
                padding: convert_edges(layout.padding),
                border: convert_edges(layout.border),
            };
            if ![
                geometry.content_size.width(),
                geometry.content_size.height(),
            ]
            .into_iter()
            .all(valid_extent)
            {
                return Err(LayoutError::InvalidGeometry);
            }
            results.push((entry.id, geometry));
        }
        for (id, geometry) in results {
            adapter.tree.set_layout(id, geometry);
        }
        let stats = adapter.stats;
        let mut retained = HashMap::with_capacity(adapter.entries.len());
        for entry in &adapter.entries {
            let parent = entry
                .parent
                .and_then(|index| adapter.entries.get(index).map(|parent| parent.id));
            retained.insert(
                entry.id,
                CachedNode {
                    identity: entry.identity,
                    input: entry.input.clone(),
                    parent,
                    children: entry.children.clone(),
                    revision: entry.revision,
                    layout: entry.layout,
                    cache: entry.cache.clone(),
                    hoisted: entry.hoisted.clone(),
                },
            );
        }
        self.retained = retained;
        self.next_identity = next_identity;
        Ok(stats)
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
struct Entry<I> {
    id: I,
    identity: usize,
    parent: Option<usize>,
    style: taffy::Style,
    input: LayoutInput,
    children: Vec<I>,
    revision: u64,
    layout: taffy::Layout,
    cache: taffy::Cache,
    hoisted: Vec<taffy::NodeId>,
}
#[derive(Debug)]
struct CachedNode<I> {
    identity: usize,
    parent: Option<I>,
    input: LayoutInput,
    children: Vec<I>,
    revision: u64,
    layout: taffy::Layout,
    cache: taffy::Cache,
    hoisted: Vec<taffy::NodeId>,
}
struct Adapter<'a, T: LayoutTree> {
    tree: &'a mut T,
    entries: Vec<Entry<T::NodeId>>,
    indices: HashMap<T::NodeId, usize>,
    entry_indices: HashMap<usize, usize>,
    default_style: taffy::Style,
    stats: LayoutStats,
    invalid_geometry: bool,
}
impl<T: LayoutTree> Adapter<'_, T> {
    fn entry(&self, id: taffy::NodeId) -> Option<&Entry<T::NodeId>> {
        self.entry_indices
            .get(&usize::from(id))
            .and_then(|index| self.entries.get(*index))
    }
    fn entry_mut(&mut self, id: taffy::NodeId) -> Option<&mut Entry<T::NodeId>> {
        let index = self.entry_indices.get(&usize::from(id)).copied()?;
        self.entries.get_mut(index)
    }
    fn style(&self, id: taffy::NodeId) -> &taffy::Style {
        self.entry(id)
            .map_or(&self.default_style, |entry| &entry.style)
    }
    fn node(&self, id: taffy::NodeId) -> Option<T::NodeId> {
        self.entry(id).map(|entry| entry.id)
    }
}
struct ChildIter<'a, I> {
    children: std::slice::Iter<'a, I>,
    indices: &'a HashMap<I, usize>,
}
impl<I: Eq + Hash> Iterator for ChildIter<'_, I> {
    type Item = taffy::NodeId;
    fn next(&mut self) -> Option<Self::Item> {
        self.children
            .next()
            .and_then(|id| self.indices.get(id).copied())
            .map(taffy::NodeId::from)
    }
}
impl<T: LayoutTree> taffy::TraversePartialTree for Adapter<'_, T> {
    type ChildIter<'a>
        = ChildIter<'a, T::NodeId>
    where
        Self: 'a;
    fn child_ids(&self, id: taffy::NodeId) -> Self::ChildIter<'_> {
        let children = self.node(id).map_or(&[][..], |id| self.tree.children(id));
        ChildIter {
            children: children.iter(),
            indices: &self.indices,
        }
    }
    fn child_count(&self, id: taffy::NodeId) -> usize {
        self.node(id).map_or(0, |id| self.tree.children(id).len())
    }
    fn get_child_id(&self, id: taffy::NodeId, index: usize) -> taffy::NodeId {
        self.node(id)
            .and_then(|id| self.tree.children(id).get(index))
            .and_then(|id| self.indices.get(id))
            .copied()
            .map_or_else(|| taffy::NodeId::from(usize::MAX), taffy::NodeId::from)
    }
}
impl<T: LayoutTree> taffy::TraverseTree for Adapter<'_, T> {}
impl<T: LayoutTree> taffy::LayoutPartialTree for Adapter<'_, T> {
    type CustomIdent = String;
    type CoreContainerStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    fn get_core_container_style(&self, id: taffy::NodeId) -> Self::CoreContainerStyle<'_> {
        self.style(id)
    }
    fn set_unrounded_layout(&mut self, id: taffy::NodeId, layout: &taffy::Layout) {
        if let Some(entry) = self.entry_mut(id) {
            entry.layout = *layout;
        } else {
            self.invalid_geometry = true;
        }
    }
    fn resolve_calc_value(&self, _value: *const (), _basis: f32) -> f32 {
        0.0
    }
    fn compute_child_layout(
        &mut self,
        id: taffy::NodeId,
        inputs: taffy::LayoutInput,
    ) -> taffy::LayoutOutput {
        taffy::compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            tree.stats.computed_nodes += 1;
            let style = tree.style(id).clone();
            let mut output = if inputs.run_mode == taffy::RunMode::PerformHiddenLayout
                || style.display == taffy::Display::None
            {
                taffy::compute_hidden_layout(tree, id)
            } else if tree.child_count(id) == 0 {
                taffy::compute_leaf_layout(
                    inputs,
                    &style,
                    |_, _| 0.0,
                    |known, available| {
                        let Some(node) = tree.node(id) else {
                            tree.invalid_geometry = true;
                            return taffy::Size::ZERO;
                        };
                        tree.stats.measured_nodes += 1;
                        let measured = tree.tree.measure(
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
                            tree.invalid_geometry = true;
                            return taffy::Size::ZERO;
                        }
                        taffy::Size {
                            width: measured.width as f32,
                            height: measured.height as f32,
                        }
                    },
                )
            } else {
                match style.display {
                    taffy::Display::Flex => taffy::compute_flexbox_layout(tree, id, inputs),
                    taffy::Display::Grid => taffy::compute_grid_layout(tree, id, inputs),
                    taffy::Display::Block | taffy::Display::FlowRoot => {
                        taffy::compute_block_layout(tree, id, inputs, None)
                    }
                    taffy::Display::None => taffy::compute_hidden_layout(tree, id),
                }
            };
            if inputs.run_mode == taffy::RunMode::PerformLayout {
                taffy::compute_oof_layout(tree, id, &mut output);
            }
            output
        })
    }
}
impl<T: LayoutTree> CacheTree for Adapter<'_, T> {
    fn cache_get(
        &mut self,
        id: taffy::NodeId,
        inputs: &taffy::LayoutInput,
    ) -> Option<taffy::LayoutOutput> {
        let result = self.entry_mut(id).and_then(|entry| entry.cache.get(inputs));
        if result.is_some() {
            self.stats.cache_hits += 1;
        }
        result
    }
    fn cache_store(
        &mut self,
        id: taffy::NodeId,
        inputs: &taffy::LayoutInput,
        output: taffy::LayoutOutput,
    ) {
        if let Some(entry) = self.entry_mut(id) {
            entry.cache.store(inputs, output);
        }
    }
    fn cache_clear(&mut self, id: taffy::NodeId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.cache.clear();
        }
    }
}
impl<T: LayoutTree> taffy::LayoutContainingBlock for Adapter<'_, T> {
    type OofItemStyle<'a>
        = &'a taffy::Style
    where
        Self: 'a;
    fn get_oof_item_style(&self, id: taffy::NodeId) -> Self::OofItemStyle<'_> {
        self.style(id)
    }
    fn clear_hoisted_children(&mut self, id: taffy::NodeId) {
        if let Some(entry) = self.entry_mut(id) {
            entry.hoisted.clear();
        }
    }
    fn add_hoisted_children(&mut self, id: taffy::NodeId, hoisted: &[taffy::NodeId]) {
        if let Some(entry) = self.entry_mut(id) {
            entry.hoisted.extend_from_slice(hoisted);
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
    fn get_flexbox_container_style(&self, id: taffy::NodeId) -> Self::FlexboxContainerStyle<'_> {
        self.style(id)
    }
    fn get_flexbox_child_style(&self, id: taffy::NodeId) -> Self::FlexboxItemStyle<'_> {
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
    fn get_grid_container_style(&self, id: taffy::NodeId) -> Self::GridContainerStyle<'_> {
        self.style(id)
    }
    fn get_grid_child_style(&self, id: taffy::NodeId) -> Self::GridItemStyle<'_> {
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
    fn get_block_container_style(&self, id: taffy::NodeId) -> Self::BlockContainerStyle<'_> {
        self.style(id)
    }
    fn get_block_child_style(&self, id: taffy::NodeId) -> Self::BlockItemStyle<'_> {
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
