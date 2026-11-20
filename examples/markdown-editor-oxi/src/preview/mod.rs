//! Owned Markdown preview compilation and native viewport painting.

mod diagram;
mod layout;
mod markdown;
mod math;

use oxiview_base::{Color, RgbaImage, Size};
use oxiview_scene::Primitive;
use std::{
    collections::HashMap,
    fmt,
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

pub use layout::PreviewLayout;

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;

fn ink() -> Color {
    Color::from_rgba8(34, 43, 47, 255)
}

/// Cumulative work performed by one preview worker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompileStats {
    /// Sources parsed into preview documents.
    pub parses: u64,
    /// Mermaid blocks compiled on a cache miss.
    pub diagram_renders: u64,
    /// Inline and display LaTeX expressions compiled on a cache miss.
    pub math_renders: u64,
    /// Reused graphic results, including cached diagnostics.
    pub cache_hits: u64,
}

/// A preview error attached to committed Markdown source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Path of the document, if it has been named.
    pub file: Option<PathBuf>,
    /// UTF-8 byte range in the Markdown source.
    pub range: Range<usize>,
    /// One-based source line.
    pub line: usize,
    /// One-based source character column.
    pub column: usize,
    /// Description returned by the compiler.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self
            .file
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Untitled".into(), |name| name.to_string_lossy());
        write!(
            out,
            "{name}:{}:{}  {}",
            self.line, self.column, self.message
        )
    }
}

/// Worker-owned compiler with a bounded cache of Mermaid and LaTeX results.
#[derive(Default)]
pub struct Compiler {
    cache: HashMap<(GraphicFormat, String), CacheEntry>,
    cache_bytes: usize,
    clock: u64,
    stats: CompileStats,
}

impl Compiler {
    /// Parses a source without a filename and resolves images from `base_dir`.
    pub fn compile(&mut self, source: &str, base_dir: &Path) -> PreviewDocument {
        self.compile_named(source, base_dir, None)
    }

    /// Parses committed source, resolving relative images and naming diagnostics.
    pub fn compile_named(
        &mut self,
        source: &str,
        base_dir: &Path,
        file: Option<&Path>,
    ) -> PreviewDocument {
        self.stats.parses = self.stats.parses.saturating_add(1);
        markdown::compile(self, source, base_dir, file)
    }

    /// Returns this worker's cumulative preview compilation counters.
    pub fn stats(&self) -> CompileStats {
        self.stats
    }

    fn graphic(&mut self, format: GraphicFormat, source: &str) -> GraphicResult {
        let key = (format, source.to_owned());
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.cache.get_mut(&key) {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
            entry.last_used = self.clock;
            return entry.result.clone();
        }

        let result = match format {
            GraphicFormat::Mermaid => {
                self.stats.diagram_renders = self.stats.diagram_renders.saturating_add(1);
                diagram::render(source)
            }
            GraphicFormat::InlineMath | GraphicFormat::DisplayMath => {
                self.stats.math_renders = self.stats.math_renders.saturating_add(1);
                math::render(source, format == GraphicFormat::DisplayMath)
            }
        }
        .map(Arc::new)
        .map_err(Arc::new);
        let bytes = source.len()
            + match &result {
                Ok(graphic) => graphic.cache_bytes(),
                Err(failure) => failure.message.len(),
            };
        if bytes <= CACHE_BYTES {
            while self.cache.len() >= CACHE_ENTRIES || self.cache_bytes + bytes > CACHE_BYTES {
                let Some((key, _)) = self.cache.iter().min_by_key(|(_, entry)| entry.last_used)
                else {
                    break;
                };
                let key = key.clone();
                if let Some(entry) = self.cache.remove(&key) {
                    self.cache_bytes -= entry.bytes;
                }
            }
            self.cache_bytes += bytes;
            self.cache.insert(
                key,
                CacheEntry {
                    result: result.clone(),
                    bytes,
                    last_used: self.clock,
                },
            );
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GraphicFormat {
    Mermaid,
    InlineMath,
    DisplayMath,
}

impl GraphicFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Mermaid => "Mermaid",
            Self::InlineMath | Self::DisplayMath => "Math",
        }
    }
}

struct CacheEntry {
    result: GraphicResult,
    bytes: usize,
    last_used: u64,
}

type GraphicResult = Result<Arc<Graphic>, Arc<GraphicError>>;

#[derive(Debug)]
struct GraphicError {
    message: String,
    location: Option<(usize, usize)>,
}

impl GraphicError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
        }
    }
}

#[derive(Debug)]
struct Graphic {
    size: Size,
    baseline: f64,
    content: GraphicContent,
}

impl Graphic {
    fn raster(image: RgbaImage, size: Size) -> Self {
        Self {
            size,
            baseline: size.height(),
            content: GraphicContent::Raster(image),
        }
    }

    fn cache_bytes(&self) -> usize {
        match &self.content {
            GraphicContent::Raster(image) => image.pixels().len(),
            GraphicContent::Math(primitives) => {
                let paths = primitives.iter().filter_map(|primitive| match primitive {
                    Primitive::FillPath(path, ..) => Some(size_of_val(path.commands())),
                    _ => None,
                });
                size_of_val(primitives.as_slice()) + paths.sum::<usize>()
            }
        }
    }
}

#[derive(Debug)]
enum GraphicContent {
    Raster(RgbaImage),
    Math(Vec<Primitive>),
}

/// A shareable preview of one committed source revision.
#[derive(Clone, Debug)]
pub struct PreviewDocument {
    blocks: Arc<[Block]>,
    diagnostics: Arc<[Diagnostic]>,
    links: Arc<[Link]>,
}

impl PreviewDocument {
    /// Diagnostics found in this revision, in document order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Link destinations and their committed source ranges in document order.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Top-level block ranges in committed source order.
    pub fn block_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.blocks.iter().map(|block| block.range.clone())
    }
}

/// A CommonMark link and its originating committed byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The link destination, without performing I/O.
    pub destination: String,
    /// UTF-8 byte range of the link in the Markdown source.
    pub range: Range<usize>,
}

#[derive(Debug)]
struct Block {
    range: Range<usize>,
    kind: BlockKind,
}

#[derive(Debug)]
enum BlockKind {
    Paragraph(Vec<Inline>),
    Heading(u8, Vec<Inline>),
    Code(String),
    Quote(Vec<Block>),
    List(Option<u64>, Vec<ListItem>),
    Table(Table),
    Rule,
    Graphic { graphic: Arc<Graphic>, math: bool },
    Diagnostic(usize),
}

#[derive(Debug)]
struct ListItem {
    task: Option<bool>,
    blocks: Vec<Block>,
}

#[derive(Debug, Default)]
struct Table {
    alignments: Vec<CellAlignment>,
    rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Clone, Copy, Debug)]
enum CellAlignment {
    Start,
    Center,
    End,
}

#[derive(Debug)]
struct Inline {
    range: Range<usize>,
    marks: Marks,
    kind: InlineKind,
}

#[derive(Clone, Copy, Debug, Default)]
struct Marks {
    strong: bool,
    italic: bool,
    code: bool,
    link: bool,
    strike: bool,
}

#[derive(Debug)]
enum InlineKind {
    Text(String),
    Graphic { graphic: Arc<Graphic>, math: bool },
    DisplayMath(Arc<Graphic>),
    Diagnostic { index: usize, display: bool },
}

#[cfg(test)]
mod tests;
