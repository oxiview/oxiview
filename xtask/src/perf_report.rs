//! Publish selected benchmark artifacts into the rolling bilingual report.

mod render;
#[cfg(test)]
mod tests;

use crate::{check::Check, report::Status};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const RUNS: &str = "docs/performance/runs";
const HISTORY: &str = "docs/performance/history.md";
const BEGIN: &str = "<!-- oxiview-perf:start -->";
const END: &str = "<!-- oxiview-perf:end -->";
const LIMIT: usize = 5;
type Result<T> = std::result::Result<T, String>;

#[derive(Deserialize)]
struct Meta {
    measured_at: u64,
    head: String,
    dirty: bool,
    implementation: Option<String>,
    label: [String; 2],
    machine: String,
    latency_decimals: Option<usize>,
    #[serde(default)]
    pointer_probes: Vec<Probe>,
    continuous_submit_ms: Option<f64>,
}

#[derive(Deserialize)]
struct Probe {
    visible: u64,
    inputs: u64,
    display_p95_ms: f64,
}

#[derive(Deserialize)]
struct Conditions {
    objects: u32,
    pixel_width: u32,
    pixel_height: u32,
    scale: f64,
    fullscreen: bool,
}

#[derive(Deserialize)]
struct Animation {
    conditions: Conditions,
    sample_seconds: f64,
    warmup_seconds: f64,
    displayed_fps: f64,
    display_gap_p95_ms: f64,
}

struct Native {
    submit: f64,
    display: f64,
    idle_cpu: f64,
    idle_seconds: f64,
    counts: [u64; 3],
    gpu_ms: [f64; 3],
}

#[derive(Deserialize)]
struct ScrollReport {
    rows: u64,
    samples: Vec<ScrollSample>,
}

#[derive(Deserialize)]
struct ScrollSample {
    occupied_slots: u64,
    expected_slots: u64,
    measured_duration_ns: u64,
    apple: Option<ScrollHitchTime>,
}

#[derive(Deserialize)]
struct ScrollHitchTime {
    observed_hitch_time_ns: u64,
    complete_apple_frame_lifetime_coverage: bool,
}

impl ScrollReport {
    fn read(data: &[u8]) -> Result<Self> {
        let report: Self = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        if report.rows != 100_000 {
            return Err("expected the 100,000-row M7 workload".into());
        }
        if report.samples.is_empty()
            || report.samples.iter().any(|s| {
                s.expected_slots == 0
                    || s.occupied_slots > s.expected_slots
                    || s.measured_duration_ns == 0
                    || s.apple
                        .as_ref()
                        .is_some_and(|h| h.observed_hitch_time_ns > s.measured_duration_ns)
            })
        {
            return Err("invalid M7 scroll sample counts or duration".into());
        }
        Ok(report)
    }
}

struct Run {
    meta: Meta,
    native: Option<Native>,
    animation: Vec<Animation>,
    scroll: Option<ScrollReport>,
    files: BTreeMap<String, Vec<u8>>,
}

fn hash(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn number(value: &Value, field: &str) -> Result<f64> {
    value[field]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0)
        .ok_or_else(|| format!("missing or invalid {field}"))
}

impl Native {
    fn read(data: &[u8]) -> Result<Self> {
        let report: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
        let rows = report["metrics"]
            .as_array()
            .ok_or("missing benchmark metrics")?;
        let metric = |name: &str, unit: &str| -> Result<(&Value, f64)> {
            let mut matches = rows.iter().filter(|r| r["name"] == name);
            let row = matches
                .next()
                .ok_or_else(|| format!("missing metric {name}"))?;
            if matches.next().is_some() || row["current"]["unit"] != unit {
                return Err(format!("ambiguous metric or wrong unit for {name}"));
            }
            Ok((&row["details"], number(&row["current"], "value")?))
        };
        let (idle, idle_cpu) = metric("idle_cpu_percent", "%")?;
        let count = |name: &str| {
            idle[name]
                .as_u64()
                .ok_or_else(|| format!("missing idle count {name}"))
        };
        let gpu =
            |n: u32| metric(&format!("render/sparse_ui/{n}"), "ns").map(|(_, ns)| ns / 1_000_000.0);
        Ok(Self {
            submit: metric("input_to_submit_ms", "ms")?.1,
            display: metric("input_to_pixel_ms", "ms")?.1,
            idle_cpu,
            idle_seconds: number(idle, "interval_seconds")?,
            counts: [
                count("submitted_frames")?,
                count("frames")?,
                count("display_link_events")?,
            ],
            gpu_ms: [gpu(1)?, gpu(128)?, gpu(1000)?],
        })
    }
}

