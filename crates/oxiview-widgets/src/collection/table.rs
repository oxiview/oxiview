use super::{
    CollectionError, TableMountStats,
    mount::{Mounts, Placement},
    order::KeyOrder,
};
use oxiview_base::{Point, Rect, Size};
use oxiview_core::{NodeId, UiTree, View};
use oxiview_layout::{FixedExtent, VariableExtent, VirtualExtent};
use std::{borrow::Cow, ops::Range};

/// Stable identity of a body cell, independent of retained nodes and current indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellKey {
    /// The record key.
    pub row: u64,
    /// The column key.
    pub column: u64,
}

/// The endpoints of a rectangle in the current display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSelection {
    /// The cell where selection extension began.
    pub anchor: CellKey,
    /// The most recently selected cell and the starting point for navigation.
    pub active: CellKey,
}

/// How a new display order was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowOrderChange {
    /// Collapse the rectangle to the active cell, or clear it if the key was filtered out.
    SortOrFilter,
    /// Keep the rectangle if the ordered keys within it are unchanged.
    DataUpdate,
}

/// Initial keyed width of a table column, in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableColumn {
    /// Stable column identity.
    pub key: u64,
    /// Width of the header and every cell in this column.
    pub width: f64,
}

/// Data and geometry passed to a newly mounted table cell or heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableItem {
    /// Stable column identity.
    pub column: u64,
    /// Zero-based displayed column index.
    pub column_index: usize,
    /// The row key and displayed row index for a cell, or `None` for a heading.
    pub row: Option<(u64, usize)>,
    /// Geometry relative to the body content origin or the header viewport origin.
    pub frame: Rect,
}

/// A two-axis retained virtual table with fixed row height and keyed rectangular selection.
///
/// Keep independent, positioned body and header parents. Their direct children belong to this
/// reconciler. The body viewport supplies clipping and translation. Clip the header parent and
/// keep it outside the body viewport so its y position is pinned. Its x origin and width must
/// match the body viewport. Reconcile with the body viewport's current offset and size.
pub struct VirtualTable {
    rows: KeyOrder,
    row_extent: FixedExtent,
    row_height: f64,
    header_height: f64,
    row_overscan: usize,
    columns: KeyOrder,
    column_extent: VariableExtent,
    column_overscan: usize,
    selection: Option<CellSelection>,
    revision: u64,
    row_mounts: Mounts<u64>,
    cell_mounts: Mounts<CellKey>,
    header_mounts: Mounts<u64>,
}

impl VirtualTable {
    /// Creates an empty table with positive body and heading heights and per-axis overscan.
    pub fn new(
        row_height: f64,
        header_height: f64,
        row_overscan: usize,
        column_overscan: usize,
    ) -> Result<Self, CollectionError> {
        FixedExtent::new(1, header_height)?;
        Ok(Self {
            rows: KeyOrder::default(),
            row_extent: FixedExtent::new(0, row_height)?,
            row_height,
            header_height,
            row_overscan,
            columns: KeyOrder::default(),
            column_extent: VariableExtent::new([])?,
            column_overscan,
            selection: None,
            revision: 0,
            row_mounts: Mounts::default(),
            cell_mounts: Mounts::default(),
            header_mounts: Mounts::default(),
        })
    }

    /// Replaces the displayed columns. Order changes inside the rectangle collapse it.
    pub fn set_columns(
        &mut self,
        columns: impl IntoIterator<Item = TableColumn>,
    ) -> Result<(), CollectionError> {
        let (keys, widths): (Vec<_>, Vec<_>) = columns
            .into_iter()
            .map(|column| (column.key, column.width))
            .unzip();
        let columns = KeyOrder::new(keys)?;
        let extent = VariableExtent::new(widths)?;
        if let Some(selection) = self.selection
            && self
                .columns
                .between(selection.anchor.column, selection.active.column)
                != columns.between(selection.anchor.column, selection.active.column)
        {
            self.collapse_or_clear(true, columns.index(selection.active.column).is_some());
        }
        self.columns = columns;
        self.column_extent = extent;
        Ok(())
    }

    /// Changes one column width, preserving any keyed selection.
    pub fn resize_column(&mut self, key: u64, width: f64) -> Result<bool, CollectionError> {
        let index = self
            .column_index(key)
            .ok_or(CollectionError::UnknownKey(key))?;
        Ok(self.column_extent.set_size(index, width)?)
    }

