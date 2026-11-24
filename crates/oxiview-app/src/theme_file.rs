use crate::FileWatch;
use oxiview_core::{UiError, UiTree};
use oxiview_platform::WindowWaker;
use oxiview_style::{Theme, ThemeError};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

const SAVE_SETTLE_TIME: Duration = Duration::from_millis(30);

/// Failure to watch, load, or install a development theme.
#[derive(Debug, thiserror::Error)]
pub enum ThemeReloadError {
    /// The file watcher could not be started or reported an error.
    #[error("theme watch {}: {message}", path.display())]
    Watch {
        /// Watched theme file.
        path: PathBuf,
        /// Backend or worker diagnostic.
        message: String,
    },
    /// The source file could not be read as UTF-8.
    #[error("theme read {}: {source}", path.display())]
    Read {
        /// Watched theme file.
        path: PathBuf,
        /// Filesystem or encoding failure.
        source: io::Error,
    },
    /// A candidate contained invalid syntax, tokens, or declarations.
    #[error("theme {}: {source}", path.display())]
    Invalid {
        /// Watched theme file.
        path: PathBuf,
        /// Offending key and diagnostic.
        source: ThemeError,
    },
    /// The window could not install a validated theme.
    #[error("theme apply {}: {source}", path.display())]
    Apply {
        /// Watched theme file.
        path: PathBuf,
        /// Window-thread failure.
        source: UiError,
    },
}

/// An optional development theme source for one window.
///
/// The worker watches the file's parent directory, reads and validates on
/// notifications, and publishes only the latest candidate. The application calls
/// [`Self::apply_latest`] on the window thread when it receives a window wake.
pub struct ThemeFile {
    path: PathBuf,
    shared: Arc<Shared>,
}

impl ThemeFile {
    /// Starts a native file watch and queues an initial load on a worker thread.
    /// Work is triggered by file notifications, including save-by-rename.
    pub fn watch(path: impl AsRef<Path>, waker: &WindowWaker) -> Result<Self, ThemeReloadError> {
        let path = path.as_ref().to_owned();
        let shared = Arc::new(Shared::default());
        let worker_shared = Arc::clone(&shared);
        let worker_path = path.clone();
        let waker = waker.clone();
        thread::Builder::new()
            .name("oxiview-theme".into())
            .spawn(move || run_worker(&worker_path, &worker_shared, &waker))
            .map_err(|error| watch_error(&path, error))?;
        Ok(Self { path, shared })
    }

    /// Applies a ready revision in one window-thread update. `None` means that
    /// no file revision was ready. `Some(changed)` distinguishes a successful
    /// save with identical effective values from one that invalidated visual work.
    pub fn apply_latest(&self, tree: &mut UiTree) -> Result<Option<bool>, ThemeReloadError> {
        let mut inbox = self
            .shared
            .inbox
            .lock()
            .map_err(|error| watch_error(&self.path, error))?;
        // Serialize publication of another file revision with this commit. The
        // worker performs all file I/O and validation outside this lock.
        match inbox.ready.take() {
            Some(candidate) => {
                tree.set_theme(candidate?)
                    .map(Some)
                    .map_err(|source| ThemeReloadError::Apply {
                        path: self.path.clone(),
                        source,
                    })
            }
            None => Ok(None),
        }
    }
}

impl Drop for ThemeFile {
    fn drop(&mut self) {
        if let Ok(mut inbox) = self.shared.inbox.lock() {
            inbox.closed = true;
            inbox.ready = None;
            self.shared.changed.notify_one();
        }
    }
}

enum Request {
    Load,
    WatchError(ThemeReloadError),
}

type Candidate = Result<Theme, ThemeReloadError>;

struct Inbox {
    // Pointer identity distinguishes revisions without an incrementing counter.
    revision: Arc<()>,
    pending: Option<Request>,
    ready: Option<Candidate>,
    closed: bool,
}

impl Default for Inbox {
    fn default() -> Self {
        Self {
            revision: Arc::new(()),
            pending: Some(Request::Load),
            ready: None,
            closed: false,
        }
    }
}

impl Inbox {
    fn request(&mut self, request: Request) {
        self.revision = Arc::new(());
        self.pending = Some(request);
        self.ready = None;
    }

