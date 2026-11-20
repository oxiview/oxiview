//! Release acceptance through the real window loop, headless backend and CPU presenter.

mod scenario;
mod window;

use crate::app::{EditorApp, fixture_dir};
use oxiview_base::RgbaImage;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Evidence collected through the native editor's normal window event paths.
#[derive(Debug, Serialize)]
pub struct Report {
    /// Directory reserved for this run.
    pub output: PathBuf,
    /// Checks which finished successfully.
    pub checks: Vec<&'static str>,
    /// CPU presentations written relative to the run directory.
    pub images: Vec<String>,
    /// Saved and reopened file relative to the run directory.
    pub roundtrip: String,
    /// Additional presentations while the window remained inactive.
    pub idle_presentations: u64,
    /// Virtual display opportunities observed while the window remained inactive.
    pub idle_opportunities: usize,
    /// Number of preview parses caused by marked-text updates.
    pub preedit_parses: u64,
    /// Number of preview parses caused by the composition commit.
    pub commit_parses: u64,
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
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    loop {
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            "replay-{timestamp}-{}-{serial}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

/// Runs one isolated editor session and writes its manifest after clean shutdown.
pub fn run(output: &Path) -> Result<Report> {
    let output = reserve_output(output)?;
    let bytes = fs::read(fixture_dir().join("hill.png"))?;
    new_file(&output.join("hill.png"))?.write_all(&bytes)?;
    let report = window::with_window(
        || EditorApp::new(None),
        |window| scenario::run(window, &output),
    )?;
    let mut manifest = BufWriter::new(new_file(&output.join("replay.json"))?);
    serde_json::to_writer_pretty(&mut manifest, &report)?;
    manifest.flush()?;
    Ok(report)
}
