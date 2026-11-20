use crate::{
    Affinity, Caret, Error, GlyphDraw, Selection, TextLayout, TextStyle, TextSystem,
    edit::{grapheme_boundaries, paragraph_ranges, word_ranges},
};
use oxiview_base::{Affine, Point, Rect, Size};
use std::{
    ops::Range,
    sync::{Arc, OnceLock},
};

const CARET_WIDTH: f64 = 1.0;

/// Cached text drawing and editing geometry in one logical content coordinate space.
#[derive(Clone)]
pub struct EditLayout(Arc<EditLayoutData>);

struct EditLayoutData {
    source: Box<str>,
    style: TextStyle,
    width: Option<f64>,
    paragraphs: Vec<Paragraph>,
    lines: usize,
    bounds: Rect,
}

struct Paragraph {
    start: usize,
    break_end: usize,
    top: f64,
    first_line: usize,
    shape: Arc<ParagraphShape>,
}

struct ParagraphShape {
    text: TextLayout,
    lines: Vec<VisualLine>,
    graphemes: Vec<usize>,
    word_boundaries: OnceLock<Vec<usize>>,
    rtl: bool,
    height: f64,
    left: f64,
    right: f64,
}

struct VisualLine {
    range: Range<usize>,
    top: f64,
    bottom: f64,
    left: f64,
    right: f64,
    fragments: Vec<Fragment>,
    logical: Vec<usize>,
}

struct Fragment {
    range: Range<usize>,
    left: f64,
    right: f64,
    rtl: bool,
}

impl Fragment {
    fn edge(&self, right: bool) -> Caret {
        if right == self.rtl {
            Caret::new(self.range.start)
        } else {
            Caret::with_affinity(self.range.end, Affinity::Upstream)
        }
    }

    fn paint_rect(&self, top: f64, bottom: f64) -> Rect {
        Rect::new(
            self.left,
            top,
            self.right.max(self.left + CARET_WIDTH),
            bottom,
        )
    }
}

#[derive(Clone, Copy)]
struct LineAddress {
    paragraph: usize,
    line: usize,
}

impl TextSystem {
    /// Shapes editable paragraphs using the same internal text layout used for drawing.
    pub fn layout_editable(
        &mut self,
        source: &str,
        style: &TextStyle,
        width: Option<f64>,
    ) -> Result<EditLayout, Error> {
        self.build_edit_layout(None, source, style, width)
    }

    /// Reuses unchanged paragraph shapes and rewraps them when only the width changes.
    pub fn update_editable_layout(
        &mut self,
        previous: &EditLayout,
        source: &str,
        style: &TextStyle,
        width: Option<f64>,
    ) -> Result<EditLayout, Error> {
        if previous.0.source.as_ref() == source
            && &previous.0.style == style
            && previous.0.width == width
        {
            return Ok(previous.clone());
        }
        self.build_edit_layout(Some(previous), source, style, width)
    }

