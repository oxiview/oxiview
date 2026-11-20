//! Local performance baselines and Divan median comparisons.

use crate::{check::Check, report::Status};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::SystemTime};

mod native;
mod record;
#[cfg(test)]
mod scope_tests;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Lower,
    Higher,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Metric {
    value: Option<f64>,
    unit: String,
    direction: Direction,
    #[serde(default = "default_threshold")]
    regression_percent: f64,
    target: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_interval_ms: Option<f64>,
}
fn default_threshold() -> f64 {
    10.0
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Machine {
    model: String,
    cpu: String,
    memory_bytes: u64,
    os: String,
    rustc: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    schema_version: u32,
    machine_id: String,
    machine: Machine,
    recorded_at_unix_seconds: u64,
    metrics: BTreeMap<String, Metric>,
}

fn command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(format!("{program} returned empty machine information"));
    }
    Ok(text)
}

// mini excludes serial numbers and UUIDs; extract only hardware fields equivalent to sysctl.
fn macos_hardware_fallback() -> Result<(String, String, String)> {
    let text = command(
        "system_profiler",
        &["SPHardwareDataType", "-detailLevel", "mini", "-json"],
    )?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let hardware = &data["SPHardwareDataType"][0];
    let field = |name| {
        hardware[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Missing hardware field: {name}"))
    };
    let model = field("machine_model")?;
    let cpu = field("chip_type").or_else(|_| field("cpu_type"))?;
    let memory = field("physical_memory")?;
    let words: Vec<_> = memory.split_whitespace().collect();
    if words.len() != 2 {
        return Err("Cannot parse physical memory".into());
    }
    let scale = match words[1] {
        "GB" => 1_u64 << 30,
        "MB" => 1_u64 << 20,
        "TB" => 1_u64 << 40,
        _ => return Err("Unknown memory unit".into()),
    };
    let bytes = words[0]
        .parse::<u64>()
        .map_err(|e| e.to_string())?
        .checked_mul(scale)
        .ok_or("Memory value overflow")?;
    Ok((model, cpu, bytes.to_string()))
}

fn machine() -> Result<Machine> {
    let (model, cpu, memory, os) = if cfg!(target_os = "macos") {
        let hardware = (|| {
            Ok::<_, String>((
                command("sysctl", &["-n", "hw.model"])?,
                command("sysctl", &["-n", "machdep.cpu.brand_string"])?,
                command("sysctl", &["-n", "hw.memsize"])?,
            ))
        })()
        .or_else(|_| macos_hardware_fallback())?;
        (
            hardware.0,
            hardware.1,
            hardware.2,
            command("sw_vers", &["-productVersion"])?,
        )
    } else if cfg!(target_os = "linux") {
        let cpu = fs::read_to_string("/proc/cpuinfo").map_err(|e| e.to_string())?;
        let memory = fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
        let kb: u64 = memory
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))
            .and_then(|v| v.split_whitespace().next())
            .ok_or("Missing memory information")?
            .parse::<u64>()
            .map_err(|e| e.to_string())?;
        (
            fs::read_to_string("/sys/class/dmi/id/product_name")
                .map_err(|e| e.to_string())?
                .trim()
                .into(),
            cpu.lines()
                .find_map(|l| {
                    l.strip_prefix("model name")
                        .and_then(|v| v.split_once(':'))
                        .map(|(_, v)| v.trim().to_owned())
                })
                .ok_or("Missing CPU model")?,
            (kb * 1024).to_string(),
            command("uname", &["-sr"])?,
        )
    } else if cfg!(target_os = "windows") {
        let ps = |s| {
            command(
                "powershell",
                &["-NoProfile", "-NonInteractive", "-Command", s],
            )
        };
        (
            ps("(Get-CimInstance Win32_ComputerSystem).Model")?,
            ps("(Get-CimInstance Win32_Processor | Select-Object -First 1).Name")?,
            ps("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory")?,
            ps("(Get-CimInstance Win32_OperatingSystem).Version")?,
        )
    } else {
        return Err("Only native macOS / Windows / Linux platforms are supported".into());
    };
    Ok(Machine {
        model,
        cpu,
        memory_bytes: memory.parse::<u64>().map_err(|e| e.to_string())?,
        os: format!("{} {os}", std::env::consts::OS),
        rustc: command("rustc", &["--version"])?,
    })
}

