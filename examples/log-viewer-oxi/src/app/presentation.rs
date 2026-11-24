//! Synchronizes the retained shell and shared virtual collections with the model.

use super::*;
use oxiview_core::{NodeId, NodeSemantics};
use oxiview_style::Display;
use oxiview_widgets::{
    collection::{CellKey, CellSelection, ListAnchor, RowOrderChange},
    controls::{ScrollbarMetrics, ToggleKind},
};

#[derive(Clone, Copy, PartialEq)]
struct Canvas {
    mode: Mode,
    bounds: Size,
    offset: Point,
    revision: u64,
    layout_revision: u64,
    selected: Option<u64>,
    range: Option<CellSelection>,
    expanded: Option<u64>,
}

#[derive(Default)]
pub(super) struct Presentation {
    canvas: Option<Canvas>,
    order_revision: Option<u64>,
    data_revision: Option<u64>,
    pub(super) sort_or_filter: bool,
    pub(super) layout_revision: u64,
    pub(super) reveal_tail: bool,
    placed_mode: Option<Mode>,
    expansion: Option<(u64, f64, u64)>,
    expanded_content_height: f64,
    detail: Option<(Option<u64>, u64, f64)>,
    controls: Option<ControlState>,
    notice: String,
}

#[derive(PartialEq)]
struct ControlState {
    selected: bool,
    follow: bool,
    unseen: usize,
    matching: usize,
    total: usize,
    levels: [bool; 5],
    source: String,
}

pub(super) fn column(key: u64) -> Option<Column> {
    Column::ALL.get(usize::try_from(key).ok()?).copied()
}

impl ViewerApp {
    pub(super) fn sync(&mut self) -> Result<bool, Error> {
        let collection = self.ui.collection.get()?;
        let detail = self.ui.detail.get()?;
        let header = self.ui.header.get()?;
        let mut changed = self.sync_mode()?;
        self.tree.layout(self.viewport)?;
        let bounds = self.tree.bounds(collection)?.size();
        let offset = self.tree.scroll_offset(collection)?;
        let anchor = self.scroll_anchor(offset.y());
        let data_changed = self.display.data_revision != Some(self.book.revision());
        let (order_changed, sort_or_filter) = self.update_order()?;
        let expansion_changed = self.measure_expanded(bounds.width())?;
        let extent = match self.mode {
            Mode::List => Size::new(bounds.width(), self.list.total_height()),
            Mode::Table => self.table.total_size(),
        };
        changed |= self.tree.set_scroll_content_extent(collection, extent)?;
        let retarget = if self.book.follow()
            && (data_changed
                || expansion_changed
                || self.display.reveal_tail
                || self
                    .display
                    .canvas
                    .is_none_or(|canvas| canvas.bounds != bounds))
        {
            Some(Point::new(
                offset.x(),
                self.tree.scroll_max_offset(collection)?.y(),
            ))
        } else if order_changed || expansion_changed {
            Some(Point::new(
                offset.x(),
                anchor
                    .and_then(|anchor| self.restore_anchor(anchor))
                    .unwrap_or(if order_changed { 0. } else { offset.y() }),
            ))
        } else {
            None
        };
        if let Some(target) = retarget
            && target != self.tree.scroll_offset(collection)?
        {
            changed |= self.tree.set_scroll_offset(collection, target)?;
        }
        if sort_or_filter
            && !self.book.follow()
            && let Some(key) = self.active_key()
        {
            changed |= self.reveal_key(key)?;
        }
        self.display.reveal_tail = false;
        let offset = self.tree.scroll_offset(collection)?;
        let stamp = Canvas {
            mode: self.mode,
            bounds,
            offset,
            revision: self.book.revision(),
            layout_revision: self.display.layout_revision,
            selected: self.active_key(),
            range: self.table.selection(),
            expanded: self.expanded,
        };
        if self.display.canvas != Some(stamp) {
            let can_reuse = self
                .display
                .canvas
                .is_some_and(|old| old.mode == stamp.mode);
            let selection_changed = can_reuse
                && self
                    .display
                    .canvas
                    .is_some_and(|old| old.selected != stamp.selected || old.range != stamp.range);
            let refresh_content = can_reuse && data_changed;
            match self.mode {
                Mode::List => {
                    let (ui, book, expanded) = (&self.ui, &self.book, self.expanded);
                    self.list
                        .reconcile(&mut self.tree, collection, offset.y(), bounds, |row| {
                            view::list_row(
                                ui,
                                book,
                                row,
                                stamp.selected == Some(row.key),
                                expanded == Some(row.key),
                            )
                        })
                        .map_err(error)?;
                    if refresh_content || selection_changed {
                        self.update_list_rows(refresh_content)?;
                    }
                }
                Mode::Table => {
                    let (ui, book) = (&self.ui, &self.book);
                    self.table
                        .reconcile(&mut self.tree, collection, header, offset, bounds, |item| {
                            if let Some(column) = column(item.column) {
                                view::table_item(ui, book, item, column)
                            } else {
                                oxiview_core::node()
                            }
                        })
                        .map_err(error)?;
                    if refresh_content || selection_changed || stamp.selected.is_some() {
                        self.update_table_cells(refresh_content)?;
                    }
                }
            }
            self.tree.layout(self.viewport)?;
            self.configure_expanded_viewport()?;
            self.display.canvas = Some(stamp);
            changed = true;
        }

        self.tree.layout(self.viewport)?;
        let detail_size = self.tree.bounds(detail)?.size();
        changed |= self.sync_detail(detail, detail_size.width(), data_changed)?;
        changed |= self.sync_controls()?;
        let maximum = self.tree.scroll_max_offset(collection)?;
        changed |= self.ui.vertical.update(
            &mut self.tree,
            ScrollbarAxis::Vertical,
            ScrollbarMetrics {
                viewport: bounds.height(),
                maximum: maximum.y(),
                offset: offset.y(),
            },
        )?;
        changed |= self.ui.horizontal.update(
            &mut self.tree,
            ScrollbarAxis::Horizontal,
            ScrollbarMetrics {
                viewport: bounds.width(),
                maximum: maximum.x(),
                offset: offset.x(),
            },
        )?;
        self.tree.layout(self.viewport)?;
        if changed {
            self.tree.flush_updates(self.viewport, |_, _, _| {})?;
        }
        self.display.data_revision = Some(self.book.revision());
        Ok(changed)
    }