    fn build_edit_layout(
        &mut self,
        previous: Option<&EditLayout>,
        source: &str,
        style: &TextStyle,
        width: Option<f64>,
    ) -> Result<EditLayout, Error> {
        let ranges = paragraph_ranges(source).collect::<Vec<_>>();
        let previous = previous.filter(|old| &old.0.style == style);
        let (prefix, suffix) = previous.map_or((0, 0), |old| {
            let old_paragraphs = &old.0.paragraphs;
            let matching = |(range, _): &(Range<usize>, usize), paragraph: &Paragraph| {
                old.0.source[paragraph.content_range()] == source[range.clone()]
            };
            let prefix = ranges
                .iter()
                .zip(old_paragraphs)
                .take_while(|(new, old)| matching(new, old))
                .count();
            let suffix = ranges[prefix..]
                .iter()
                .rev()
                .zip(old_paragraphs[prefix..].iter().rev())
                .take_while(|(new, old)| matching(new, old))
                .count();
            (prefix, suffix)
        });
        let mut paragraphs = Vec::with_capacity(ranges.len());
        let mut top = 0.0;
        let mut first_line = 0;
        let mut left: f64 = 0.0;
        let mut right: f64 = 0.0;
        let count = ranges.len();
        let break_width = style.font_size * 0.4;
        for (index, (range, break_end)) in ranges.into_iter().enumerate() {
            let paragraph_text = &source[range.clone()];
            let cached = previous.and_then(|old| {
                if index < prefix {
                    old.0.paragraphs.get(index)
                } else if count - index <= suffix {
                    old.0
                        .paragraphs
                        .get(old.0.paragraphs.len() - (count - index))
                } else {
                    None
                }
            });
            let shape = match cached {
                Some(paragraph) if previous.is_some_and(|old| old.0.width == width) => {
                    paragraph.shape.clone()
                }
                Some(paragraph) => Arc::new(ParagraphShape::new(
                    paragraph.shape.text.rewrap(width)?,
                    paragraph_text,
                    style,
                )),
                None => Arc::new(ParagraphShape::new(
                    self.layout(paragraph_text, style, width)?,
                    paragraph_text,
                    style,
                )),
            };
            left = left.min(shape.left);
            right = right.max(shape.right + CARET_WIDTH);
            let paragraph = Paragraph {
                start: range.start,
                break_end,
                top,
                first_line,
                shape,
            };
            if let Some(rect) = paragraph.break_rect(break_width) {
                let [x0, _, x1, _] = rect.edges();
                left = left.min(x0);
                right = right.max(x1);
            }
            top += paragraph.shape.height;
            first_line += paragraph.shape.lines.len();
            paragraphs.push(paragraph);
        }
        if ![left, right, top].into_iter().all(f64::is_finite) {
            return Err(Error::InvalidInput);
        }
        Ok(EditLayout(Arc::new(EditLayoutData {
            source: source.into(),
            style: style.clone(),
            width,
            paragraphs,
            lines: first_line,
            bounds: Rect::new(left, 0.0, right, top),
        })))
    }

    /// Draws only the lines intersecting the supplied vertical interval in layout space.
    pub fn editable_glyph_draws(
        &mut self,
        layout: &EditLayout,
        origin: Point,
        local_to_device: Affine,
        visible_y: Range<f64>,
    ) -> Result<Vec<GlyphDraw>, Error> {
        let mut glyphs = Vec::new();
        for paragraph in layout.paragraphs_for_y(&visible_y) {
            let range = paragraph.lines_for_y(&visible_y);
            let origin = Point::new(origin.x(), origin.y() + paragraph.top);
            // Include a neighboring line when ink can extend beyond the line box.
            let lines = range.start.saturating_sub(1)..range.end.saturating_add(1);
            glyphs.extend(self.glyph_draws_in_lines(
                &paragraph.shape.text,
                origin,
                local_to_device,
                lines,
            )?);
        }
        Ok(glyphs)
    }
}

impl EditLayout {
    /// Full scrollable bounds, including the caret width and hanging whitespace.
    pub fn bounds(&self) -> Rect {
        self.0.bounds
    }

    /// Logical extent occupied by the editable text.
    pub fn size(&self) -> Size {
        self.bounds().size()
    }

    /// Includes both visual wraps and the empty line after a trailing newline.
    pub fn line_count(&self) -> usize {
        self.0.lines
    }

    /// Nearest visual caret on the hit line.
    pub fn hit_test(&self, point: Point) -> Caret {
        let paragraph = &self.0.paragraphs[self.paragraph_at_y(point.y())];
        let index = paragraph.line_at_y(point.y());
        paragraph.global_caret(paragraph.shape.caret_at_x(index, point.x()))
    }

    /// Logical caret rectangle on the side identified by its affinity.
    pub fn caret_rect(&self, caret: Caret) -> Option<Rect> {
        let resolved = self.resolve(caret)?;
        let (paragraph, line) = self.line(resolved.address);
        let x = line.x(resolved.caret);
        Some(Rect::new(
            x,
            paragraph.top + line.top,
            x + CARET_WIDTH,
            paragraph.top + line.bottom,
        ))
    }

