//! Captured collection gestures, selection, and keyboard navigation.

use crate::{
    app::{Mode, ViewerApp, error, presentation::column},
    model::Column,
    view,
};
use oxiview_base::Point;
use oxiview_core::{Error, PointerEventKind, UiPointerEvent};
use oxiview_widgets::collection::CellKey;

#[derive(Clone, Copy)]
pub(crate) enum Navigation {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    First,
    Last,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct RecordDrag {
    position: Option<Point>,
}

#[derive(Clone, Copy)]
pub(in crate::app) enum HeadingDrag {
    Sort { column: Column, x: f64 },
    Resize { column: Column, x: f64, width: f64 },
}

impl ViewerApp {
    pub(super) fn toggle_expanded(&mut self, key: u64) {
        self.expanded = (self.expanded != Some(key)).then_some(key);
        self.display.layout_revision = self.display.layout_revision.wrapping_add(1);
    }

    pub(super) fn sort(&mut self, column: Column) {
        self.book.sort_by(column);
        self.display.sort_or_filter = true;
    }

    pub(super) fn heading(&mut self, event: UiPointerEvent) -> Result<bool, Error> {
        if !self.is_table() {
            return Ok(false);
        }
        let scroll = self.tree.scroll_offset(self.ui.collection.get()?)?;
        let content_x = event.local_position.x() + scroll.x();
        let hit = || {
            self.table.column_at(content_x).and_then(|index| {
                let key = self.table.column_key(index)?;
                let width = self.table.column_width(index)?;
                let end = self.table.column_offset(index)? + width;
                Some((column(key)?, width, end))
            })
        };
        match event.kind {
            PointerEventKind::Down => {
                // Test just inside the boundary so its hit target is the column being resized.
                let grip = (content_x - 5.).max(0.);
                let resizing = self.table.column_at(grip).and_then(|index| {
                    let end = self.table.column_offset(index)? + self.table.column_width(index)?;
                    if (end - content_x).abs() > 5. {
                        return None;
                    }
                    Some(HeadingDrag::Resize {
                        column: column(self.table.column_key(index)?)?,
                        x: event.position.x(),
                        width: self.table.column_width(index)?,
                    })
                });
                self.heading_drag = resizing.or_else(|| {
                    hit().map(|(column, _, _)| HeadingDrag::Sort {
                        column,
                        x: event.position.x(),
                    })
                });
            }
            PointerEventKind::Move => {
                if let Some(HeadingDrag::Resize { column, x, width }) = self.heading_drag {
                    let width = (width + event.position.x() - x).clamp(48., 1400.);
                    if self
                        .table
                        .resize_column(column as u64, width)
                        .map_err(error)?
                    {
                        self.display.layout_revision = self.display.layout_revision.wrapping_add(1);
                        return Ok(true);
                    }
                }
            }
            PointerEventKind::Up => {
                if let Some(HeadingDrag::Sort { column, x }) = self.heading_drag.take()
                    && (event.position.x() - x).abs() < 5.
                    && (0. ..self.table.header_height()).contains(&event.local_position.y())
                    && hit().is_some_and(|(hit, _, _)| hit == column)
                {
                    self.sort(column);
                    return Ok(true);
                }
            }
            PointerEventKind::Cancel => self.heading_drag = None,
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn record_pointer(&mut self, event: UiPointerEvent) -> Result<bool, Error> {
        let body = self.ui.collection.get()?;
        let offset = self.tree.scroll_offset(body)?;
        match (self.mode, event.kind) {
            (Mode::List, PointerEventKind::Down) => {
                let content_y = event.local_position.y() + offset.y();
                if let Some(key) = self
                    .list
                    .index_at(content_y)
                    .and_then(|index| self.list.key_at(index))
                {
                    self.list_key = Some(key);
                    if event.local_position.x() < view::DISCLOSURE_EDGE {
                        self.toggle_expanded(key);
                    }
                    return Ok(true);
                }
            }
            (Mode::Table, PointerEventKind::Down) => {
                if let Some(cell) = self.cell_at(event.position, false)? {
                    self.record_drag = (cell.column != Column::Number as u64)
                        .then_some(RecordDrag { position: None });
                    return Ok(self.table.select(cell, false));
                }
            }
            (Mode::Table, PointerEventKind::Move) => {
                if let Some(drag) = &mut self.record_drag {
                    drag.position = Some(event.position);
                    if let Some(cell) = self.cell_at(event.position, true)? {
                        return Ok(self.table.select(cell, true));
                    }
                }
            }
            (Mode::Table, PointerEventKind::Up) => {
                if self
                    .record_drag
                    .take()
                    .is_some_and(|drag| drag.position.is_some())
                    && let Some(cell) = self.cell_at(event.position, true)?
                {
                    return Ok(self.table.select(cell, true));
                }
            }
            (_, PointerEventKind::Cancel) => self.record_drag = None,
            _ => {}
        }
        Ok(false)
    }

    fn cell_at(&self, position: Point, clamp_to_view: bool) -> Result<Option<CellKey>, Error> {
        let body = self.ui.collection.get()?;
        let bounds = self.tree.bounds(body)?;
        let offset = self.tree.scroll_offset(body)?;
        let total = self.table.total_size();
        let origin = bounds.origin();
        let local = Point::new(position.x() - origin.x(), position.y() - origin.y());
        let Some(x) = axis_point(
            local.x(),
            bounds.size().width(),
            offset.x(),
            total.width(),
            clamp_to_view,
        ) else {
            return Ok(None);
        };
        let Some(y) = axis_point(
            local.y(),
            bounds.size().height(),
            offset.y(),
            total.height(),
            clamp_to_view,
        ) else {
            return Ok(None);
        };
        let column = self
            .table
            .column_at(x)
            .and_then(|index| self.table.column_key(index));
        let row = self
            .table
            .row_at(y)
            .and_then(|index| self.table.row_key(index));
        Ok(row.zip(column).map(|(row, column)| CellKey { row, column }))
    }

    fn drag_target(&self) -> Result<Option<(Point, Point)>, Error> {
        let Some(position) = self.record_drag.and_then(|drag| drag.position) else {
            return Ok(None);
        };
        let body = self.ui.collection.get()?;
        let bounds = self.tree.bounds(body)?;
        let offset = self.tree.scroll_offset(body)?;
        let max = self.tree.scroll_max_offset(body)?;
        let [left, top, right, bottom] = bounds.edges();
        let target = Point::new(
            (offset.x() + drag_distance(position.x(), left, right)).clamp(0., max.x()),
            (offset.y() + drag_distance(position.y(), top, bottom)).clamp(0., max.y()),
        );
        Ok((target != offset).then_some((position, target)))
    }

    pub(in crate::app) fn drag_can_scroll(&self) -> Result<bool, Error> {
        Ok(self.drag_target()?.is_some())
    }

    pub(in crate::app) fn advance_drag(&mut self) -> Result<bool, Error> {
        let Some((position, target)) = self.drag_target()? else {
            return Ok(false);
        };
        let body = self.ui.collection.get()?;
        let changed = self.tree.set_scroll_offset(body, target)?;
        if let Some(cell) = self.cell_at(position, true)? {
            return Ok(self.table.select(cell, true) || changed);
        }
        Ok(changed)
    }

    pub(super) fn navigate(&mut self, direction: Navigation, extend: bool) -> Result<bool, Error> {
        if self.book.is_empty() {
            return Ok(false);
        }
        let body = self.ui.collection.get()?;
        let before = self.tree.scroll_offset(body)?;
        let height = self.tree.bounds(body)?.size().height();
        let selection = self.active_key();
        let key = if self.is_table() {
            let page = ((height / self.table.row_height()).floor() as isize - 1).max(1);
            let (row, column) = navigation_delta(direction, page);
            if selection.is_none() {
                let current = match direction {
                    Navigation::First => self.table.row_key(0),
                    Navigation::Last => {
                        self.table.row_key(self.table.row_count().saturating_sub(1))
                    }
                    _ => self.first_visible_key(),
                };
                current.and_then(|row| {
                    let cell = CellKey {
                        row,
                        column: Column::Number as u64,
                    };
                    self.table.select(cell, false).then_some(row)
                })
            } else {
                self.table
                    .navigate(row, column, extend)
                    .map(|cell| cell.row)
            }
        } else {
            let page = ((height / view::LIST_HEIGHT).floor() as isize - 1).max(1);
            let (delta, _) = navigation_delta(direction, page);
            let key = if let Some(key) = selection {
                self.list.index_of(key).and_then(|index| {
                    self.list.key_at(
                        index
                            .saturating_add_signed(delta)
                            .min(self.list.len().saturating_sub(1)),
                    )
                })
            } else {
                match direction {
                    Navigation::First => self.list.key_at(0),
                    Navigation::Last => self.list.key_at(self.list.len().saturating_sub(1)),
                    _ => self.first_visible_key(),
                }
            };
            if matches!(direction, Navigation::Left | Navigation::Right)
                && let Some(key) = key
            {
                let expand = matches!(direction, Navigation::Right);
                if expand != (self.expanded == Some(key)) {
                    self.toggle_expanded(key);
                }
            }
            self.list_key = key;
            key
        };
        if let Some(key) = key {
            self.reveal_key(key)?;
            self.pause_if_scrolled_back(before)?;
        }
        Ok(key.is_some())
    }
}

fn axis_point(local: f64, size: f64, offset: f64, total: f64, clamp: bool) -> Option<f64> {
    if size <= 0. || total <= 0. {
        return None;
    }
    let max_local = (size - 0.01).max(0.);
    if clamp {
        return Some((local.clamp(0., max_local) + offset).min((total - 0.01).max(0.)));
    }
    ((0. ..size).contains(&local) && local + offset < total).then_some(local + offset)
}

fn drag_distance(position: f64, start: f64, end: f64) -> f64 {
    const MARGIN: f64 = 15.;
    if end - start <= 2. * MARGIN {
        return 0.;
    }
    if position < start + MARGIN {
        -((start + MARGIN - position) * 0.55).clamp(2., 35.)
    } else if position > end - MARGIN {
        ((position - end + MARGIN) * 0.55).clamp(2., 35.)
    } else {
        0.
    }
}

fn navigation_delta(direction: Navigation, page: isize) -> (isize, isize) {
    match direction {
        Navigation::Up => (-1, 0),
        Navigation::Down => (1, 0),
        Navigation::Left => (0, -1),
        Navigation::Right => (0, 1),
        Navigation::PageUp => (-page, 0),
        Navigation::PageDown => (page, 0),
        Navigation::First => (isize::MIN, 0),
        Navigation::Last => (isize::MAX, 0),
    }
}