    fn sync_mode(&mut self) -> Result<bool, Error> {
        if self.display.placed_mode == Some(self.mode) {
            return Ok(false);
        }
        let table = self.is_table();
        let collection = self.ui.collection.get()?;
        let old_offset = if self.display.placed_mode.is_some() {
            self.tree.scroll_offset(collection)?
        } else {
            Point::default()
        };
        let old_anchor = self.display.placed_mode.and_then(|mode| match mode {
            Mode::List => self.list.anchor(old_offset.y()).map(|a| a.key),
            Mode::Table => self.table.row_key(self.table.row_at(old_offset.y())?),
        });
        if table {
            self.list.clear(&mut self.tree).map_err(error)?;
        } else {
            self.table.clear(&mut self.tree).map_err(error)?;
        }
        self.ui
            .mode
            .set_selected(&mut self.tree, usize::from(table))?;
        for (id, visible) in [
            (self.ui.header.get()?, table),
            (self.ui.legend.get()?, !table),
            (self.ui.horizontal_bar.get()?, table),
        ] {
            let mut style = self.tree.style(id)?.clone();
            style.layout.display = if visible {
                Display::Flex
            } else {
                Display::None
            };
            self.tree.set_style(id, style)?;
        }
        let mut semantics = self
            .tree
            .semantics(collection)?
            .cloned()
            .unwrap_or(NodeSemantics {
                label: "Log records".into(),
                ..Default::default()
            });
        semantics.role = if table { "table" } else { "list" }.into();
        self.tree.set_semantics(collection, Some(semantics))?;
        self.tree.layout(self.viewport)?;
        let extent = if table {
            self.table.total_size()
        } else {
            Size::new(
                self.tree.bounds(collection)?.size().width(),
                self.list.total_height(),
            )
        };
        self.tree.set_scroll_content_extent(collection, extent)?;
        let y = old_anchor
            .and_then(|key| self.restore_anchor(ListAnchor { key, intra_row: 0. }))
            .unwrap_or(0.);
        // Start the presentation at its matching row. A selected key can stay offscreen.
        self.tree.set_scroll_offset(collection, Point::new(0., y))?;
        self.display.placed_mode = Some(self.mode);
        self.display.canvas = None;
        Ok(true)
    }