    /// Disjoint visual rectangles covering a UTF-8 range.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        self.selection_rects_inner(range, None)
    }

    /// Selection rectangles on the lines intersecting a vertical viewport.
    pub fn visible_selection_rects(&self, range: Range<usize>, visible_y: Range<f64>) -> Vec<Rect> {
        self.selection_rects_inner(range, Some(&visible_y))
    }

    fn selection_rects_inner(
        &self,
        range: Range<usize>,
        visible_y: Option<&Range<f64>>,
    ) -> Vec<Rect> {
        if range.is_empty() || self.0.source.get(range.clone()).is_none() {
            return Vec::new();
        }
        let first = self.paragraph_at_offset(range.start);
        let last = self.paragraph_at_offset(range.end - 1);
        let paragraphs = &self.0.paragraphs[first..=last];
        let paragraphs = if let Some(visible_y) = visible_y {
            let start = paragraphs.partition_point(|p| p.top + p.shape.height <= visible_y.start);
            let end = paragraphs
                .partition_point(|p| p.top < visible_y.end)
                .max(start);
            &paragraphs[start..end]
        } else {
            paragraphs
        };
        let mut result = Vec::new();
        for paragraph in paragraphs {
            let lines =
                visible_y.map_or(0..paragraph.shape.lines.len(), |y| paragraph.lines_for_y(y));
            let local = range.start.saturating_sub(paragraph.start)..range.end - paragraph.start;
            for index in lines {
                let line = &paragraph.shape.lines[index];
                if !overlaps(&line.range, &local) && local.start < line.range.end {
                    continue;
                }
                let mut segment = None;
                for fragment in &line.fragments {
                    if overlaps(&fragment.range, &local) {
                        let rect = fragment
                            .paint_rect(paragraph.top + line.top, paragraph.top + line.bottom);
                        if let Some(current) = segment
                            && let Some(joined) = join_rects(current, rect)
                        {
                            segment = Some(joined);
                        } else if let Some(previous) = segment.replace(rect) {
                            result.push(previous);
                        }
                    }
                }
                if let Some(rect) = segment {
                    result.push(rect);
                }
                if index + 1 == paragraph.shape.lines.len()
                    && range.end > paragraph.content_range().end
                    && let Some(rect) = paragraph.break_rect(self.0.style.font_size * 0.4)
                {
                    result.push(rect);
                }
            }
        }
        result
    }

    /// First connected rectangle and the portion of the range it covers.
    /// A collapsed range returns a caret rectangle.
    pub fn first_rect(&self, range: Range<usize>) -> Option<(Range<usize>, Rect)> {
        self.0.source.get(range.clone())?;
        if range.is_empty() {
            return self
                .caret_rect(Caret::new(range.start))
                .map(|rect| (range, rect));
        }
        let paragraph = &self.0.paragraphs[self.paragraph_at_offset(range.start)];
        let content_end = paragraph.content_range().end;
        if range.start >= content_end {
            let rect = paragraph.break_rect(self.0.style.font_size * 0.4)?;
            return Some((range.start..range.end.min(paragraph.break_end), rect));
        }
        let local_start = range.start - paragraph.start;
        let local_end = range.end.min(content_end) - paragraph.start;
        let line = &paragraph.shape.lines[paragraph.shape.line_at_offset(Caret::new(local_start))];
        let top = paragraph.top + line.top;
        let bottom = paragraph.top + line.bottom;
        let mut fragments = line
            .logical
            .iter()
            .map(|&index| &line.fragments[index])
            .skip_while(|fragment| fragment.range.end <= local_start);
        let first = fragments.next()?;
        let mut rect = first.paint_rect(top, bottom);
        let mut actual_end = first.range.end.min(local_end);
        for fragment in fragments.take_while(|fragment| fragment.range.start < local_end) {
            if fragment.range.start != actual_end {
                break;
            }
            let next = fragment.paint_rect(top, bottom);
            let Some(joined) = join_rects(rect, next) else {
                break;
            };
            actual_end = fragment.range.end.min(local_end);
            rect = joined;
        }
        Some((range.start..paragraph.start + actual_end, rect))
    }

    /// Individual text rectangles in logical order for synchronous native queries.
    pub fn visible_range_rects(&self, visible_y: Range<f64>) -> Vec<(Range<usize>, Rect)> {
        let mut result = Vec::new();
        for paragraph in self.paragraphs_for_y(&visible_y) {
            for index in paragraph.lines_for_y(&visible_y) {
                let line = &paragraph.shape.lines[index];
                let top = paragraph.top + line.top;
                let bottom = paragraph.top + line.bottom;
                for &index in &line.logical {
                    let fragment = &line.fragments[index];
                    let range = paragraph.start + fragment.range.start
                        ..paragraph.start + fragment.range.end;
                    result.push((range, fragment.paint_rect(top, bottom)));
                }
            }
            let end = paragraph.content_range().end;
            if let Some(rect) = paragraph.break_rect(self.0.style.font_size * 0.4)
                && rect.edges()[1] < visible_y.end
                && visible_y.start < rect.edges()[3]
            {
                result.push((end..paragraph.break_end, rect));
            }
        }
        result
    }

    /// Moves one visual caret to the left or right, crossing wraps when needed.
    pub fn move_visual(&self, caret: Caret, forward: bool) -> Caret {
        let Some(resolved) = self.resolve(caret) else {
            return caret;
        };
        let (paragraph, line) = self.line(resolved.address);
        let x = line.x(resolved.caret);
        let local = if forward {
            let next = line
                .fragments
                .partition_point(|fragment| fragment.right <= x);
            line.fragments.get(next).map(|fragment| fragment.edge(true))
        } else {
            let previous = line.fragments.partition_point(|fragment| fragment.left < x);
            previous
                .checked_sub(1)
                .and_then(|index| line.fragments.get(index))
                .map(|fragment| fragment.edge(false))
        };
        if let Some(local) = local {
            return paragraph.global_caret(paragraph.shape.snap(local));
        }
        let toward_next_line = forward != paragraph.shape.rtl;
        let Some(adjacent) = self.adjacent(resolved.address, toward_next_line) else {
            return caret;
        };
        let (target, line) = self.line(adjacent);
        let edge = line.edge(!forward);
        target.global_caret(target.shape.snap(edge))
    }

    /// Moves visually by one word. Separators are crossed to reach a word end.
    pub fn move_word(&self, caret: Caret, forward: bool) -> Caret {
        let mut current = caret;
        // An affinity change can revisit the same byte on another visual edge.
        // Each nonempty visual movement crosses at least one grapheme or wrap.
        for _ in 0..self.0.source.len().saturating_add(self.0.lines) {
            let next = self.move_visual(current, forward);
            if next == current {
                return current;
            }
            let paragraph = &self.0.paragraphs[self.paragraph_at_offset(next.offset)];
            let local_offset = next.offset - paragraph.start;
            let boundaries = paragraph.shape.word_boundaries.get_or_init(|| {
                word_ranges(&self.0.source[paragraph.content_range()])
                    .flat_map(|word| [word.start, word.end])
                    .collect()
            });
            if next.offset != caret.offset && boundaries.binary_search(&local_offset).is_ok() {
                return next;
            }
            current = next;
        }
        current
    }

    /// Chooses the corresponding horizontal position on the adjacent visual line.
    pub fn move_vertical(&self, caret: Caret, down: bool, desired_x: f64) -> Caret {
        let Some(resolved) = self.resolve(caret) else {
            return caret;
        };
        let Some(adjacent) = self.adjacent(resolved.address, down) else {
            return caret;
        };
        let (paragraph, _) = self.line(adjacent);
        paragraph.global_caret(paragraph.shape.caret_at_x(adjacent.line, desired_x))
    }

    /// Visual leading or trailing edge of the current wrapped line.
    pub fn line_edge(&self, caret: Caret, end: bool) -> Caret {
        let Some(resolved) = self.resolve(caret) else {
            return caret;
        };
        let (paragraph, line) = self.line(resolved.address);
        paragraph.global_caret(paragraph.shape.snap(line.edge(end != paragraph.shape.rtl)))
    }

    /// Collapses a nonempty selection toward the visually requested side.
    pub fn selection_edge(&self, selection: Selection, right: bool) -> Caret {
        let a = self.caret_rect(selection.anchor).map(|r| r.origin());
        let b = self.caret_rect(selection.active).map(|r| r.origin());
        if let Some((a, b)) = a.zip(b) {
            let active_is_right = (a.y(), a.x()) <= (b.y(), b.x());
            if active_is_right == right {
                return selection.active;
            }
        }
        selection.anchor
    }

    fn line(&self, address: LineAddress) -> (&Paragraph, &VisualLine) {
        let paragraph = &self.0.paragraphs[address.paragraph];
        (paragraph, &paragraph.shape.lines[address.line])
    }

    fn adjacent(&self, address: LineAddress, next: bool) -> Option<LineAddress> {
        let (paragraph, _) = self.line(address);
        let line = paragraph.first_line + address.line;
        let target = if next {
            line.checked_add(1)?
        } else {
            line.checked_sub(1)?
        };
        if target >= self.0.lines {
            return None;
        }
        let paragraph = self
            .0
            .paragraphs
            .partition_point(|paragraph| paragraph.first_line <= target)
            - 1;
        Some(LineAddress {
            paragraph,
            line: target - self.0.paragraphs[paragraph].first_line,
        })
    }

    fn paragraphs_for_y(&self, visible_y: &Range<f64>) -> &[Paragraph] {
        let start = self
            .0
            .paragraphs
            .partition_point(|p| p.top + p.shape.height <= visible_y.start);
        let end = self
            .0
            .paragraphs
            .partition_point(|p| p.top < visible_y.end)
            .max(start);
        &self.0.paragraphs[start..end]
    }

    fn paragraph_at_y(&self, y: f64) -> usize {
        self.0
            .paragraphs
            .partition_point(|p| p.top <= y)
            .saturating_sub(1)
    }

    fn paragraph_at_offset(&self, offset: usize) -> usize {
        self.0
            .paragraphs
            .partition_point(|p| p.start <= offset)
            .saturating_sub(1)
    }

    fn resolve(&self, caret: Caret) -> Option<ResolvedCaret> {
        if !self.0.source.is_char_boundary(caret.offset) {
            return None;
        }
        let index = self.paragraph_at_offset(caret.offset);
        let paragraph = &self.0.paragraphs[index];
        let local = Caret::with_affinity(
            (caret.offset - paragraph.start).min(paragraph.shape.text.layout.text_len()),
            caret.affinity,
        );
        let caret = paragraph.shape.snap(local);
        let line = paragraph.shape.line_at_offset(caret);
        Some(ResolvedCaret {
            address: LineAddress {
                paragraph: index,
                line,
            },
            caret,
        })
    }
}