    /// Replaces the row order and applies its selection policy for the new source revision.
    pub fn set_rows(
        &mut self,
        keys: impl IntoIterator<Item = u64>,
        revision: u64,
        change: RowOrderChange,
    ) -> Result<(), CollectionError> {
        let rows = KeyOrder::new(keys)?;
        self.row_extent.set_len(rows.keys.len())?;
        if let Some(selection) = self.selection
            && (change == RowOrderChange::SortOrFilter
                || self
                    .rows
                    .between(selection.anchor.row, selection.active.row)
                    != rows.between(selection.anchor.row, selection.active.row))
        {
            self.collapse_or_clear(rows.index(selection.active.row).is_some(), true);
        }
        self.rows = rows;
        self.revision = revision;
        Ok(())
    }

    /// Adds new records at the end of display order without changing the rectangle.
    pub fn append_rows(
        &mut self,
        keys: impl IntoIterator<Item = u64>,
        revision: u64,
    ) -> Result<(), CollectionError> {
        let keys = self.rows.checked_tail(keys)?;
        let len = self
            .row_extent
            .len()
            .checked_add(keys.len())
            .ok_or(oxiview_layout::ExtentError::TooLarge)?;
        self.row_extent.set_len(len)?;
        self.rows.append(keys);
        self.revision = revision;
        Ok(())
    }

    /// Advances the source revision after an edit with no display-order change.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }

    /// Returns the entire body content extent, excluding the pinned header.
    pub fn total_size(&self) -> Size {
        Size::new(self.column_extent.total(), self.row_extent.total())
    }

    /// Returns the number of rows in the current order.
    pub fn row_count(&self) -> usize {
        self.row_extent.len()
    }

    /// Returns the number of columns in the current order.
    pub fn column_count(&self) -> usize {
        self.column_extent.len()
    }

    /// Returns the displayed row key at an index.
    pub fn row_key(&self, index: usize) -> Option<u64> {
        self.rows.key(index)
    }

    /// Returns the displayed column key at an index.
    pub fn column_key(&self, index: usize) -> Option<u64> {
        self.columns.key(index)
    }

    /// Looks up a row key in the ordered model, including offscreen rows.
    pub fn row_index(&self, key: u64) -> Option<usize> {
        self.rows.index(key)
    }

    /// Looks up a column key in the ordered model, including offscreen columns.
    pub fn column_index(&self, key: u64) -> Option<usize> {
        self.columns.index(key)
    }

    /// Returns the fixed body row height.
    pub fn row_height(&self) -> f64 {
        self.row_height
    }

    /// Returns the pinned heading height.
    pub fn header_height(&self) -> f64 {
        self.header_height
    }

    /// Returns the leading edge of a column, including the final edge at `column_count()`.
    pub fn column_offset(&self, index: usize) -> Option<f64> {
        self.column_extent.offset(index)
    }

    /// Returns the current width of the column at the given displayed index.
    pub fn column_width(&self, index: usize) -> Option<f64> {
        self.column_extent.size(index)
    }

    /// Returns the leading edge of a row, including the final edge at `row_count()`.
    pub fn row_offset(&self, index: usize) -> Option<f64> {
        self.row_extent.offset(index)
    }

    /// Returns the row containing the given content-space y coordinate.
    pub fn row_at(&self, offset: f64) -> Option<usize> {
        self.row_extent.index_at(offset)
    }

    /// Returns the column containing the given content-space x coordinate.
    pub fn column_at(&self, offset: f64) -> Option<usize> {
        self.column_extent.index_at(offset)
    }

    /// Returns the actual visible row range before adding overscan.
    pub fn visible_rows(&self, offset: f64, viewport: f64) -> Range<usize> {
        self.row_extent.visible_range(offset, viewport, 0)
    }

    /// Returns the actual visible column range before adding overscan.
    pub fn visible_columns(&self, offset: f64, viewport: f64) -> Range<usize> {
        self.column_extent.visible_range(offset, viewport, 0)
    }

    /// Returns the two-axis body scroll offset that reveals a displayed cell.
    pub fn reveal(&self, cell: CellKey, offset: Point, viewport: Size) -> Option<Point> {
        Some(Point::new(
            self.column_extent.reveal(
                self.column_index(cell.column)?,
                offset.x(),
                viewport.width(),
            )?,
            self.row_extent
                .reveal(self.row_index(cell.row)?, offset.y(), viewport.height())?,
        ))
    }

    /// Returns the current keyed rectangle, if any.
    pub fn selection(&self) -> Option<CellSelection> {
        self.selection
    }

