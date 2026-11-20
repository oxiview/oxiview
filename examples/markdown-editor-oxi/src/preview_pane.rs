//! Compiles committed revisions on a worker and paints the latest result.

use crate::{
    app::error,
    preview::{CompileStats, Compiler, PreviewDocument, PreviewLayout},
    view::Ui,
};
use oxiview_base::{Point, Rect};
use oxiview_core::{Error, UiTree};
use oxiview_platform::WindowWaker;
use std::{
    io,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

struct Job {
    source: String,
    directory: PathBuf,
    path: Option<PathBuf>,
    serial: u64,
    waker: WindowWaker,
}

pub(crate) struct PreviewPane {
    jobs: Sender<Job>,
    completed: Receiver<(u64, PreviewDocument, CompileStats)>,
    pub(crate) submitted: u64,
    pub(crate) published: u64,
    pub(crate) stats: CompileStats,
    document: Option<PreviewDocument>,
    layout: Option<PreviewLayout>,
    width: f64,
    scroll: f64,
    painted: Option<(u64, Rect, f64, f64)>,
}

impl PreviewPane {
    pub(crate) fn new() -> io::Result<Self> {
        let (jobs, pending) = mpsc::channel::<Job>();
        let (send, completed) = mpsc::channel();
        thread::Builder::new()
            .name("markdown-preview".into())
            .spawn(move || {
                let mut compiler = Compiler::default();
                while let Ok(mut job) = pending.recv() {
                    for next in pending.try_iter() {
                        job = next
                    }
                    let doc =
                        compiler.compile_named(&job.source, &job.directory, job.path.as_deref());
                    if send.send((job.serial, doc, compiler.stats())).is_err() {
                        break;
                    }
                    let _ = job.waker.wake();
                }
            })?;
        Ok(Self {
            jobs,
            completed,
            submitted: 0,
            published: 0,
            stats: CompileStats::default(),
            document: None,
            layout: None,
            width: 0.,
            scroll: 0.,
            painted: None,
        })
    }

    pub(crate) fn submit(
        &mut self,
        source: String,
        directory: PathBuf,
        path: Option<PathBuf>,
        waker: &WindowWaker,
    ) -> Result<(), Error> {
        self.submitted = self
            .submitted
            .checked_add(1)
            .ok_or_else(|| error("Preview revisions exhausted"))?;
        self.jobs
            .send(Job {
                source,
                directory,
                path,
                serial: self.submitted,
                waker: waker.clone(),
            })
            .map_err(error)
    }

    pub(crate) fn poll(&mut self) -> bool {
        let mut dirty = false;
        for (revision, doc, stats) in self.completed.try_iter() {
            if revision == self.submitted {
                self.published = revision;
                self.document = Some(doc);
                self.layout = None;
                self.stats = stats;
                dirty = true;
            }
        }
        dirty
    }
    pub(crate) fn forget(&mut self) {
        self.document = None;
        self.layout = None;
        self.scroll = 0.;
        self.painted = None;
    }
    pub(crate) fn diagnostics(&self) -> usize {
        self.diagnostic_details().len()
    }
    pub(crate) fn diagnostic_details(&self) -> &[crate::preview::Diagnostic] {
        self.document
            .as_ref()
            .map_or(&[], PreviewDocument::diagnostics)
    }
    pub(crate) fn scroll(&mut self, delta: f64, height: f64) -> bool {
        let max = self
            .layout
            .as_ref()
            .map_or(0., |l| (l.height() - height).max(0.));
        let next = (self.scroll + delta).clamp(0., max);
        let changed = next != self.scroll;
        self.scroll = next;
        changed
    }
    pub(crate) fn update(&mut self, tree: &mut UiTree, ui: &Ui, scale: f64) -> Result<(), Error> {
        let id = ui.slots.preview.get()?;
        let bounds = tree.bounds(id)?;
        let size = bounds.size();
        let width = size.width();
        if width < 1. || size.height() < 1. {
            return Ok(());
        }
        let viewport = Rect::from_origin_size(Point::new(0., 0.), size);
        let mut system = ui.text.borrow_mut();
        if let Some(doc) = self.document.as_ref()
            && (self.layout.is_none() || self.width != width)
        {
            let layout = doc.layout(&mut system, width).map_err(error)?;
            self.scroll = self
                .scroll
                .clamp(0., (layout.height() - size.height()).max(0.));
            self.layout = Some(layout);
            self.width = width;
            self.painted = None;
        }
        let signature = (self.published, bounds, scale, self.scroll);
        if self.painted == Some(signature) {
            return Ok(());
        }
        let primitives = match self.layout.as_ref() {
            Some(layout) => layout
                .primitives_in_scene(&mut system, viewport, self.scroll, scale, bounds.origin())
                .map_err(error)?,
            None => Vec::new(),
        };
        tree.set_content(id, primitives)?;
        self.painted = Some(signature);
        Ok(())
    }
}