struct ResolvedCaret {
    address: LineAddress,
    caret: Caret,
}

impl Paragraph {
    fn content_range(&self) -> Range<usize> {
        self.start..self.start + self.shape.text.layout.text_len()
    }

    fn global_caret(&self, caret: Caret) -> Caret {
        Caret::with_affinity(self.start + caret.offset, caret.affinity)
    }

    fn line_at_y(&self, y: f64) -> usize {
        self.shape
            .lines
            .partition_point(|line| line.top <= y - self.top)
            .saturating_sub(1)
    }

    fn lines_for_y(&self, visible_y: &Range<f64>) -> Range<usize> {
        let start = self
            .shape
            .lines
            .partition_point(|line| self.top + line.bottom <= visible_y.start);
        let end = self
            .shape
            .lines
            .partition_point(|line| self.top + line.top < visible_y.end)
            .max(start);
        start..end
    }

    fn break_rect(&self, width: f64) -> Option<Rect> {
        let end = self.shape.text.layout.text_len();
        if self.start + end == self.break_end {
            return None;
        }
        let last = self.shape.lines.last()?;
        let x = last.x(Caret::with_affinity(end, Affinity::Upstream));
        let left = if self.shape.rtl { x - width } else { x };
        Some(Rect::new(
            left,
            self.top + last.top,
            left + width,
            self.top + last.bottom,
        ))
    }
}