impl Run {
    fn id(&self) -> String {
        format!("{}-{}", self.meta.measured_at, &self.meta.head[..7])
    }

    fn read(dir: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        for item in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if matches!(name, "run.json" | "bench.json" | "m7-scroll.json")
                || (name.starts_with("animation-") && name.ends_with(".json"))
            {
                if !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                {
                    return Err(format!("invalid artifact name {name}"));
                }
                files.insert(name.to_owned(), fs::read(&path).map_err(|e| e.to_string())?);
            }
        }
        let meta: Meta = serde_json::from_slice(files.get("run.json").ok_or("missing run.json")?)
            .map_err(|e| e.to_string())?;
        if !hash(&meta.head)
            || meta.implementation.as_deref().is_some_and(|s| !hash(s))
            || meta.measured_at == 0
            || meta.machine.trim().is_empty()
            || meta.label.iter().any(|s| s.trim().is_empty())
            || meta.latency_decimals.unwrap_or(4) > 6
        {
            return Err(
                "run.json needs full commit hashes, measurement time, machine and bilingual labels"
                    .into(),
            );
        }
        if meta
            .pointer_probes
            .iter()
            .any(|p| p.inputs == 0 || p.visible > p.inputs || p.display_p95_ms < 0.0)
            || meta.continuous_submit_ms.is_some_and(|v| v < 0.0)
        {
            return Err("invalid fullscreen input probe".into());
        }
        let native = files
            .get("bench.json")
            .map(|bytes| Native::read(bytes))
            .transpose()?;
        let mut animation = Vec::new();
        for (name, bytes) in &files {
            if !name.starts_with("animation-") {
                continue;
            }
            let a: Animation = serde_json::from_slice(bytes).map_err(|e| format!("{name}: {e}"))?;
            let c = &a.conditions;
            if c.objects == 0
                || c.pixel_width == 0
                || c.pixel_height == 0
                || c.scale <= 0.0
                || a.sample_seconds <= 0.0
                || a.warmup_seconds < 0.0
                || a.displayed_fps < 0.0
                || a.display_gap_p95_ms < 0.0
            {
                return Err(format!("invalid sample in {name}"));
            }
            animation.push(a);
        }
        let scroll = files
            .get("m7-scroll.json")
            .map(|data| ScrollReport::read(data))
            .transpose()?;
        if native.is_none() && animation.is_empty() && scroll.is_none() {
            return Err("expected bench.json, animation-*.json or m7-scroll.json".into());
        }
        Ok(Self {
            meta,
            native,
            animation,
            scroll,
            files,
        })
    }
}

fn read_archive(root: &Path) -> Result<Vec<Run>> {
    let mut runs = Vec::new();
    for entry in fs::read_dir(root.join(RUNS)).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_dir()
            || path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let run = Run::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if path.file_name().and_then(|s| s.to_str()) != Some(run.id().as_str()) {
            return Err(format!("expected run directory {}", run.id()));
        }
        runs.push(run);
    }
    runs.sort_by_key(|r| (r.meta.measured_at, r.id()));
    if runs.is_empty() {
        return Err("performance archive is empty".into());
    }
    Ok(runs)
}

fn replace_section(text: &str, body: &str) -> Result<String> {
    if text.matches(BEGIN).count() != 1 || text.matches(END).count() != 1 {
        return Err("expected one pair of performance report markers".into());
    }
    let (prefix, rest) = text.split_once(BEGIN).ok_or("missing opening marker")?;
    let (_, suffix) = rest
        .split_once(END)
        .ok_or("closing marker precedes opening marker")?;
    Ok(format!(
        "{prefix}{BEGIN}\n\n{}\n\n{END}{suffix}",
        body.trim_end()
    ))
}

