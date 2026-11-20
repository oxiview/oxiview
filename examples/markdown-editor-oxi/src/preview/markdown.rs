use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Cursor, Read},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use image::{ImageReader, Limits};
use oxiview_base::{PixelSize, RgbaImage, Size};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};

use super::{
    Block, BlockKind, CellAlignment, Compiler, Diagnostic, Graphic, GraphicFormat, Inline,
    InlineKind, Link, ListItem, Marks, PreviewDocument, Table,
};

const MAX_MARKDOWN_NESTING: usize = 64;
const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;

pub(super) fn compile(
    compiler: &mut Compiler,
    source: &str,
    base_dir: &Path,
    file: Option<&Path>,
) -> PreviewDocument {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_MATH;
    let events = &mut Parser::new_ext(source, options).into_offset_iter();
    let nodes = read_nodes(events, source, 0);
    let mut markdown = Markdown {
        compiler,
        source,
        base_dir,
        file,
        diagnostics: Vec::new(),
        line_starts: None,
        links: Vec::new(),
        images: HashMap::new(),
    };
    let blocks = markdown.blocks(&nodes);
    PreviewDocument {
        blocks: blocks.into(),
        diagnostics: markdown.diagnostics.into(),
        links: markdown.links.into(),
    }
}

struct Node<'a> {
    event: Event<'a>,
    range: Range<usize>,
    children: Vec<Node<'a>>,
}

fn read_nodes<'a>(
    events: &mut impl Iterator<Item = (Event<'a>, Range<usize>)>,
    source: &'a str,
    depth: usize,
) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    while let Some((mut event, range)) = events.next() {
        let children = match event {
            Event::End(_) => break,
            Event::Start(_) if depth < MAX_MARKDOWN_NESTING => {
                read_nodes(events, source, depth + 1)
            }
            Event::Start(_) => {
                // Bound the recursion for pathological nesting while keeping the
                // original text visible and leaving the rest of the document intact.
                let mut nesting = 1usize;
                for (nested, _) in events.by_ref() {
                    match nested {
                        Event::Start(_) => nesting += 1,
                        Event::End(_) => nesting -= 1,
                        _ => {}
                    }
                    if nesting == 0 {
                        break;
                    }
                }
                event = Event::Text(source[range.clone()].into());
                Vec::new()
            }
            _ => Vec::new(),
        };
        nodes.push(Node {
            event,
            range,
            children,
        });
    }
    nodes
}

struct Markdown<'a, 'c> {
    compiler: &'c mut Compiler,
    source: &'a str,
    base_dir: &'a Path,
    file: Option<&'a Path>,
    diagnostics: Vec<Diagnostic>,
    line_starts: Option<Vec<usize>>,
    links: Vec<Link>,
    images: HashMap<PathBuf, Option<Arc<Graphic>>>,
}

