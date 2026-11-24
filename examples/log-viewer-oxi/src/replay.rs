//! Release acceptance through the viewer's normal event loop and CPU presenter.

mod collection;
mod files;
mod window;

use oxiview_base::RgbaImage;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// An application, driver, verification or filesystem failure.
pub type ReplayError = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, ReplayError>;

/// Evidence from a completed headless viewer session.
#[derive(Debug, Serialize)]
pub struct ReplayReport {
    /// Unique artifact directory beneath the caller's requested directory.
    pub output: PathBuf,
    /// Behaviors verified through the application's input and output ports.
    pub checks: Vec<String>,
    /// CPU presentations saved relative to the run directory.
    pub images: Vec<String>,
    /// Virtual display opportunities advanced with the settled viewer inactive.
    pub idle_opportunities: usize,
    /// Frames submitted during those opportunities.
    pub idle_submissions: u64,
    /// CPU presentations completed during those opportunities.
    pub idle_presentations: u64,
}

fn check(condition: bool, failure: impl FnOnce() -> String) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(failure().into())
    }
}

fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn write_png(path: &Path, image: &RgbaImage) -> Result<()> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(new_file(path)?),
        image.width(),
        image.height(),
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(image.pixels())?;
    writer.finish()?;
    Ok(())
}

fn reserve_output(parent: &Path) -> Result<PathBuf> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(parent)?;
    let parent = parent.canonicalize()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    loop {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("replay-{stamp}-{}-{serial}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

/// Drives the production viewer on the virtual platform and writes the replay evidence.
pub fn run(output: &Path) -> Result<ReplayReport> {
    let output = reserve_output(output)?;
    let mut report = ReplayReport {
        output: output.clone(),
        checks: Vec::new(),
        images: Vec::new(),
        idle_opportunities: 1_000,
        idle_submissions: 0,
        idle_presentations: 0,
    };
    let fixture = files::Fixture::new(&output)?;
    window::with_window(|window| {
        collection::run(window, &mut report)?;
        files::run(window, &fixture, &mut report)?;
        (report.idle_submissions, report.idle_presentations) =
            window.idle(report.idle_opportunities)?;
        check(
            (report.idle_submissions, report.idle_presentations) == (0, 0),
            || {
                format!(
                    "Idle viewer submitted {} frames and presented {} times",
                    report.idle_submissions, report.idle_presentations,
                )
            },
        )?;
        report.passed("idle-1000-opportunities");
        Ok(())
    })
    .map_err(|error| format!("Replay failed in {}: {error}", output.display()))?;
    fixture.remove()?;
    let mut manifest = BufWriter::new(new_file(&output.join("replay.json"))?);
    serde_json::to_writer_pretty(&mut manifest, &report)?;
    manifest.flush()?;
    Ok(report)
}

impl ReplayReport {
    fn passed(&mut self, check: &str) {
        self.checks.push(check.into());
    }

    fn capture(&mut self, window: &window::Window, name: &str) -> Result<()> {
        let image = window.capture()?;
        write_png(&self.output.join(name), &image)?;
        self.images.push(name.into());
        Ok(())
    }
}