fn machine_id(machine: &Machine) -> String {
    // Use fixed FNV-1a to avoid standard-library hash or seed changes; do not collect serial numbers.
    let hardware = format!(
        "{}\n{}\n{}\n{}",
        std::env::consts::ARCH,
        machine.model,
        machine.cpu,
        machine.memory_bytes
    );
    let hash = hardware.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{}-{hash:016x}", std::env::consts::OS)
}

fn duration_ns(text: &str) -> Result<f64> {
    let words: Vec<_> = text.split_whitespace().collect();
    if words.len() != 2 {
        return Err(format!("Invalid duration: {text}"));
    }
    let scale = match words[1] {
        "ps" => 0.001,
        "ns" => 1.0,
        "µs" | "μs" | "us" => 1_000.0,
        "ms" => 1_000_000.0,
        "s" => 1_000_000_000.0,
        _ => return Err(format!("Unknown duration unit: {text}")),
    };
    let value = words[0].parse::<f64>().map_err(|e| e.to_string())? * scale;
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("Invalid measurement: {text}"));
    }
    Ok(value)
}

fn parse_divan(text: &str) -> Result<BTreeMap<String, f64>> {
    let mut values = BTreeMap::new();
    let mut parents: Vec<(usize, String)> = Vec::new();
    let mut header = false;
    for line in text.lines() {
        let content = line.trim_start_matches(|c: char| c.is_whitespace() || "├╰─│".contains(c));
        let prefix = line.chars().count() - content.chars().count();
        let columns: Vec<_> = content.split('│').collect();
        if line.contains("fastest") && line.contains("median") {
            if columns.len() != 6 || columns[2].trim() != "median" {
                return Err("Divan table header changed".into());
            }
            parents.clear();
            let name = columns[0]
                .split_whitespace()
                .next()
                .ok_or("Missing benchmark target")?;
            parents.push((0, name.into()));
            header = true;
            continue;
        }
        if !header || !line.contains('─') {
            continue;
        }
        if columns.len() != 6 {
            return Err(format!("Divan row format changed: {line}"));
        }
        let name = columns[0]
            .split_whitespace()
            .next()
            .ok_or("Empty benchmark name")?;
        while parents.last().is_some_and(|(depth, _)| *depth >= prefix) {
            parents.pop();
        }
        let mut path: Vec<_> = parents.iter().map(|(_, n)| n.as_str()).collect();
        path.push(name);
        if columns[2].trim().is_empty() {
            if columns
                .iter()
                .skip(1)
                .any(|column| !column.trim().is_empty())
            {
                return Err("Incomplete Divan statistics columns".into());
            }
            parents.push((prefix, name.into()));
            continue;
        }
        let first: Vec<_> = columns[0].split_whitespace().collect();
        if first.len() != 3 {
            return Err(
                "Benchmark names must contain no whitespace and include fastest statistics".into(),
            );
        }
        duration_ns(&first[1..].join(" "))?;
        // Every leaf must have complete statistics; never silently skip metrics that fail to parse.
        duration_ns(columns[1])?;
        let value = duration_ns(columns[2])?;
        duration_ns(columns[3])?;
        for count in &columns[4..] {
            if count.trim().parse::<u64>().map_err(|e| e.to_string())? == 0 {
                return Err("Empty samples".into());
            }
        }
        let key = path.join("/");
        if values.insert(key.clone(), value).is_some() {
            return Err(format!("Duplicate metric: {key}"));
        }
    }
    if values.is_empty() {
        return Err("No Divan metrics parsed".into());
    }
    Ok(values)
}

fn pending() -> BTreeMap<String, Metric> {
    [
        ("input_to_submit_ms", "ms", 0.0),
        ("input_to_pixel_ms", "ms", 0.0),
        ("idle_cpu_percent", "%", 0.0),
        ("list_100k_scroll_dropped_frames", "frames", 0.0),
    ]
    .into_iter()
    .map(|(name, unit, target)| {
        (
            name.into(),
            Metric {
                value: None,
                unit: unit.into(),
                direction: Direction::Lower,
                regression_percent: default_threshold(),
                target: Some(target),
                refresh_interval_ms: None,
            },
        )
    })
    .collect()
}

