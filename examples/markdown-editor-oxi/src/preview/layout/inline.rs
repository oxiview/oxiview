use oxiview_base::{Color, Size};
use oxiview_text::{
    Error, RichText, TextAlign, TextLayout, TextSpan, TextSpanStyle, TextStyle, TextSystem,
};
use std::{ops::Range, sync::Arc};
use unicode_bidi::{BidiInfo, ParagraphInfo};
use unicode_linebreak::linebreaks;

use super::{Paint, error_color};
use crate::preview::{Diagnostic, Graphic, Inline, InlineKind, Marks};

pub(super) struct Paragraph {
    pub(super) lines: Vec<Line>,
    pub(super) height: f64,
}

pub(super) struct Line {
    pub(super) pieces: Vec<Piece>,
    pub(super) offset: f64,
    pub(super) height: f64,
    pub(super) baseline: f64,
}

pub(super) struct Piece {
    pub(super) paint: Paint,
    pub(super) size: Size,
    pub(super) baseline: f64,
}

impl Piece {
    fn text(layout: TextLayout) -> Self {
        Self {
            size: layout.size(),
            baseline: layout.first_baseline().unwrap_or(0.0),
            paint: Paint::Text(layout),
        }
    }

    pub(super) fn graphic(graphic: Arc<Graphic>, em: f64, width: f64) -> Self {
        let scale = fit_graphic(&graphic, em, width);
        Self {
            size: Size::new(graphic.size.width() * scale, graphic.size.height() * scale),
            baseline: graphic.baseline * scale,
            paint: Paint::Graphic(graphic, scale),
        }
    }
}

fn fit_graphic(graphic: &Graphic, em: f64, width: f64) -> f64 {
    if graphic.size.width() > 0.0 {
        em.min(width / graphic.size.width())
    } else {
        em
    }
}

pub(super) fn paragraph(
    text: &mut TextSystem,
    input: &[Inline],
    diagnostics: &[Diagnostic],
    style: &TextStyle,
    width: f64,
) -> Result<Paragraph, Error> {
    let flow = Flow::new(input, diagnostics);
    let lines = if flow.objects.is_empty() {
        // Keep native shaping, bidi, wrapping, and decorations together for
        // ordinary paragraphs, including the CommonMark hard-break case.
        let rich = RichText::new(flow.text, flow.styles)?;
        let piece = Piece::text(text.layout_rich(&rich, style, Some(width))?);
        vec![Line {
            offset: 0.0,
            height: piece.size.height(),
            baseline: piece.baseline,
            pieces: vec![piece],
        }]
    } else {
        flow.lines(text, style, width)?
    };
    let height = lines.iter().map(|line| line.height).sum();
    Ok(Paragraph { lines, height })
}

#[derive(Default)]
struct Flow {
    text: String,
    styles: Vec<TextSpan>,
    objects: Vec<Object>,
}

struct Object {
    range: Range<usize>,
    graphic: Arc<Graphic>,
    math: bool,
}

impl Flow {
    fn new(input: &[Inline], diagnostics: &[Diagnostic]) -> Self {
        let mut flow = Self::default();
        for run in input {
            match &run.kind {
                InlineKind::Text(text) => {
                    flow.push_text(text, style_for_marks(run.marks));
                }
                InlineKind::Diagnostic { index, .. } => {
                    let style = TextSpanStyle {
                        color: Some(error_color()),
                        ..TextSpanStyle::default()
                    };
                    if let Some(diagnostic) = diagnostics.get(*index) {
                        flow.push_text(&format!("‹{diagnostic}›"), style);
                    }
                }
                InlineKind::Graphic { graphic, math } => flow.push_object(graphic, *math),
                InlineKind::DisplayMath(graphic) => flow.push_object(graphic, true),
            }
        }
        flow
    }

    fn push_text(&mut self, text: &str, style: TextSpanStyle) {
        let start = self.text.len();
        self.text.push_str(text);
        if !text.is_empty() && style != TextSpanStyle::default() {
            if let Some(last) = self.styles.last_mut()
                && last.range.end == start
                && last.style == style
            {
                last.range.end = self.text.len();
            } else {
                self.styles.push(TextSpan {
                    range: start..self.text.len(),
                    style,
                });
            }
        }
    }