fn outputs(root: &Path, runs: &[Run]) -> Result<Vec<(PathBuf, String)>> {
    let latest = &runs[runs.len().saturating_sub(LIMIT)..];
    let mut out = Vec::new();
    for (file, lang) in [
        ("README.md", render::Lang::En),
        ("README_ZH.md", render::Lang::Zh),
    ] {
        let path = root.join(file);
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        out.push((
            path,
            replace_section(&text, &render::summary(latest, lang))?,
        ));
    }
    out.push((root.join(HISTORY), render::history(runs)));
    Ok(out)
}

pub(crate) fn check(root: &Path) -> Result<()> {
    for (path, expected) in outputs(root, &read_archive(root)?)? {
        if fs::read_to_string(&path).ok().as_ref() != Some(&expected) {
            return Err(format!(
                "{} is stale; run cargo xtask perf-report",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(crate) struct PerfReport {
    pub check: bool,
    pub add: Option<PathBuf>,
    pub start: Option<PathBuf>,
    pub machine: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    String::from_utf8(out.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|e| e.to_string())
}

fn start_run(root: &Path, dir: &Path, machine: &str) -> Result<()> {
    if machine.trim().is_empty() {
        return Err("provide the measurement machine and display".into());
    }
    let head = git(root, &["rev-parse", "HEAD"])?;
    let dirty = !git(root, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty();
    let measured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let metadata = serde_json::json!({"head":head,"dirty":dirty,"measured_at":measured_at,"machine":machine,"label":["Measurement","实测"]});
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    if fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .next()
        .is_some()
    {
        return Err("choose an empty measurement directory".into());
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join("run.json"))
        .map_err(|e| e.to_string())?;
    writeln!(
        file,
        "{}",
        serde_json::to_string_pretty(&metadata).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())
}

fn write_page(path: &Path, page: &str) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing report filename"))?
        .to_string_lossy();
    let temp = path.with_file_name(format!(".{name}.perf-report-{}", std::process::id()));
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = out.write_all(page.as_bytes()).and_then(|()| {
        drop(out);
        fs::rename(&temp, path)
    });
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

impl PerfReport {
    fn update(&self, root: &Path) -> Result<()> {
        let lock = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("target/xtask/perf-report.lock"))
            .map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        if self.check {
            return check(root);
        }
        if let Some(dir) = &self.start {
            return start_run(root, dir, self.machine.as_deref().unwrap_or_default());
        }
        let mut runs = read_archive(root)?;
        let mut added = None;
        if let Some(input) = &self.add {
            let run = Run::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
            git(
                root,
                &["cat-file", "-e", &format!("{}^{{commit}}", run.meta.head)],
            )?;
            let id = run.id();
            if let Some(previous) = runs.iter().find(|r| r.id() == id) {
                if previous.files != run.files {
                    return Err(format!("record {id} already exists with different data"));
                }
            } else {
                added = Some(id);
                runs.push(run);
                runs.sort_by_key(|r| (r.meta.measured_at, r.id()));
            }
        }
        let pages = outputs(root, &runs)?;
        if let Some(id) = added {
            let run = runs
                .iter()
                .find(|r| r.id() == id)
                .ok_or("missing new performance run")?;
            let parent = root.join(RUNS);
            let stage = parent.join(format!(".{id}-{}", std::process::id()));
            fs::create_dir(&stage).map_err(|e| e.to_string())?;
            let result = (|| -> io::Result<()> {
                for (name, data) in &run.files {
                    fs::write(stage.join(name), data)?;
                }
                fs::rename(&stage, parent.join(id))
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(stage);
            }
            result.map_err(|e| e.to_string())?;
        }
        for (path, page) in pages {
            if fs::read_to_string(&path).ok().as_ref() != Some(&page) {
                write_page(&path, &page).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        Ok(())
    }
}

impl Check for PerfReport {
    fn name(&self) -> &str {
        "perf-report"
    }
    fn run(&self, root: &Path) -> (Status, String) {
        match self.update(root) {
            Ok(()) if self.start.is_some() => (
                Status::Pass,
                "Captured measurement revision in run.json.".into(),
            ),
            Ok(()) => (
                Status::Pass,
                "Performance history and both README excerpts are synchronized.".into(),
            ),
            Err(e) => (Status::Fail, e),
        }
    }
}