fn validate(baseline: &Baseline) -> Result<()> {
    if baseline.schema_version != 1 || baseline.machine_id != machine_id(&baseline.machine) {
        return Err("Invalid baseline version or machine identifier".into());
    }
    for (name, metric) in &baseline.metrics {
        if !metric.regression_percent.is_finite()
            || !(0.0..=100.0).contains(&metric.regression_percent)
            || metric.value.is_some_and(|v| !v.is_finite() || v < 0.0)
            || metric
                .refresh_interval_ms
                .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            return Err(format!("Invalid baseline metric: {name}"));
        }
        if let Some(reserved) = pending().get(name) {
            if (name == "list_100k_scroll_dropped_frames" && metric.value.is_some())
                || metric.target.is_some_and(|v| !v.is_finite() || v < 0.0)
                || (name == "list_100k_scroll_dropped_frames" && metric.target != reserved.target)
                || metric.unit != reserved.unit
                || metric.direction != reserved.direction
            {
                return Err(format!(
                    "Invalid hard-target metadata (M7 list must remain pending): {name}"
                ));
            }
        } else if metric.value.is_none()
            || metric.unit != "ns"
            || metric.direction != Direction::Lower
        {
            return Err(format!(
                "Incompatible microbenchmark metric metadata: {name}"
            ));
        }
    }
    if pending()
        .keys()
        .any(|key| key != "input_to_submit_ms" && !baseline.metrics.contains_key(key))
    {
        return Err("Baseline is missing reserved hard-target metrics".into());
    }
    Ok(())
}

fn regressed(current: f64, previous: f64, direction: &Direction, percent: f64) -> bool {
    let allowance = previous.abs() * percent / 100.0;
    match direction {
        Direction::Lower => current > previous * (1.0 + percent / 100.0),
        Direction::Higher => previous - current > allowance,
    }
}

// The display link reports a stable nominal interval. Allow only clock
// rounding/jitter (0.1%), not switching between e.g. 60 and 120 Hz.
fn same_display(recorded: Option<f64>, measured: Option<f64>) -> Result<()> {
    if let (Some(recorded), Some(measured)) = (recorded, measured)
        && (recorded - measured).abs() > recorded * 0.001
    {
        return Err("display refresh interval differs from baseline; use the same display or re-record after human confirmation".into());
    }
    Ok(())
}

fn metric_state(
    metric: &Metric,
    old: Option<f64>,
    record: bool,
    hard: Option<bool>,
) -> &'static str {
    match metric.value {
        None => "pending",
        Some(_) if hard == Some(false) => "hard_fail",
        Some(_) if record => "recorded",
        Some(value) => match old {
            None => "missing_baseline",
            Some(old) if regressed(value, old, &metric.direction, metric.regression_percent) => {
                "fail"
            }
            Some(_) => "pass",
        },
    }
}

const M2_METRICS: &[&str] = &[
    "layout/initial_layout/100",
    "layout/initial_layout/1000",
    "layout/initial_layout/10000",
    "ui_tree/arena_edits",
    "ui_tree/clean_preparation",
    "ui_tree/contained_layout_change",
    "ui_tree/hit_testing",
    "ui_tree/paint_only_update",
];
const M3_METRICS: &[&str] = &[
    "components/batched_memo_diamond",
    "components/bound_style_update",
    "components/show_mount_and_dispose",
    "components/keyed_reorder_32",
];
const M4_METRICS: &[&str] = &[
    "text_pipeline/shape_wrapped_paragraph",
    "text_pipeline/shape_rich_paragraph",
    "text_pipeline/cold_label_glyphs_2x",
    "text_pipeline/warm_label_glyphs_2x",
];
const M5_METRICS: &[&str] = &[
    "interaction/tab_256",
    "interaction/action_eight_contexts",
    "interaction/color_token_16_of_256",
];

const M6_METRICS: &[&str] = &[
    "text_editing/replace_and_undo_4k",
    "text_editing/cached_wrapped_geometry_32_lines",
    "text_editing/preedit_16_updates_then_commit",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Full,
    M2,
    M3,
    M4,
    M5,
    M6,
}

