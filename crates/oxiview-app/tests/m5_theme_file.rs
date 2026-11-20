//! Development theme reload through a real native file watcher and window wake.

use oxiview_app::{ThemeFile, ThemeReloadError};
use oxiview_base::{Color, Size};
use oxiview_core::{FocusPolicy, UiTree, node};
use oxiview_platform::{Error, WindowWake, WindowWaker};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};

const VIEWPORT: Size = Size::new(64.0, 32.0);
const TIMEOUT: Duration = Duration::from_secs(10);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> std::io::Result<Self> {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("theme-rename-{}", std::process::id()));
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn save(&self, source: &str) -> std::io::Result<()> {
        let temporary = self.0.join("editor-save.tmp");
        fs::write(&temporary, source)?;
        fs::rename(temporary, self.0.join("theme.toml"))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct WakeNotice(Sender<()>);

impl WindowWake for WakeNotice {
    fn wake(&self) -> Result<(), Error> {
        self.0.send(()).map_err(|_| Error::WindowDestroyed)
    }
}

fn document(color: &str) -> String {
    format!("version = 1\n[rules.panel]\nbackground = '{color}'")
}

fn await_revision(
    receive: &Receiver<()>,
    source: &ThemeFile,
    tree: &mut UiTree,
) -> Result<Result<bool, ThemeReloadError>, mpsc::RecvTimeoutError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        receive.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
        if let Some(applied) = source.apply_latest(tree).transpose() {
            return Ok(applied);
        }
    }
}

#[test]
fn save_by_rename_loads_and_recovers_after_invalid_edit() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TestDirectory::new()?;
    let path = directory.0.join("theme.toml");
    directory.save(&document("#9b0b16"))?;
    let (send, receive) = mpsc::channel();
    let waker = WindowWaker::new(Arc::new(WakeNotice(send)));
    let source = ThemeFile::watch(&path, &waker)?;
    let mut tree = UiTree::new()?;
    let id = tree.mount(
        tree.root(),
        node().theme_type("panel").focusable(FocusPolicy::Tab),
    )?;
    tree.focus(id)?;
    assert!(await_revision(&receive, &source, &mut tree)??);
    assert_eq!(
        tree.style(id)?.background,
        Color::from_rgba8(155, 11, 22, 255)
    );
    tree.paint(VIEWPORT, 1.0)?;

    directory.save(&document("@missing"))?;
    let rejected = await_revision(&receive, &source, &mut tree)?;
    assert!(
        matches!(rejected, Err(ThemeReloadError::Invalid { path: rejected_path, source })
        if rejected_path == path && source.key() == "rules.panel.background")
    );
    assert_eq!(
        tree.style(id)?.background,
        Color::from_rgba8(155, 11, 22, 255)
    );
    assert!(!tree.flush_updates(VIEWPORT, |_, _, _| {})?.changed);

    directory.save(&document("#9b0b16"))?;
    assert!(!await_revision(&receive, &source, &mut tree)??);

    directory.save(&document("#123456"))?;
    assert!(await_revision(&receive, &source, &mut tree)??);
    assert_eq!(
        tree.style(id)?.background,
        Color::from_rgba8(18, 52, 86, 255)
    );
    assert_eq!(tree.focused_node(), Some(id));

    drop(source);
    while receive.try_recv().is_ok() {}
    directory.save(&document("#654321"))?;
    assert_eq!(
        receive.recv_timeout(Duration::from_millis(250)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    Ok(())
}