    fn scroll_anchor(&self, offset: f64) -> Option<ListAnchor> {
        match self.mode {
            Mode::List => self.list.anchor(offset),
            Mode::Table => {
                let index = self.table.row_at(offset)?;
                Some(ListAnchor {
                    key: self.table.row_key(index)?,
                    intra_row: offset - self.table.row_offset(index)?,
                })
            }
        }
    }

    fn restore_anchor(&self, anchor: ListAnchor) -> Option<f64> {
        match self.mode {
            Mode::List => self.list.offset_for_anchor(anchor),
            Mode::Table => Some(
                self.table.row_offset(self.table.row_index(anchor.key)?)?
                    + anchor.intra_row.min(self.table.row_height()),
            ),
        }
    }

    fn update_order(&mut self) -> Result<(bool, bool), Error> {
        let order_revision = self.book.order_revision();
        let changed = self.display.order_revision != Some(order_revision);
        let sort_or_filter = std::mem::take(&mut self.display.sort_or_filter);
        let count = self.book.len();
        let tail = |from| {
            (from..count).filter_map(|index| self.book.record_at(index).map(|record| record.id))
        };
        if changed {
            self.record_drag = None;
            self.list.set_rows(tail(0)).map_err(error)?;
            self.table
                .set_rows(
                    tail(0),
                    self.book.revision(),
                    if sort_or_filter {
                        RowOrderChange::SortOrFilter
                    } else {
                        RowOrderChange::DataUpdate
                    },
                )
                .map_err(error)?;
            self.list_key = self
                .list_key
                .filter(|key| self.book.index_of(*key).is_some());
            self.expanded = self
                .expanded
                .filter(|key| self.book.index_of(*key).is_some());
        } else if self.list.len() < count {
            let previous = self.list.len();
            self.list.append_rows(tail(previous)).map_err(error)?;
            self.table
                .append_rows(tail(previous), self.book.revision())
                .map_err(error)?;
        } else if self.display.data_revision != Some(self.book.revision()) {
            self.table.set_revision(self.book.revision());
        }
        self.display.order_revision = Some(order_revision);
        Ok((changed, sort_or_filter))
    }

    fn measure_expanded(&mut self, width: f64) -> Result<bool, Error> {
        if self.mode != Mode::List {
            return Ok(false);
        }
        let next = self.expanded.map(|key| (key, width, self.book.revision()));
        let previous = self.display.expansion;
        if previous == next {
            return Ok(false);
        }
        let mut changed = false;
        if let Some((key, _, _)) = previous
            && next.is_none_or(|next| next.0 != key)
            && self.list.index_of(key).is_some()
        {
            changed |= self
                .list
                .set_height(key, view::LIST_HEIGHT)
                .map_err(error)?;
            self.list
                .invalidate_row(&mut self.tree, key)
                .map_err(error)?;
        }
        if let Some((key, _, _)) = next
            && let Some(record) = self
                .book
                .index_of(key)
                .and_then(|index| self.book.record_at(index))
        {
            let style = view::list_line_typeface();
            let text_height = self
                .ui
                .text
                .borrow_mut()
                .layout(
                    &record.message,
                    &style,
                    Some(view::list_message_width(width)),
                )
                .map_err(error)?
                .size()
                .height()
                .ceil();
            self.display.expanded_content_height = text_height;
            changed |= self
                .list
                .set_height(key, (text_height + 16.).clamp(view::LIST_HEIGHT, 206.))
                .map_err(error)?;
            if previous.is_none_or(|previous| previous.0 != key) {
                self.list
                    .invalidate_row(&mut self.tree, key)
                    .map_err(error)?;
            }
        }
        self.display.expansion = next;
        Ok(changed)
    }

    fn configure_expanded_viewport(&mut self) -> Result<(), Error> {
        if self.mode == Mode::List
            && let Some(row) = self.expanded.and_then(|key| self.list.row_node(key))
            && let Some(body) = self.tree.children(row)?.last().copied()
        {
            let width = self.tree.bounds(body)?.size().width();
            self.tree.set_scroll_content_extent(
                body,
                Size::new(width, self.display.expanded_content_height),
            )?;
        }
        Ok(())
    }