impl Scope {
    fn check_name(self) -> &'static str {
        match self {
            Self::Full => "bench",
            Self::M2 => "bench-m2",
            Self::M3 => "bench-m3",
            Self::M4 => "bench-m4",
            Self::M5 => "bench-m5",
            Self::M6 => "bench-m6",
        }
    }

    pub(crate) fn from_check_name(name: &str) -> Option<Self> {
        match name {
            "bench" => Some(Self::Full),
            "bench-m2" => Some(Self::M2),
            "bench-m3" => Some(Self::M3),
            "bench-m4" => Some(Self::M4),
            "bench-m5" => Some(Self::M5),
            "bench-m6" => Some(Self::M6),
            _ => None,
        }
    }

    fn artifacts(self) -> &'static [&'static str] {
        match self {
            Self::Full => &[
                "bench-results.json",
                "bench-1.log",
                "bench-2.log",
                "bench-3.log",
                "bench-latency-1.json",
                "bench-latency-1.log",
                "bench-latency-2.json",
                "bench-latency-2.log",
                "bench-latency-3.json",
                "bench-latency-3.log",
                "bench-idle.json",
                "bench-idle.log",
            ],
            Self::M2 => &[
                "bench-m2-results.json",
                "bench-m2-1.log",
                "bench-m2-2.log",
                "bench-m2-3.log",
            ],
            Self::M3 => &[
                "bench-m3-results.json",
                "bench-m3-1.log",
                "bench-m3-2.log",
                "bench-m3-3.log",
            ],
            Self::M4 => &[
                "bench-m4-results.json",
                "bench-m4-1.log",
                "bench-m4-2.log",
                "bench-m4-3.log",
            ],
            Self::M5 => &[
                "bench-m5-results.json",
                "bench-m5-1.log",
                "bench-m5-2.log",
                "bench-m5-3.log",
            ],
            Self::M6 => &[
                "bench-m6-results.json",
                "bench-m6-1.log",
                "bench-m6-2.log",
                "bench-m6-3.log",
            ],
        }
    }

    pub(crate) fn results_file(self) -> &'static str {
        self.artifacts()[0]
    }

    fn selection(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Self::Full => None,
            Self::M2 => Some(("M2", M2_METRICS)),
            Self::M3 => Some(("M3", M3_METRICS)),
            Self::M4 => Some(("M4", M4_METRICS)),
            Self::M5 => Some(("M5", M5_METRICS)),
            Self::M6 => Some(("M6", M6_METRICS)),
        }
    }

    fn divan_args(self) -> &'static [&'static str] {
        match self {
            Self::Full => &["bench", "--workspace", "--benches", "--locked"],
            Self::M2 => &[
                "bench",
                "--locked",
                "-p",
                "oxiview-layout",
                "-p",
                "oxiview-core",
                "--bench",
                "layout",
                "--bench",
                "ui_tree",
            ],
            Self::M3 => &[
                "bench",
                "--locked",
                "-p",
                "oxiview-core",
                "--bench",
                "components",
            ],
            Self::M4 => &[
                "bench",
                "--locked",
                "-p",
                "oxiview-text",
                "--bench",
                "text_pipeline",
            ],
            Self::M5 => &[
                "bench",
                "--locked",
                "-p",
                "oxiview-core",
                "--bench",
                "interaction",
            ],
            Self::M6 => &[
                "bench",
                "--locked",
                "-p",
                "oxiview-text",
                "--bench",
                "text_editing",
            ],
        }
    }

    fn record_command(self) -> &'static str {
        match self {
            Self::Full => "cargo xtask bench --record",
            Self::M2 => "cargo xtask bench --m2 --record",
            Self::M3 => "cargo xtask bench --m3 --record",
            Self::M4 => "cargo xtask bench --m4 --record",
            Self::M5 => "cargo xtask bench --m5 --record",
            Self::M6 => "cargo xtask bench --m6 --record",
        }
    }
}

pub(crate) struct BenchCheck {
    pub scope: Scope,
    pub record: bool,
}