impl Markdown<'_, '_> {
    fn blocks(&mut self, mut nodes: &[Node<'_>]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut loose = Vec::new();
        while let Some((node, next)) = nodes.split_first() {
            let kind = match &node.event {
                Event::Start(Tag::Paragraph) => {
                    self.flush_loose(&mut blocks, &mut loose);
                    let inlines = self.inlines(&node.children);
                    push_paragraphs(&mut blocks, inlines, Some(node.range.clone()));
                    nodes = next;
                    continue;
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    BlockKind::Heading(*level as u8, self.inlines(&node.children))
                }
                Event::Start(Tag::BlockQuote(_)) => BlockKind::Quote(self.blocks(&node.children)),
                Event::Start(Tag::List(start)) => {
                    BlockKind::List(*start, self.list(&node.children))
                }
                Event::Start(Tag::Table(alignments)) => {
                    BlockKind::Table(self.table(alignments, &node.children))
                }
                Event::Start(Tag::CodeBlock(info)) => self.code(info, node),
                Event::Start(Tag::HtmlBlock) => {
                    BlockKind::Code(self.source[node.range.clone()].trim_end().to_owned())
                }
                Event::Rule => BlockKind::Rule,
                _ => {
                    nodes = self.inline_run(nodes, Marks::default(), &mut loose);
                    continue;
                }
            };
            self.flush_loose(&mut blocks, &mut loose);
            blocks.push(Block {
                range: node.range.clone(),
                kind,
            });
            nodes = next;
        }
        self.flush_loose(&mut blocks, &mut loose);
        blocks
    }

    fn flush_loose(&self, blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
        push_paragraphs(blocks, std::mem::take(loose), None);
    }

    fn inlines(&mut self, nodes: &[Node<'_>]) -> Vec<Inline> {
        let mut inlines = Vec::new();
        self.append_inlines(nodes, Marks::default(), &mut inlines);
        inlines
    }

    fn append_inlines(&mut self, mut nodes: &[Node<'_>], marks: Marks, output: &mut Vec<Inline>) {
        while !nodes.is_empty() {
            nodes = self.inline_run(nodes, marks, output);
        }
    }

    fn inline_run<'n, 's>(
        &mut self,
        nodes: &'n [Node<'s>],
        marks: Marks,
        output: &mut Vec<Inline>,
    ) -> &'n [Node<'s>] {
        let Some((first, rest)) = nodes.split_first() else {
            return nodes;
        };
        if let Some(draft) = unbalanced_math(self.source, nodes) {
            let kind = self.formula(&self.source[draft.body], draft.range.clone(), draft.display);
            output.push(Inline {
                range: draft.range,
                marks,
                kind,
            });
            &nodes[draft.events..]
        } else {
            self.inline(first, marks, output);
            rest
        }
    }

    fn inline(&mut self, node: &Node<'_>, mut marks: Marks, output: &mut Vec<Inline>) {
        let range = node.range.clone();
        let kind = match &node.event {
            Event::Start(Tag::Emphasis) => {
                marks.italic = true;
                self.inline_children(node, marks, output);
                return;
            }
            Event::Start(Tag::Strong) => {
                marks.strong = true;
                self.inline_children(node, marks, output);
                return;
            }
            Event::Start(Tag::Strikethrough) => {
                marks.strike = true;
                self.inline_children(node, marks, output);
                return;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                self.links.push(Link {
                    destination: dest_url.to_string(),
                    range,
                });
                marks.link = true;
                self.inline_children(node, marks, output);
                return;
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if let Some(graphic) = self.image(dest_url) {
                    InlineKind::Graphic {
                        graphic,
                        math: false,
                    }
                } else {
                    let mut alt = String::new();
                    for child in &node.children {
                        image_alt(child, &mut alt);
                    }
                    InlineKind::Text(alt)
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                InlineKind::Text(text.to_string())
            }
            Event::Code(code) => {
                marks.code = true;
                InlineKind::Text(code.to_string())
            }
            Event::SoftBreak => InlineKind::Text(" ".into()),
            Event::HardBreak => InlineKind::Text("\n".into()),
            Event::InlineMath(source) => self.formula(source, range.clone(), false),
            Event::DisplayMath(source) => self.formula(source, range.clone(), true),
            Event::TaskListMarker(_) => return,
            _ => {
                self.inline_children(node, marks, output);
                return;
            }
        };
        output.push(Inline { range, marks, kind });
    }

    fn inline_children(&mut self, node: &Node<'_>, marks: Marks, output: &mut Vec<Inline>) {
        self.append_inlines(&node.children, marks, output);
    }

    fn formula(&mut self, source: &str, range: Range<usize>, display: bool) -> InlineKind {
        let format = if display {
            GraphicFormat::DisplayMath
        } else {
            GraphicFormat::InlineMath
        };
        match self.compiler.graphic(format, source) {
            Ok(graphic) if display => InlineKind::DisplayMath(graphic),
            Ok(graphic) => InlineKind::Graphic {
                graphic,
                math: true,
            },
            Err(error) => InlineKind::Diagnostic {
                index: self.diagnostic(range, format, &error.message),
                display,
            },
        }
    }

    fn list(&mut self, nodes: &[Node<'_>]) -> Vec<ListItem> {
        nodes
            .iter()
            .filter(|node| matches!(node.event, Event::Start(Tag::Item)))
            .map(|item| {
                let first = item.children.first().and_then(|node| {
                    if matches!(node.event, Event::Start(Tag::Paragraph)) {
                        node.children.first()
                    } else {
                        Some(node)
                    }
                });
                let task = first.and_then(|node| match node.event {
                    Event::TaskListMarker(checked) => Some(checked),
                    _ => None,
                });
                ListItem {
                    task,
                    blocks: self.blocks(&item.children),
                }
            })
            .collect()
    }

    fn table(&mut self, alignments: &[Alignment], nodes: &[Node<'_>]) -> Table {
        let alignments = alignments
            .iter()
            .map(|alignment| match alignment {
                Alignment::None | Alignment::Left => CellAlignment::Start,
                Alignment::Center => CellAlignment::Center,
                Alignment::Right => CellAlignment::End,
            })
            .collect();
        let rows = nodes
            .iter()
            .filter(|row| matches!(row.event, Event::Start(Tag::TableHead | Tag::TableRow)))
            .map(|row| {
                row.children
                    .iter()
                    .filter(|cell| matches!(cell.event, Event::Start(Tag::TableCell)))
                    .map(|cell| self.inlines(&cell.children))
                    .collect()
            })
            .collect();
        Table { alignments, rows }
    }

    fn code(&mut self, info: &CodeBlockKind<'_>, node: &Node<'_>) -> BlockKind {
        let mut body = String::new();
        let mut chunks = Vec::new();
        for child in &node.children {
            if let Event::Text(text) = &child.event {
                let start = body.len();
                body.push_str(text);
                chunks.push((start..body.len(), child.range.clone()));
            }
        }
        let mermaid = matches!(info, CodeBlockKind::Fenced(info)
            if info.split_ascii_whitespace().next()
                .is_some_and(|name| name.eq_ignore_ascii_case("mermaid")));
        if !mermaid {
            if body.ends_with('\n') {
                body.pop();
            }
            return BlockKind::Code(body);
        }
        let format = GraphicFormat::Mermaid;
        match self.compiler.graphic(format, &body) {
            Ok(graphic) => BlockKind::Graphic {
                graphic,
                math: false,
            },
            Err(error) => {
                let range = error
                    .location
                    .and_then(|(line, col)| source_location(self.source, &body, &chunks, line, col))
                    .unwrap_or_else(|| node.range.clone());
                BlockKind::Diagnostic(self.diagnostic(range, format, &error.message))
            }
        }
    }

    fn diagnostic(&mut self, range: Range<usize>, format: GraphicFormat, message: &str) -> usize {
        let line_starts = self
            .line_starts
            .get_or_insert_with(|| line_starts(self.source));
        let index = line_starts
            .partition_point(|&start| start <= range.start)
            .saturating_sub(1);
        let column = self.source[line_starts[index]..range.start].chars().count() + 1;
        let index_in_document = self.diagnostics.len();
        self.diagnostics.push(Diagnostic {
            file: self.file.map(Path::to_owned),
            range,
            line: index + 1,
            column,
            message: format!("{}: {message}", format.label()),
        });
        index_in_document
    }

    fn image(&mut self, destination: &str) -> Option<Arc<Graphic>> {
        let path = local_image_path(self.base_dir, destination)?;
        self.images
            .entry(path.clone())
            .or_insert_with(|| read_image(&path))
            .clone()
    }
}

struct MathDraft {
    range: Range<usize>,
    body: Range<usize>,
    events: usize,
    display: bool,
}

// Pulldown emits literal text for paired dollar delimiters when brace scopes
// differ. Recover that edit as a formula so the typesetter can report its error.
// Code, parsed math, and link/image boundaries separate candidate spans.
fn unbalanced_math(source: &str, nodes: &[Node<'_>]) -> Option<MathDraft> {
    let open = nodes.first()?.range.start;
    let count = dollar_run(source, nodes)?;
    let body_start = open + count;
    if count == 1 && source.as_bytes().get(body_start)?.is_ascii_whitespace() {
        return None;
    }
    let mut close = count;
    while let Some(node) = nodes.get(close) {
        if node.range.start.saturating_sub(body_start) > super::math::MAX_SOURCE_BYTES {
            break;
        }
        if let Some(closing_count) = dollar_run(source, &nodes[close..]) {
            let body = body_start..node.range.start;
            let expression = source.get(body.clone())?;
            if closing_count == count
                && (count == 2 || !expression.ends_with(char::is_whitespace))
                && has_unbalanced_braces(expression)
            {
                return Some(MathDraft {
                    range: open..node.range.start + count,
                    body,
                    events: close + count,
                    display: count == 2,
                });
            }
            break;
        }
        if !matches!(
            node.event,
            Event::Text(_)
                | Event::SoftBreak
                | Event::HardBreak
                | Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
        ) {
            break;
        }
        close += 1;
    }
    None
}

fn dollar_run(source: &str, nodes: &[Node<'_>]) -> Option<usize> {
    let node = nodes.first()?;
    let start = node.range.start;
    if !matches!(&node.event, Event::Text(text) if text.as_ref() == "$")
        || source.get(node.range.clone()) != Some("$")
        || source.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\\')
            .count()
            % 2
            != 0
    {
        return None;
    }
    let paired = nodes.get(1).is_some_and(|next| {
        next.range.start == start + 1
            && matches!(&next.event, Event::Text(text) if text.as_ref() == "$")
    });
    Some(1 + usize::from(paired))
}

fn has_unbalanced_braces(source: &str) -> bool {
    let mut depth = 0;
    let mut escaped = false;
    for character in source.chars() {
        if std::mem::take(&mut escaped) {
            continue;
        }
        match character {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' if depth == 0 => return true,
            '}' => depth -= 1,
            _ => {}
        }
    }
    depth != 0
}

fn push_paragraphs(blocks: &mut Vec<Block>, inlines: Vec<Inline>, range: Option<Range<usize>>) {
    let mut paragraph = Vec::new();
    let mut split = false;
    for inline in inlines {
        let kind = match inline.kind {
            InlineKind::DisplayMath(graphic) => BlockKind::Graphic {
                graphic,
                math: true,
            },
            InlineKind::Diagnostic {
                index,
                display: true,
            } => BlockKind::Diagnostic(index),
            _ => {
                paragraph.push(inline);
                continue;
            }
        };
        push_paragraph(blocks, std::mem::take(&mut paragraph), None);
        blocks.push(Block {
            range: inline.range,
            kind,
        });
        split = true;
    }
    push_paragraph(blocks, paragraph, range.filter(|_| !split));
}

fn push_paragraph(blocks: &mut Vec<Block>, inlines: Vec<Inline>, range: Option<Range<usize>>) {
    if let Some((first, last)) = inlines.first().zip(inlines.last()) {
        blocks.push(Block {
            range: range.unwrap_or(first.range.start..last.range.end),
            kind: BlockKind::Paragraph(inlines),
        });
    }
}

fn image_alt(node: &Node<'_>, output: &mut String) {
    match &node.event {
        Event::Text(text) | Event::Code(text) | Event::InlineMath(text) => output.push_str(text),
        Event::SoftBreak | Event::HardBreak => output.push(' '),
        _ => node
            .children
            .iter()
            .for_each(|child| image_alt(child, output)),
    }
}

fn local_image_path(base_dir: &Path, destination: &str) -> Option<PathBuf> {
    let raw = destination.split(['#', '?']).next()?;
    let mut decoded = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)? as u8;
            let low = char::from(bytes.next()?).to_digit(16)? as u8;
            decoded.push((high << 4) | low);
        } else {
            decoded.push(byte);
        }
    }
    let path = std::str::from_utf8(&decoded).ok()?;
    if path.is_empty() || path.starts_with("//") || path.starts_with("\\\\") {
        return None;
    }
    let file = Path::new(path);
    if !file.is_absolute() && path.split(['/', '\\']).next()?.contains(':') {
        return None;
    }
    Some(base_dir.join(file))
}

fn read_image(path: &Path) -> Option<Arc<Graphic>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return None;
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(48 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let image = RgbaImage::new(PixelSize { width, height }, rgba.into_raw()).ok()?;
    let size = Size::new(f64::from(width), f64::from(height));
    Some(Arc::new(Graphic::raster(image, size)))
}

fn line_starts(source: &str) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut starts = vec![0];
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n')) {
            starts.push(index + 1);
        }
    }
    starts
}

fn source_location(
    source: &str,
    body: &str,
    chunks: &[(Range<usize>, Range<usize>)],
    line: usize,
    column: usize,
) -> Option<Range<usize>> {
    let body_lines = line_starts(body);
    let body_start = *body_lines.get(line.checked_sub(1)?)?;
    let (body_chunk, source_chunk) = chunks
        .iter()
        .find(|(chunk, _)| chunk.contains(&body_start))?;
    let preceding = &body[body_chunk.start..body_start];
    let inner_line = line_starts(preceding).len() - 1;
    let original = &source[source_chunk.clone()];
    let source_lines = line_starts(original);
    let source_start = source_chunk.start + *source_lines.get(inner_line)?;
    let source_end = source_lines
        .get(inner_line + 1)
        .map_or(source_chunk.end, |relative| source_chunk.start + relative);
    let original_line = source[source_start..source_end].trim_end_matches(['\r', '\n']);
    let index = column.saturating_sub(1);
    let offset = original_line
        .char_indices()
        .nth(index)
        .map_or(original_line.len(), |(offset, _)| offset);
    let length = original_line[offset..]
        .chars()
        .next()
        .map_or(0, char::len_utf8);
    Some(source_start + offset..source_start + offset + length)
}