    fn update_list_rows(&mut self, content: bool) -> Result<(), Error> {
        for (key, node) in self.list.mounted_rows() {
            let Some(index) = self.book.index_of(key) else {
                continue;
            };
            let mut style = self.tree.style(node)?.clone();
            let background = view::row_background(index, self.list_key == Some(key), false);
            if style.background != background {
                style.background = background;
                self.tree.set_style(node, style)?;
            }
            if content {
                let Some(record) = self.book.record_at(index) else {
                    continue;
                };
                let children = self.tree.children(node)?;
                if let [_, line, time, level, message] = children {
                    let (line, time, level, message) = (*line, *time, *level, *message);
                    self.tree.set_text(line, format!("#{}", record.line))?;
                    self.tree
                        .set_text(time, self.book.cell(index, Column::Time).as_ref())?;
                    if let Some(mut semantics) = self.tree.semantics(node)?.cloned()
                        && semantics.value != record.severity.label()
                    {
                        self.tree.set_rich_text(
                            level,
                            view::severity_text(record.severity).map_err(error)?,
                        )?;
                        semantics.value = record.severity.label().into();
                        self.tree.set_semantics(node, Some(semantics))?;
                    }
                    if self.expanded == Some(key) {
                        if let Some(text) = self.tree.children(message)?.first().copied() {
                            self.tree.set_text(text, record.message.as_str())?;
                        }
                    } else {
                        self.tree
                            .set_text(message, view::compact(&record.message))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn update_table_cells(&mut self, content: bool) -> Result<(), Error> {
        let active = self.table.selection().map(|selection| selection.active);
        for (cell, node) in self.table.mounted_cells() {
            let Some(index) = self.book.index_of(cell.row) else {
                continue;
            };
            let whole_row = active.is_some_and(|active| {
                active.row == cell.row
                    && active.column == Column::Number as u64
                    && self
                        .table
                        .selection()
                        .is_some_and(|range| range.anchor == range.active)
            });
            let mut style = self.tree.style(node)?.clone();
            let background = view::row_background(
                index,
                whole_row || self.table.is_selected(cell),
                active == Some(cell),
            );
            if style.background != background {
                style.background = background;
                self.tree.set_style(node, style)?;
            }
            if content
                && let Some(column) = column(cell.column)
                && let Some(text) = self.tree.children(node)?.first().copied()
            {
                let value = view::compact(self.book.cell(index, column).as_ref());
                if let Some(mut semantics) = self.tree.semantics(node)?.cloned()
                    && semantics.label != value
                {
                    if column == Column::Level
                        && let Some(record) = self.book.record_at(index)
                    {
                        self.tree.set_rich_text(
                            text,
                            view::severity_text(record.severity).map_err(error)?,
                        )?;
                    } else {
                        self.tree.set_text(text, value.as_str())?;
                    }
                    semantics.label = value;
                    self.tree.set_semantics(node, Some(semantics))?;
                }
            }
        }
        if content {
            for (key, node) in self.table.mounted_headers() {
                if let Some(column) = column(key)
                    && let Some(indicator) = self.tree.children(node)?.last().copied()
                {
                    let heading = view::heading_label(&self.book, column);
                    if let Some(mut semantics) = self.tree.semantics(node)?.cloned()
                        && semantics.label != heading
                    {
                        self.tree
                            .set_content(indicator, view::heading_indicator(&self.book, column))?;
                        semantics.label = heading;
                        self.tree.set_semantics(node, Some(semantics))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn sync_detail(
        &mut self,
        detail: NodeId,
        width: f64,
        data_changed: bool,
    ) -> Result<bool, Error> {
        let key = self.active_key();
        let stamp = (key, self.book.revision(), width);
        if self.display.detail == Some(stamp) {
            return Ok(false);
        }
        let previous = self.display.detail.and_then(|old| old.0);
        self.display.detail = Some(stamp);
        if key.is_none() && previous.is_none() {
            return Ok(false);
        }
        let record = key
            .and_then(|key| self.book.index_of(key))
            .and_then(|index| self.book.record_at(index));
        let message = record.map_or("Select a record to inspect its complete contents.", |row| {
            row.raw.as_str()
        });
        if key != previous || data_changed {
            let label = record.map_or_else(String::new, |record| {
                format!(
                    "#{}  ·  {}  ·  {} bytes",
                    record.line,
                    record.severity.label(),
                    record.raw.len()
                )
            });
            self.tree.set_text(self.ui.detail_label.get()?, label)?;
            self.tree.set_text(self.ui.detail_body.get()?, message)?;
        }
        let height = self
            .ui
            .text
            .borrow_mut()
            .layout(
                message,
                &view::detail_typeface(),
                Some(view::detail_content_width(width)),
            )
            .map_err(error)?
            .size()
            .height()
            .ceil()
            + 14.;
        self.tree
            .set_scroll_content_extent(detail, Size::new(width, height))?;
        if key != previous {
            self.tree.set_scroll_offset(detail, Point::default())?;
        }
        Ok(true)
    }

    fn sync_controls(&mut self) -> Result<bool, Error> {
        let mut changed = false;
        if self.display.notice != self.notice {
            self.tree
                .set_text(self.ui.status.get()?, self.notice.as_str())?;
            self.display.notice.clone_from(&self.notice);
            changed = true;
        }
        let state = ControlState {
            selected: self.active_key().is_some(),
            follow: self.book.follow(),
            unseen: self.book.unseen(),
            matching: self.book.len(),
            total: self.book.total(),
            levels: Severity::ALL.map(|level| self.book.severity_enabled(level)),
            source: self.book.label().into(),
        };
        if self.display.controls.as_ref() == Some(&state) {
            return Ok(changed);
        }
        let old = self.display.controls.as_ref();
        if old.is_none_or(|old| old.selected != state.selected) {
            self.ui.copy.set_enabled(&mut self.tree, state.selected)?;
        }
        if old.is_none_or(|old| old.follow != state.follow) {
            self.ui
                .follow
                .set_checked(&mut self.tree, ToggleKind::Switch, state.follow)?;
        }
        if old.is_none_or(|old| old.levels != state.levels) {
            for (toggle, checked) in self.ui.severity.iter().zip(state.levels) {
                toggle.set_checked(&mut self.tree, ToggleKind::Checkbox, checked)?;
            }
        }
        if old.is_none_or(|old| old.source != state.source) {
            self.tree
                .set_text(self.ui.title.get()?, state.source.as_str())?;
        }
        if old.is_none_or(|old| old.matching != state.matching || old.total != state.total) {
            let count = if state.matching == state.total {
                format!("{} records", count(state.total))
            } else {
                format!("{} of {}", count(state.matching), count(state.total))
            };
            self.tree.set_text(self.ui.count.get()?, count)?;
            self.tree
                .set_visible(self.ui.empty.get()?, state.matching == 0)?;
        }
        if old.is_none_or(|old| old.unseen != state.unseen) {
            if let Some(button) = self.ui.resume.node() {
                self.tree.set_visible(button, state.unseen != 0)?;
            }
            let arrivals = match state.unseen {
                0 => String::new(),
                1 => "1 new record".into(),
                unseen => format!("{} new records", count(unseen)),
            };
            self.tree.set_text(self.ui.arrivals.get()?, arrivals)?;
        }
        self.display.controls = Some(state);
        Ok(true)
    }

    pub(super) fn reveal_key(&mut self, key: u64) -> Result<bool, Error> {
        let node = self.ui.collection.get()?;
        let offset = self.tree.scroll_offset(node)?;
        let bounds = self.tree.bounds(node)?.size();
        let target = match self.mode {
            Mode::List => self
                .list
                .reveal(key, offset.y(), bounds.height())
                .map(|y| Point::new(0., y)),
            Mode::Table => {
                let cell = self.table.selection().map_or(
                    CellKey {
                        row: key,
                        column: Column::Number as u64,
                    },
                    |range| range.active,
                );
                self.table.reveal(cell, offset, bounds)
            }
        };
        if let Some(target) = target
            && target != offset
        {
            return Ok(self.tree.set_scroll_offset(node, target)?);
        }
        Ok(false)
    }
}

fn count(n: usize) -> String {
    let value = n.to_string();
    let mut out = String::with_capacity(value.len() + value.len() / 3);
    for (index, digit) in value.bytes().enumerate() {
        if index != 0 && (value.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(digit));
    }
    out
}