impl ParagraphShape {
    fn new(text: TextLayout, source: &str, style: &TextStyle) -> Self {
        let graphemes: Vec<_> = grapheme_boundaries(source).collect();
        let rtl = text.layout.is_rtl();
        let mut lines = Vec::new();
        for line in text.layout.lines() {
            let metrics = line.metrics();
            if metrics.block_min_coord == metrics.block_max_coord {
                continue;
            }
            let left = f64::from(metrics.offset + metrics.inline_min_coord);
            let mut right = left;
            let mut fragments = Vec::new();
            for run in line.runs() {
                for cluster in run.visual_clusters() {
                    let range = cluster.text_range();
                    let advance = f64::from(cluster.advance()).max(0.0);
                    let rtl = cluster.is_rtl();
                    split_fragment(&mut fragments, &graphemes, range, right, advance, rtl);
                    right += advance;
                }
            }
            let mut logical: Vec<_> = (0..fragments.len()).collect();
            // Runs are in visual order. This index preserves the logical side at a bidi split.
            logical.sort_unstable_by_key(|&index| fragments[index].range.start);
            lines.push(VisualLine {
                range: line.text_range(),
                top: f64::from(metrics.block_min_coord),
                bottom: f64::from(metrics.block_max_coord),
                left,
                right,
                fragments,
                logical,
            });
        }
        let height = lines
            .last()
            .map_or(style.font_size * style.line_height, |line| line.bottom);
        if lines.is_empty() {
            lines.push(VisualLine {
                range: 0..source.len(),
                top: 0.0,
                bottom: height,
                left: 0.0,
                right: 0.0,
                fragments: Vec::new(),
                logical: Vec::new(),
            });
        }
        let left = lines.iter().map(|line| line.left).fold(0.0_f64, f64::min);
        let right = lines.iter().map(|line| line.right).fold(0.0_f64, f64::max);
        Self {
            text,
            lines,
            graphemes,
            word_boundaries: OnceLock::new(),
            rtl,
            height,
            left,
            right,
        }
    }