impl BenchCheck {
    fn process(
        &self,
        root: &Path,
        log: &mut String,
        probe_machine: impl FnOnce() -> Result<Machine>,
        mut run_divan: impl FnMut(&mut Command) -> std::io::Result<std::process::Output>,
        measure_native: impl FnOnce(&Path) -> Result<BTreeMap<String, native::Measurement>>,
    ) -> Result<Status> {
        let artifacts = root.join("target/xtask");
        let legacy = if self.scope == Scope::Full {
            &["bench-latency.json", "bench-latency.log"][..]
        } else {
            &[]
        };
        for artifact in self.scope.artifacts().iter().chain(legacy) {
            match fs::remove_file(artifacts.join(artifact)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let selection = self.scope.selection();
        match self.scope {
            Scope::Full => {}
            Scope::M2 => log.push_str("Scope: M2 layout and UI tree (8 metrics)\n"),
            Scope::M3 => log.push_str("Scope: M3 reactivity and components (4 metrics)\n"),
            Scope::M4 => {
                log.push_str("Scope: M4 text shaping and glyph rasterization (4 metrics)\n")
            }
            Scope::M5 => {
                log.push_str("Scope: M5 focus, Actions and theme resolution (3 metrics)\n")
            }
            Scope::M6 => {
                log.push_str("Scope: M6 Unicode editing, geometry and preedit (3 metrics)\n")
            }
        }
        let machine = probe_machine()?;
        let id = machine_id(&machine);
        log.push_str(&format!("Current machine: {id}, {machine:?}\n"));
        let directory = root.join("perf/baselines");
        let path = directory.join(format!("{id}.json"));
        let original = record::read(&path)?;
        let previous: Option<Baseline> = original
            .as_deref()
            .map(|text| -> Result<Baseline> {
                let baseline: Baseline = serde_json::from_str(text).map_err(|e| e.to_string())?;
                validate(&baseline)?;
                if baseline.machine_id != id {
                    return Err("Baseline filename does not match the current machine".into());
                }
                Ok(baseline)
            })
            .transpose()?;
        if directory.exists() {
            let mut paths = fs::read_dir(&directory)
                .map_err(|e| e.to_string())?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            paths.sort();
            for other in paths {
                if other != path && other.extension().is_some_and(|e| e == "json") {
                    log.push_str(&format!(
                        "Other machine (informational only) {}:\n{}\n",
                        other.display(),
                        fs::read_to_string(&other).map_err(|e| e.to_string())?
                    ));
                }
            }
        }
        let mut rounds = Vec::new();
        for round in 1..=3 {
            let mut command =
                Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
            command
                .args(self.scope.divan_args())
                .current_dir(root)
                .env("CARGO_TERM_COLOR", "never");
            if let Some((_, names)) = selection {
                command
                    .args(["--", "--exact"])
                    .args(names.iter().map(|name| name.replace('/', "::")));
            }
            for (key, _) in std::env::vars_os() {
                if key.to_string_lossy().starts_with("DIVAN_") {
                    command.env_remove(key);
                }
            }
            let output = run_divan(&mut command).map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let raw = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
            let raw_file = format!("{}-{round}.log", self.scope.check_name());
            fs::write(artifacts.join(&raw_file), raw).map_err(|e| e.to_string())?;
            if !output.status.success() {
                log.push_str(&format!(
                    "Benchmark process failed: {}; see {raw_file}\n",
                    output.status
                ));
                return Ok(if output.status.code().is_some() {
                    Status::Fail
                } else {
                    Status::Error
                });
            }
            rounds.push(parse_divan(&stdout)?);
        }
        if rounds
            .iter()
            .skip(1)
            .any(|r| !r.keys().eq(rounds[0].keys()))
        {
            return Err("Metric sets differ across the three runs".into());
        }
        if let Some((milestone, names)) = selection
            && let Some(name) = rounds[0]
                .keys()
                .find(|name| !names.contains(&name.as_str()))
        {
            return Err(format!("Unexpected {milestone} metric: {name}"));
        }
        let mut metrics = match self.scope {
            Scope::Full => pending(),
            Scope::M2 | Scope::M3 | Scope::M4 | Scope::M5 | Scope::M6 => BTreeMap::new(),
        };
        for name in rounds[0].keys() {
            let mut samples: Vec<_> = rounds.iter().map(|round| round[name]).collect();
            samples.sort_by(f64::total_cmp);
            let threshold = previous
                .as_ref()
                .and_then(|b| b.metrics.get(name))
                .map_or(default_threshold(), |m| m.regression_percent);
            metrics.insert(
                name.clone(),
                Metric {
                    value: Some(samples[1]),
                    unit: "ns".into(),
                    direction: Direction::Lower,
                    regression_percent: threshold,
                    target: None,
                    refresh_interval_ms: None,
                },
            );
        }
        let measured = match self.scope {
            Scope::Full => measure_native(root),
            Scope::M2 | Scope::M3 | Scope::M4 | Scope::M5 | Scope::M6 => Ok(BTreeMap::new()),
        };
        let mut native = match measured {
            Ok(measurements) => measurements,
            Err(error) => {
                let results: Vec<_> = metrics.iter().map(|(name, metric)| {
                    let native = matches!(name.as_str(), "input_to_submit_ms" | "input_to_pixel_ms" | "idle_cpu_percent");
                    serde_json::json!({
                        "name": name, "current": metric,
                        "baseline_value": previous.as_ref().and_then(|b| b.metrics.get(name)).and_then(|m| m.value),
                        "status": if native { "environment_error" } else if metric.value.is_none() { "pending" } else { "not_compared" },
                        "hard_check_pass": null,
                        "error": if native { Some(&error) } else { None }
                    })
                }).collect();
                fs::write(
                    artifacts.join(self.scope.results_file()),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"machine_id": id, "metrics": results}),
                    )
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                return Err(error);
            }
        };
        for (name, measurement) in &native {
            let metric = metrics
                .get_mut(name)
                .ok_or("missing native metric definition")?;
            metric.value = Some(measurement.value);
            metric.target = measurement.target;
            metric.refresh_interval_ms = measurement.refresh_interval_ms;
            metric.regression_percent = previous
                .as_ref()
                .and_then(|b| b.metrics.get(name))
                .map_or(default_threshold(), |m| m.regression_percent);
        }
        let mut environment_error = None;
        // Pixel comparisons require the optimized baseline protocol, including
        // the display interval. Legacy values are deliberately not reused.
        if let Some(pixel) = native.get_mut("input_to_pixel_ms") {
            let old = previous
                .as_ref()
                .and_then(|b| b.metrics.get("input_to_pixel_ms"));
            if let Some(old) = old {
                if !self.record {
                    environment_error =
                        same_display(old.refresh_interval_ms, pixel.refresh_interval_ms).err();
                }
                if let (Some(value), Some(_)) = (old.value, old.refresh_interval_ms) {
                    let target = value * 1.15;
                    pixel.target = Some(target);
                    pixel.hard_pass = Some(pixel.value <= target);
                }
            }
            let metric = metrics
                .get_mut("input_to_pixel_ms")
                .ok_or("missing pixel metric")?;
            metric.regression_percent = 15.0;
            metric.target = pixel.target;
        }
        let mut status = Status::Pass;
        let mut results = Vec::new();
        for (name, metric) in &metrics {
            let old = previous
                .as_ref()
                .and_then(|b| b.metrics.get(name))
                .and_then(|m| {
                    if name == "input_to_pixel_ms" && m.refresh_interval_ms.is_none() {
                        None
                    } else {
                        m.value
                    }
                });
            let hard = native.get(name);
            let state = if name == "input_to_pixel_ms" && environment_error.is_some() {
                "environment_error"
            } else {
                metric_state(metric, old, self.record, hard.and_then(|m| m.hard_pass))
            };
            if metric.value.is_some() && old.is_none() && !self.record {
                log.push_str(&format!("Unmeasured recorded metric or missing baseline: {name}; confirm with the owner, then run {} after human confirmation.\n", self.scope.record_command()));
            }
            if ["fail", "missing_baseline", "hard_fail"].contains(&state) {
                status = Status::Fail;
            }
            log.push_str(&format!(
                "{name}: {state}, current {:?} {}, baseline {old:?}, target {:?}, allowed regression {}%\n",
                metric.value, metric.unit, metric.target, metric.regression_percent
            ));
            if let Some(hard) = hard {
                log.push_str(&format!(
                    "{name}: hard check {:?}; details {}\n",
                    hard.hard_pass, hard.details
                ));
            }
            results.push(serde_json::json!({"name": name, "status": state, "current": metric, "baseline_value": old,
                "hard_check_pass": hard.and_then(|m| m.hard_pass), "details": hard.map(|m| &m.details)}));
        }
        let expected: Vec<&str> = match selection {
            None => previous
                .iter()
                .flat_map(|baseline| baseline.metrics.keys().map(String::as_str))
                .collect(),
            Some((_, names)) => names.to_vec(),
        };
        for name in expected {
            if !metrics.contains_key(name) {
                status = Status::Fail;
                match selection {
                    None => log.push_str(&format!(
                        "Missing recorded metric: {name}; silent removal is prohibited\n"
                    )),
                    Some((milestone, _)) => {
                        log.push_str(&format!("Missing {milestone} measurement: {name}\n"))
                    }
                }
                let old = previous
                    .as_ref()
                    .and_then(|baseline| baseline.metrics.get(name))
                    .and_then(|metric| metric.value);
                results.push(serde_json::json!({
                    "name": name, "status": "missing_measurement", "baseline_value": old
                }));
            }
        }
        if previous.is_none() && !self.record {
            log.push_str(&format!(
                "No baseline for this machine; run {} after human confirmation.\n",
                self.scope.record_command()
            ));
        }
        let mut report = serde_json::json!({"machine_id":id,"metrics":results});
        if let Some((milestone, _)) = selection {
            report["scope"] = milestone.to_ascii_lowercase().into();
        }
        fs::write(
            artifacts.join(self.scope.results_file()),
            serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let Some(error) = environment_error {
            return Err(error);
        }
        if self.record && status == Status::Pass {
            let content = match (selection, original.as_deref()) {
                (Some(_), Some(text)) => record::merge_measured(text, &metrics)?,
                (scope, _) => {
                    if scope.is_some() {
                        metrics.extend(pending());
                    }
                    let baseline = Baseline {
                        schema_version: 1,
                        machine_id: id,
                        machine,
                        recorded_at_unix_seconds: SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .map_err(|e| e.to_string())?
                            .as_secs(),
                        metrics,
                    };
                    serde_json::to_string_pretty(&baseline).map_err(|e| e.to_string())?
                }
            };
            record::write(&artifacts, &path, original.as_deref(), &content)?;
            log.push_str(&format!(
                "Created or overwritten repository baseline: {}; review and commit it.\n",
                path.display()
            ));
        }
        Ok(status)
    }
}
impl Check for BenchCheck {
    fn name(&self) -> &str {
        self.scope.check_name()
    }
    fn artifacts(&self) -> &[&str] {
        self.scope.artifacts()
    }
    fn run(&self, root: &Path) -> (Status, String) {
        let mut log = String::new();
        match self.process(root, &mut log, machine, Command::output, native::measure) {
            Ok(status) => (status, log),
            Err(error) => (Status::Error, format!("{log}{error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_regressions_and_hard_failures_override_baselines() {
        for name in ["input_to_pixel_ms", "idle_cpu_percent"] {
            let mut metric = pending().remove(name).unwrap();
            metric.value = Some(0.09);
            assert_eq!(metric_state(&metric, Some(0.08), false, Some(true)), "fail");
            assert_eq!(metric_state(&metric, Some(0.09), false, Some(true)), "pass");
            assert_eq!(
                metric_state(&metric, Some(1.0), false, Some(false)),
                "hard_fail"
            );
            assert_eq!(
                metric_state(&metric, Some(1.0), true, Some(false)),
                "hard_fail"
            );
            assert_eq!(
                metric_state(&metric, None, false, Some(true)),
                "missing_baseline"
            );
        }
    }
    #[test]
    fn pixel_boundary_and_display_environment() {
        let threshold = 20.0 * 1.15;
        assert!(!regressed(threshold, 20.0, &Direction::Lower, 15.0));
        assert!(regressed(
            threshold + 0.00001,
            20.0,
            &Direction::Lower,
            15.0
        ));
        assert!(same_display(Some(16.666667), Some(16.666667)).is_ok());
        let mut report = crate::report::Report::default();
        report.checks.push(crate::report::CheckResult::error(
            "bench",
            same_display(Some(16.666667), Some(8.333333)).unwrap_err(),
        ));
        assert_eq!(report.exit_code(), 2);
    }
    #[test]
    fn thresholds_and_directions() {
        assert!(!regressed(110.0, 100.0, &Direction::Lower, 10.0));
        assert!(regressed(110.01, 100.0, &Direction::Lower, 10.0));
        assert!(!regressed(90.0, 100.0, &Direction::Higher, 10.0));
        assert!(regressed(89.0, 100.0, &Direction::Higher, 10.0));
        assert!(regressed(1.0, 0.0, &Direction::Lower, 10.0));
    }
    #[test]
    fn parser_requires_complete_statistics() {
        let text = "compare fastest │ slowest │ median │ mean │ samples │ iters\n╰─ image_compare │ │ │ │ │\n   ├─ 256 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 1000\n   ╰─ 1024 1 ms │ 3 ms │ 2 ms │ 2 ms │ 100 │ 1000";
        let result = parse_divan(text).expect("valid output");
        assert_eq!(result["compare/image_compare/256"], 2000.0);
        assert_eq!(result["compare/image_compare/1024"], 2_000_000.0);
        for invalid in [
            text.replace("2 µs", "NaN µs"),
            text.replace("2 ms", "2 ticks"),
            text.replace("median", "p50"),
            String::new(),
        ] {
            assert!(parse_divan(&invalid).is_err());
        }
    }
    #[test]
    fn parser_handles_nested_branches_and_duplicate_metrics() {
        let text = "bench fastest │ slowest │ median │ mean │ samples │ iters\n├─ first │ │ │ │ │\n│  ╰─ 1 1 ns │ 3 ns │ 2 ns │ 2 ns │ 10 │ 10\n╰─ second 1 ns │ 4 ns │ 3 ns │ 3 ns │ 10 │ 10";
        let values = parse_divan(text).expect("nested output");
        assert_eq!(values["bench/first/1"], 2.0);
        assert_eq!(values["bench/second"], 3.0);
        assert!(parse_divan(&format!("{text}\n{text}")).is_err());
    }

    #[test]
    fn m2_divan_metrics_require_baselines() {
        let values = parse_divan(scope_tests::DIVAN_M2).expect("both M2 targets");
        assert_eq!(
            values.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "layout/initial_layout/100",
                "layout/initial_layout/1000",
                "layout/initial_layout/10000",
                "ui_tree/arena_edits",
                "ui_tree/clean_preparation",
                "ui_tree/contained_layout_change",
                "ui_tree/hit_testing",
                "ui_tree/paint_only_update",
            ]
        );
        for value in values.into_values() {
            let metric = Metric {
                value: Some(value),
                unit: "ns".into(),
                direction: Direction::Lower,
                regression_percent: default_threshold(),
                target: None,
                refresh_interval_ms: None,
            };
            assert_eq!(metric_state(&metric, None, false, None), "missing_baseline");
        }
    }

    #[test]
    fn baseline_accepts_native_but_rejects_pending_measurements() {
        let machine = Machine {
            model: "model".into(),
            cpu: "cpu".into(),
            memory_bytes: 100,
            os: "os".into(),
            rustc: "rustc".into(),
        };
        let mut baseline = Baseline {
            schema_version: 1,
            machine_id: machine_id(&machine),
            machine,
            recorded_at_unix_seconds: 1,
            metrics: pending(),
        };
        assert!(validate(&baseline).is_ok());
        baseline
            .metrics
            .get_mut("idle_cpu_percent")
            .expect("hard-target metric")
            .value = Some(0.0);
        assert!(validate(&baseline).is_ok());
        baseline.metrics.get_mut("input_to_pixel_ms").unwrap().value = Some(8.0);
        assert!(validate(&baseline).is_ok());
        baseline
            .metrics
            .get_mut("list_100k_scroll_dropped_frames")
            .unwrap()
            .value = Some(0.0);
        assert!(validate(&baseline).is_err());
        baseline.metrics = pending();
        baseline
            .metrics
            .get_mut("idle_cpu_percent")
            .expect("hard-target metric")
            .regression_percent = -1.0;
        assert!(validate(&baseline).is_err());
        let metric: Metric =
            serde_json::from_str(r#"{"value":1.0,"unit":"ns","direction":"lower","target":null}"#)
                .expect("default threshold");
        assert_eq!(metric.regression_percent, 10.0);
    }

    #[test]
    fn identity_excludes_software_and_pending_has_no_measurement() {
        let mut m = Machine {
            model: "model".into(),
            cpu: "cpu".into(),
            memory_bytes: 100,
            os: "os".into(),
            rustc: "rustc".into(),
        };
        let id = machine_id(&m);
        m.os = "new".into();
        m.rustc = "new".into();
        assert_eq!(id, machine_id(&m));
        m.memory_bytes += 1;
        assert_ne!(id, machine_id(&m));
        assert_eq!(pending().len(), 4);
        assert!(pending().values().all(|m| m.value.is_none()));
    }
}