    fn push_object(&mut self, graphic: &Arc<Graphic>, math: bool) {
        let start = self.text.len();
        self.text.push('\u{fffc}');
        self.objects.push(Object {
            range: start..self.text.len(),
            graphic: graphic.clone(),
            math,
        });
    }

    fn lines(
        &self,
        text: &mut TextSystem,
        style: &TextStyle,
        width: f64,
    ) -> Result<Vec<Line>, Error> {
        let bidi = BidiInfo::new(&self.text, None);
        let mut lines = Vec::new();
        let mut fragment_style = style.clone();
        fragment_style.align = TextAlign::Start;
        let default_height = style.font_size * style.line_height;

        for paragraph in &bidi.paragraphs {
            let mut start = paragraph.range.start;
            let end = paragraph.range.end;
            let mut fitting = None;
            for (offset, _) in linebreaks(&self.text[start..end]) {
                let break_at = paragraph.range.start + offset;
                let range = self.trimmed(start..break_at);
                let pieces = self.pieces(text, &fragment_style, width, range, None)?;
                let advance = pieces.iter().map(|part| part.size.width()).sum::<f64>();
                if advance > width
                    && let Some(previous) = fitting.take()
                {
                    lines.push(self.visual_line(
                        text,
                        &fragment_style,
                        style.align,
                        width,
                        &bidi,
                        paragraph,
                        self.trimmed(start..previous),
                    )?);
                    start = previous;
                    let range = self.trimmed(start..break_at);
                    let pending_width = self
                        .pieces(text, &fragment_style, width, range.clone(), None)?
                        .iter()
                        .map(|part| part.size.width())
                        .sum::<f64>();
                    if pending_width > width {
                        lines.push(self.visual_line(
                            text,
                            &fragment_style,
                            style.align,
                            width,
                            &bidi,
                            paragraph,
                            range,
                        )?);
                        start = break_at;
                    } else {
                        fitting = Some(break_at);
                    }
                } else if advance > width {
                    lines.push(self.visual_line(
                        text,
                        &fragment_style,
                        style.align,
                        width,
                        &bidi,
                        paragraph,
                        self.trimmed(start..break_at),
                    )?);
                    start = break_at;
                } else {
                    fitting = Some(break_at);
                }
            }
            if let Some(end) = fitting {
                lines.push(self.visual_line(
                    text,
                    &fragment_style,
                    style.align,
                    width,
                    &bidi,
                    paragraph,
                    self.trimmed(start..end),
                )?);
            }
        }
        if self.text.ends_with('\n') {
            lines.push(Line {
                pieces: Vec::new(),
                offset: 0.0,
                height: default_height,
                baseline: default_height,
            });
        }
        Ok(lines)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "visual line construction needs the native and bidi contexts"
    )]
    fn visual_line(
        &self,
        text: &mut TextSystem,
        style: &TextStyle,
        align: TextAlign,
        width: f64,
        bidi: &BidiInfo<'_>,
        paragraph: &ParagraphInfo,
        range: Range<usize>,
    ) -> Result<Line, Error> {
        let mut pieces = Vec::new();
        if !range.is_empty() {
            let (levels, visual_runs) = bidi.visual_runs(paragraph, range);
            for run in visual_runs {
                let rtl = levels.get(run.start).is_some_and(|level| level.is_rtl());
                let mut parts = self.pieces(text, style, width, run, Some(rtl))?;
                if rtl {
                    parts.reverse();
                }
                pieces.extend(parts);
            }
        }
        let used: f64 = pieces.iter().map(|piece| piece.size.width()).sum();
        let remaining = (width - used).max(0.0);
        let rtl = paragraph.level.is_rtl();
        let offset = match align {
            TextAlign::Center => remaining * 0.5,
            TextAlign::Start if rtl => remaining,
            TextAlign::End if !rtl => remaining,
            _ => 0.0,
        };
        let min_height = style.font_size * style.line_height;
        let (mut ascent, mut descent) = (0.0_f64, 0.0_f64);
        for part in &pieces {
            ascent = ascent.max(part.baseline);
            descent = descent.max(part.size.height() - part.baseline);
        }
        let extra = (min_height - ascent - descent).max(0.0);
        Ok(Line {
            pieces,
            offset,
            height: ascent + descent + extra,
            baseline: ascent + extra * 0.5,
        })
    }

    fn pieces(
        &self,
        text: &mut TextSystem,
        style: &TextStyle,
        width: f64,
        range: Range<usize>,
        visual_direction: Option<bool>,
    ) -> Result<Vec<Piece>, Error> {
        let mut pieces = Vec::new();
        let start = self
            .objects
            .partition_point(|object| object.range.end <= range.start);
        let mut cursor = range.start;
        for object in self.objects[start..]
            .iter()
            .take_while(|object| object.range.start < range.end)
        {
            if cursor < object.range.start {
                pieces.push(self.text_piece(
                    text,
                    style,
                    cursor..object.range.start,
                    visual_direction,
                    width,
                )?);
            }
            let em = if object.math { style.font_size } else { 1.0 };
            pieces.push(Piece::graphic(object.graphic.clone(), em, width));
            cursor = object.range.end;
        }
        if cursor < range.end {
            pieces.push(self.text_piece(
                text,
                style,
                cursor..range.end,
                visual_direction,
                width,
            )?);
        }
        Ok(pieces)
    }

    fn text_piece(
        &self,
        text: &mut TextSystem,
        style: &TextStyle,
        range: Range<usize>,
        direction: Option<bool>,
        width: f64,
    ) -> Result<Piece, Error> {
        let content = &self.text[range.clone()];
        // Preserve trailing whitespace before an inline object. NBSP has the
        // same advance as a space but retains it at the end of a shaped run.
        let preserve_spaces = content.ends_with(' ');
        let mut value = String::new();
        if let Some(rtl) = direction {
            value.push(if rtl { '\u{200f}' } else { '\u{200e}' });
        }
        let mut spans = Vec::new();
        let mut cursor = range.start;
        let start = self.styles.partition_point(|span| span.range.end <= cursor);
        let append = |target: &mut String, value: &str| {
            if preserve_spaces {
                target.extend(
                    value
                        .chars()
                        .map(|ch| if ch == ' ' { '\u{a0}' } else { ch }),
                );
            } else {
                target.push_str(value);
            }
        };
        for span in self.styles[start..]
            .iter()
            .take_while(|span| span.range.start < range.end)
        {
            let clipped = span.range.start.max(range.start)..span.range.end.min(range.end);
            append(&mut value, &self.text[cursor..clipped.start]);
            let start = value.len();
            append(&mut value, &self.text[clipped.clone()]);
            spans.push(TextSpan {
                range: start..value.len(),
                style: span.style.clone(),
            });
            cursor = clipped.end;
        }
        append(&mut value, &self.text[cursor..range.end]);

        let rich = RichText::new(value, spans)?;
        let layout = text.layout_rich(&rich, style, None)?;
        let layout = if direction.is_some() && layout.size().width() > width {
            layout.rewrap(Some(width))?
        } else {
            layout
        };
        Ok(Piece::text(layout))
    }

    fn trimmed(&self, range: Range<usize>) -> Range<usize> {
        let line = &self.text[range.clone()];
        let spaces = [' ', '\t', '\r', '\n'];
        let stripped = line.trim_start_matches(spaces);
        let start = range.end - stripped.len();
        start..start + stripped.trim_end_matches(spaces).len()
    }
}

fn style_for_marks(marks: Marks) -> TextSpanStyle {
    let color = if marks.link {
        Some(Color::from_rgba8(27, 91, 140, 255))
    } else if marks.code {
        Some(Color::from_rgba8(127, 57, 60, 255))
    } else {
        None
    };
    TextSpanStyle {
        family: marks.code.then(|| "monospace".into()),
        weight: marks.strong.then_some(700),
        color,
        italic: marks.italic,
        underline: marks.link,
        strikethrough: marks.strike,
        ..TextSpanStyle::default()
    }
}