    fn snap(&self, caret: Caret) -> Caret {
        let offset = match self.graphemes.binary_search(&caret.offset) {
            Ok(_) => caret.offset,
            Err(index) => {
                let index = if caret.affinity == Affinity::Downstream {
                    index.saturating_sub(1)
                } else {
                    index
                };
                self.graphemes
                    .get(index)
                    .copied()
                    .unwrap_or(self.text.layout.text_len())
            }
        };
        Caret::with_affinity(offset, caret.affinity)
    }

    fn line_at_offset(&self, caret: Caret) -> usize {
        let index = self
            .lines
            .partition_point(|line| line.range.start <= caret.offset)
            .saturating_sub(1);
        if caret.affinity == Affinity::Upstream
            && index > 0
            && self.lines[index - 1].range.end == caret.offset
        {
            index - 1
        } else {
            index
        }
    }

    fn caret_at_x(&self, line_index: usize, x: f64) -> Caret {
        let line = &self.lines[line_index];
        let index = line
            .fragments
            .partition_point(|fragment| fragment.right < x);
        let caret = line.fragments.get(index).map_or_else(
            || line.edge(true),
            |fragment| fragment.edge(x > (fragment.left + fragment.right) * 0.5),
        );
        self.snap(caret)
    }
}

impl VisualLine {
    fn edge(&self, right: bool) -> Caret {
        let fragment = if right {
            self.fragments.last()
        } else {
            self.fragments.first()
        };
        fragment.map_or(Caret::new(self.range.start), |fragment| {
            fragment.edge(right)
        })
    }