    /// Selects a displayed cell, optionally keeping the previous anchor. Returns whether it changed.
    pub fn select(&mut self, cell: CellKey, extend: bool) -> bool {
        if self.row_index(cell.row).is_none() || self.column_index(cell.column).is_none() {
            return false;
        }
        let anchor = self
            .selection
            .filter(|_| extend)
            .map_or(cell, |current| current.anchor);
        let selection = Some(CellSelection {
            anchor,
            active: cell,
        });
        let changed = selection != self.selection;
        self.selection = selection;
        changed
    }

    /// Clears the keyed rectangle.
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Tests whether a displayed cell lies inside the current rectangle, including its endpoints.
    pub fn is_selected(&self, cell: CellKey) -> bool {
        self.selected_ranges().is_some_and(|(rows, columns)| {
            self.row_index(cell.row)
                .is_some_and(|row| rows.contains(&row))
                && self
                    .column_index(cell.column)
                    .is_some_and(|column| columns.contains(&column))
        })
    }

    /// Moves from the active cell by signed offsets, clamping both axes. With no active cell,
    /// selects the first cell. Use [`Self::reveal`] to scroll the returned destination into view.
    pub fn navigate(
        &mut self,
        row_delta: isize,
        column_delta: isize,
        extend: bool,
    ) -> Option<CellKey> {
        let cell = if let Some(CellSelection { active, .. }) = self.selection {
            let row = self
                .row_index(active.row)?
                .saturating_add_signed(row_delta)
                .min(self.row_count().saturating_sub(1));
            let column = self
                .column_index(active.column)?
                .saturating_add_signed(column_delta)
                .min(self.column_count().saturating_sub(1));
            CellKey {
                row: self.row_key(row)?,
                column: self.column_key(column)?,
            }
        } else {
            CellKey {
                row: self.row_key(0)?,
                column: self.column_key(0)?,
            }
        };
        self.select(cell, extend);
        Some(cell)
    }

