//! Filesystem change notifications shared by native desktop consumers.

use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{AccessKind, AccessMode},
};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Keeps one file under observation, including when a writer replaces it by renaming.
/// Drop the handle to stop observing. The callback executes on the watcher thread.
pub struct FileWatch {
    _watcher: RecommendedWatcher,
    target: PathBuf,
}

impl FileWatch {
    /// Watches the parent directory before the consumer's first read. A notification means
    /// the consumer should inspect the current file state. Duplicate notifications can occur.
    pub fn start(
        path: &Path,
        mut changed: impl FnMut(io::Result<()>) + Send + 'static,
    ) -> io::Result<Self> {
        let absolute = std::path::absolute(path)?;
        let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a file path",
            ));
        };
        let parent = parent.canonicalize()?;
        let target = parent.join(name);
        let observed = target.clone();
        let mut watcher = RecommendedWatcher::new(
            move |event: notify::Result<Event>| match event {
                Ok(event)
                    if event.need_rescan()
                        || (matches!(
                            event.kind,
                            EventKind::Any
                                | EventKind::Create(_)
                                | EventKind::Modify(_)
                                | EventKind::Remove(_)
                                | EventKind::Access(AccessKind::Close(AccessMode::Write))
                        ) && event.paths.iter().any(|p| p == &observed)) =>
                {
                    changed(Ok(()))
                }
                Err(error) => changed(Err(io::Error::other(error.to_string()))),
                _ => {}
            },
            Config::default(),
        )
        .map_err(io::Error::other)?;
        watcher
            .watch(&parent, RecursiveMode::NonRecursive)
            .map_err(io::Error::other)?;
        Ok(Self {
            _watcher: watcher,
            target,
        })
    }
    /// Observed file path with its parent directory canonicalized.
    pub fn path(&self) -> &Path {
        &self.target
    }
}