    fn x(&self, caret: Caret) -> f64 {
        let index = self
            .logical
            .partition_point(|&i| self.fragments[i].range.start < caret.offset);
        let previous = index
            .checked_sub(1)
            .map(|i| &self.fragments[self.logical[i]]);
        let next = self.logical.get(index).map(|&i| &self.fragments[i]);
        match (caret.affinity, previous, next) {
            (Affinity::Upstream, Some(fragment), _) | (_, Some(fragment), None) => {
                if fragment.rtl {
                    fragment.left
                } else {
                    fragment.right
                }
            }
            (_, _, Some(fragment)) => {
                if fragment.rtl {
                    fragment.right
                } else {
                    fragment.left
                }
            }
            _ => self.left,
        }
    }
}

fn split_fragment(
    fragments: &mut Vec<Fragment>,
    boundaries: &[usize],
    range: Range<usize>,
    left: f64,
    advance: f64,
    rtl: bool,
) {
    let first = boundaries.partition_point(|&offset| offset <= range.start);
    let last = boundaries.partition_point(|&offset| offset < range.end);
    let interior = &boundaries[first..last.max(first)];
    let count = interior.len() + 1;
    if count == 1 {
        fragments.push(Fragment {
            range,
            left,
            right: left + advance,
            rtl,
        });
        return;
    }
    // Parley normally supplies one cluster per grapheme. Subdivide a shaped
    // cluster if Unicode segmentation supplies additional insertion positions.
    let cuts = std::iter::once(range.start)
        .chain(interior.iter().copied())
        .chain(std::iter::once(range.end));
    let mut ranges = cuts
        .clone()
        .zip(cuts.skip(1))
        .map(|(start, end)| start..end)
        .collect::<Vec<_>>();
    if rtl {
        ranges.reverse();
    }
    let width = advance / count as f64;
    let mut x = left;
    for range in ranges {
        fragments.push(Fragment {
            range,
            left: x,
            right: x + width,
            rtl,
        });
        x += width;
    }
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn join_rects(a: Rect, b: Rect) -> Option<Rect> {
    let [ax0, ay0, ax1, ay1] = a.edges();
    let [bx0, by0, bx1, by1] = b.edges();
    (ay0 == by0 && ay1 == by1 && ax1 + 0.001 >= bx0 && bx1 + 0.001 >= ax0)
        .then(|| Rect::new(ax0.min(bx0), ay0, ax1.max(bx1), ay1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn editing_a_paragraph_reuses_unchanged_shaping_on_both_sides() {
        let mut system = TextSystem::without_system_fonts();
        system
            .register_font(include_bytes!("../tests/fonts/NotoSans.ttf").to_vec())
            .unwrap();
        let style = TextStyle {
            family: Some("Noto Sans".into()),
            ..TextStyle::default()
        };
        let old = system
            .layout_editable("first\nsecond\nthird", &style, Some(90.0))
            .unwrap();
        let new = system
            .update_editable_layout(
                &old,
                "first\nnew and longer second\nthird",
                &style,
                Some(90.0),
            )
            .unwrap();
        for index in [0, 2] {
            assert!(Arc::ptr_eq(
                &old.0.paragraphs[index].shape,
                &new.0.paragraphs[index].shape
            ));
        }
        assert!(!Arc::ptr_eq(
            &old.0.paragraphs[1].shape,
            &new.0.paragraphs[1].shape
        ));
        assert!(new.0.paragraphs[2].top > old.0.paragraphs[2].top);
    }
}