    /// Copies a rectangle in displayed order from a single source revision, including unmounted
    /// cells. The resolver receives that revision and the stable cell key. Fields with a tab,
    /// carriage return, newline, or quote are quoted and their embedded quotes are doubled.
    pub fn copy_tsv<'a>(
        &self,
        mut text: impl FnMut(u64, CellKey) -> Cow<'a, str>,
    ) -> Option<String> {
        let (rows, columns) = self.selected_ranges()?;
        let revision = self.revision;
        let mut tsv = String::new();
        for (output_row, &row) in self.rows.keys.get(rows)?.iter().enumerate() {
            if output_row != 0 {
                tsv.push('\n');
            }
            for (output_column, &column) in
                self.columns.keys.get(columns.clone())?.iter().enumerate()
            {
                if output_column != 0 {
                    tsv.push('\t');
                }
                push_field(&mut tsv, &text(revision, CellKey { row, column }));
            }
        }
        Some(tsv)
    }

    fn selected_ranges(&self) -> Option<(Range<usize>, Range<usize>)> {
        let CellSelection { anchor, active } = self.selection?;
        let top = self.row_index(anchor.row)?;
        let bottom = self.row_index(active.row)?;
        let left = self.column_index(anchor.column)?;
        let right = self.column_index(active.column)?;
        Some((
            top.min(bottom)..top.max(bottom) + 1,
            left.min(right)..left.max(right) + 1,
        ))
    }

    fn collapse_or_clear(&mut self, keeps_row: bool, keeps_column: bool) {
        self.selection = self
            .selection
            .filter(|_| keeps_row && keeps_column)
            .map(|current| CellSelection {
                anchor: current.active,
                active: current.active,
            });
    }

    /// Reconciles visible and overscan rows, cells and headings. The builder runs only for
    /// entering keys. Builder roots are stretched and clipped inside their placement wrappers.
    pub fn reconcile(
        &mut self,
        tree: &mut UiTree,
        body: NodeId,
        header: NodeId,
        offset: Point,
        viewport: Size,
        mut build: impl FnMut(TableItem) -> View,
    ) -> Result<TableMountStats, CollectionError> {
        tree.children(body)?;
        tree.children(header)?;
        let rows = self
            .row_extent
            .visible_range(offset.y(), viewport.height(), self.row_overscan);
        let columns =
            self.column_extent
                .visible_range(offset.x(), viewport.width(), self.column_overscan);
        let row_in_range = |key| {
            self.rows
                .index(key)
                .is_some_and(|index| rows.contains(&index))
        };
        let column_in_range = |key| {
            self.columns
                .index(key)
                .is_some_and(|index| columns.contains(&index))
        };
        let disposed_cells = self.cell_mounts.prune(tree, |key| {
            (row_in_range(key.row) && column_in_range(key.column))
                .then(|| self.row_mounts.view(key.row))
                .flatten()
        })?;
        let disposed_rows = self
            .row_mounts
            .prune(tree, |key| row_in_range(key).then_some(body))?;
        let disposed_headers = self
            .header_mounts
            .prune(tree, |key| column_in_range(key).then_some(header))?;

        let row_placements = rows.clone().enumerate().filter_map(|(index, row)| {
            let key = self.rows.key(row)?;
            let frame = Rect::from_origin_size(
                Point::new(0.0, self.row_extent.offset(row)?),
                Size::new(self.column_extent.total(), self.row_height),
            );
            Some(Placement {
                key,
                parent: body,
                index,
                frame,
                item: (),
            })
        });
        let mut row_stats = self.row_mounts.place(tree, row_placements, |_| None)?;

        let tracks: Vec<_> = columns
            .enumerate()
            .filter_map(|(index, column)| {
                Some((
                    index,
                    column,
                    self.column_key(column)?,
                    self.column_offset(column)?,
                    self.column_width(column)?,
                ))
            })
            .collect();
        let mut cells = Vec::new();
        for row_index in rows {
            let Some(row) = self.row_key(row_index) else {
                continue;
            };
            let Some(parent) = self.row_mounts.view(row) else {
                continue;
            };
            let top = self.row_extent.offset(row_index).unwrap_or_default();
            for &(index, column_index, column, left, width) in &tracks {
                let frame = Rect::from_origin_size(
                    Point::new(left, 0.0),
                    Size::new(width, self.row_height),
                );
                let item_frame = Rect::from_origin_size(Point::new(left, top), frame.size());
                cells.push(Placement {
                    key: CellKey { row, column },
                    parent,
                    index,
                    frame,
                    item: TableItem {
                        column,
                        column_index,
                        row: Some((row, row_index)),
                        frame: item_frame,
                    },
                });
            }
        }
        let mut cell_stats = self
            .cell_mounts
            .place(tree, cells, |item| Some(build(item)))?;
        let headers = tracks
            .into_iter()
            .map(|(index, column_index, column, left, width)| {
                let frame = Rect::from_origin_size(
                    Point::new(left - offset.x(), 0.0),
                    Size::new(width, self.header_height),
                );
                Placement {
                    key: column,
                    parent: header,
                    index,
                    frame,
                    item: TableItem {
                        column,
                        column_index,
                        row: None,
                        frame,
                    },
                }
            });
        let mut header_stats = self
            .header_mounts
            .place(tree, headers, |item| Some(build(item)))?;
        row_stats.disposed = disposed_rows;
        cell_stats.disposed = disposed_cells;
        header_stats.disposed = disposed_headers;
        Ok(TableMountStats {
            rows: row_stats,
            cells: cell_stats,
            headers: header_stats,
        })
    }

    /// Returns the builder root for a retained data cell.
    pub fn cell_node(&self, cell: CellKey) -> Option<NodeId> {
        self.cell_mounts.view(cell)
    }

    /// Returns the builder root for a retained column heading.
    pub fn header_node(&self, column: u64) -> Option<NodeId> {
        self.header_mounts.view(column)
    }

    /// Iterates the retained cells' keys and builder roots in unspecified order.
    pub fn mounted_cells(&self) -> impl Iterator<Item = (CellKey, NodeId)> + '_ {
        self.cell_mounts.views()
    }

    /// Iterates the retained headings' keys and builder roots in unspecified order.
    pub fn mounted_headers(&self) -> impl Iterator<Item = (u64, NodeId)> + '_ {
        self.header_mounts.views()
    }

    /// Unmounts all table Views while preserving the ordered data, widths and selection.
    pub fn clear(&mut self, tree: &mut UiTree) -> Result<(), CollectionError> {
        self.cell_mounts.clear(tree)?;
        self.row_mounts.clear(tree)?;
        self.header_mounts.clear(tree)
    }
}

fn push_field(output: &mut String, field: &str) {
    if !field.contains(['\t', '\r', '\n', '"']) {
        output.push_str(field);
        return;
    }
    output.push('"');
    for ch in field.chars() {
        if ch == '"' {
            output.push('"');
        }
        output.push(ch);
    }
    output.push('"');
}