    fn finish(&mut self, revision: &Arc<()>, candidate: Candidate) -> bool {
        if self.closed || !Arc::ptr_eq(&self.revision, revision) {
            return false;
        }
        self.ready = Some(candidate);
        true
    }
}

#[derive(Default)]
struct Shared {
    inbox: Mutex<Inbox>,
    changed: Condvar,
}

impl Shared {
    fn request(&self, request: Request) {
        if let Ok(mut inbox) = self.inbox.lock()
            && !inbox.closed
        {
            inbox.request(request);
            self.changed.notify_one();
        }
    }

    fn next_request(&self) -> Option<(Arc<()>, Request)> {
        let mut inbox = self.inbox.lock().ok()?;
        loop {
            inbox = self
                .changed
                .wait_while(inbox, |inbox| !inbox.closed && inbox.pending.is_none())
                .ok()?;
            if inbox.closed {
                return None;
            }
            let revision = Arc::clone(&inbox.revision);
            (inbox, _) = self
                .changed
                .wait_timeout_while(inbox, SAVE_SETTLE_TIME, |inbox| {
                    !inbox.closed && Arc::ptr_eq(&inbox.revision, &revision)
                })
                .ok()?;
            if inbox.closed {
                return None;
            }
            if Arc::ptr_eq(&inbox.revision, &revision) {
                return inbox.pending.take().map(|request| (revision, request));
            }
        }
    }

    fn finish(&self, revision: &Arc<()>, candidate: Candidate, waker: &WindowWaker) -> bool {
        let Ok(mut inbox) = self.inbox.lock() else {
            return false;
        };
        if inbox.finish(revision, candidate) && waker.wake().is_err() {
            inbox.closed = true;
            inbox.ready = None;
        }
        !inbox.closed
    }
}

fn run_worker(path: &Path, shared: &Arc<Shared>, waker: &WindowWaker) {
    let watcher = match start_watch(path, shared) {
        Ok(watch) => watch,
        Err(error) => {
            if let Ok(mut inbox) = shared.inbox.lock()
                && !inbox.closed
            {
                inbox.ready = Some(Err(error));
                let _ = waker.wake();
            }
            return;
        }
    };
    while let Some((revision, request)) = shared.next_request() {
        let result = match request {
            Request::Load => load(watcher.path(), path),
            Request::WatchError(error) => Err(error),
        };
        if !shared.finish(&revision, result, waker) {
            break;
        }
    }
}

fn start_watch(path: &Path, shared: &Arc<Shared>) -> Result<FileWatch, ThemeReloadError> {
    let shared = Arc::clone(shared);
    let display_path = path.to_owned();
    FileWatch::start(path, move |event| match event {
        Ok(()) => shared.request(Request::Load),
        Err(error) => shared.request(Request::WatchError(watch_error(&display_path, error))),
    })
    .map_err(|error| watch_error(path, error))
}

fn load(target: &Path, path: &Path) -> Candidate {
    let result = fs::metadata(target).and_then(|metadata| {
        if metadata.is_file() {
            fs::read_to_string(target)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file",
            ))
        }
    });
    let source = result.map_err(|source| ThemeReloadError::Read {
        path: path.to_owned(),
        source,
    })?;
    Theme::from_toml(&source).map_err(|source| ThemeReloadError::Invalid {
        path: path.to_owned(),
        source,
    })
}

fn watch_error(path: &Path, message: impl std::fmt::Display) -> ThemeReloadError {
    ThemeReloadError::Watch {
        path: path.to_owned(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_started_work_cannot_publish_after_a_new_request_or_close() {
        let mut inbox = Inbox::default();
        let older = Arc::clone(&inbox.revision);
        inbox.request(Request::Load);
        let latest = Arc::clone(&inbox.revision);
        assert!(inbox.finish(&latest, Ok(Theme::default())));
        assert!(!inbox.finish(
            &older,
            Err(watch_error(Path::new("theme.toml"), "stale read"))
        ));
        assert!(matches!(inbox.ready.take(), Some(Ok(_))));
        inbox.closed = true;
        assert!(!inbox.finish(&latest, Ok(Theme::default())));
        assert!(inbox.ready.is_none());
    }
}
