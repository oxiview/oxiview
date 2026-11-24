//! Run the Apple Animation Hitches observer against the unchanged native M7 scroll.

use super::{Check, Result, Status, native, scroll};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

pub(crate) const NAME: &str = "bench-m7-hitches";
pub(crate) const RESULTS: &str = "bench-m7-hitches-results.json";
const METRIC: &str = "list_100k_scroll_apple_hitch_ms_per_second";
const TARGET: f64 = 1.2;

pub(crate) struct AppleHitchCheck;

impl Check for AppleHitchCheck {
    fn name(&self) -> &str {
        NAME
    }

    fn artifacts(&self) -> &[&str] {
        &[RESULTS]
    }

    fn run(&self, root: &Path) -> (Status, String) {
        match run(root) {
            Ok(measurement) => measurement,
            Err(error) => (Status::Error, error),
        }
    }
}

fn run(root: &Path) -> Result<(Status, String)> {
    let artifacts = root.join("target/xtask");
    let results = artifacts.join(RESULTS);
    match fs::remove_file(&results) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    if !cfg!(target_os = "macos") {
        return Err("Animation Hitches requires macOS, Xcode and a visible native display".into());
    }
    let (relative_path, directory) = native::scroll_directory(root, "m7-hitches")?;
    match measure(root, &directory) {
        Ok((status, value, details)) => {
            let status_name = match status {
                Status::Pass => "pass",
                Status::Fail => "hard_fail",
                Status::Error => "environment_error",
            };
            let report = json!({
                "scope": "m7-hitches",
                "artifact_directory": relative_path,
                "metrics": [{
                    "name": METRIC,
                    "status": status_name,
                    "current": {"value": value, "unit": "ms/s", "target": TARGET},
                    "hard_check_pass": details["hard_check_pass"],
                    "details": details,
                }],
            });
            let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
            fs::write(directory.join("result.json"), &bytes).map_err(|error| error.to_string())?;
            fs::write(results, bytes).map_err(|error| error.to_string())?;
            let coverage = if status == Status::Error {
                " The trace and Metal records do not establish a valid reference result."
            } else {
                ""
            };
            Ok((
                status,
                format!(
                    "Apple Animation Hitches observed {value:.6} ms/s during the native 100,000-row scroll (target <= {TARGET} ms/s).{coverage}\nEvidence: {}\n",
                    relative_path.display()
                ),
            ))
        }
        Err(error) => Err(format!("{error}\nEvidence: {}", relative_path.display())),
    }
}

fn measure(root: &Path, directory: &Path) -> Result<(Status, f64, Value)> {
    let viewer = native::release_viewer(root, directory)?;
    let hash = native::executable_sha256(&viewer)?;
    fs::write(
        directory.join("provenance.json"),
        json!({"executable": native::VIEWER_EXECUTABLE, "executable_sha256": hash, "arguments": ["--measure-scroll", "--json"]}).to_string(),
    ).map_err(|error| error.to_string())?;
    let raw = directory.join("viewer.json");
    let output = Command::new("/usr/bin/python3")
        .args(["-I", "-B"])
        .arg(root.join("xtask/m7_hitches/observe.py"))
        .arg(directory.join("viewer.trace"))
        .arg(directory.join("viewer-output.log"))
        .arg(directory.join("observer"))
        .arg("--viewer")
        .arg(&viewer)
        .arg("--raw-json")
        .arg(&raw)
        .env_remove("OXIVIEW_METAL_TIMING_FILE")
        .current_dir(root)
        .output()
        .map_err(|error| format!("Cannot run the Apple observer with Xcode Python: {error}"))?;
    fs::write(directory.join("observer.log"), &output.stderr).map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Apple observer failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    fs::write(directory.join("observer.json"), &output.stdout)
        .map_err(|error| error.to_string())?;
    let verdict = validate(
        &output.stdout,
        &fs::read_to_string(raw).map_err(|error| error.to_string())?,
    )?;
    if verdict.2["executable_sha256"] != hash || native::executable_sha256(&viewer)? != hash {
        return Err("The observer and capture disagree about the release executable".into());
    }
    Ok(verdict)
}

#[derive(Deserialize)]
struct Hitch {
    trace_start_ns: i64,
    duration_ns: u64,
    swap_id: u32,
    surface_id: u32,
}

#[derive(Deserialize)]
struct Observation {
    method: String,
    app_pid: u32,
    display: Option<String>,
    surface_display: Option<String>,
    host_origin_ns: u64,
    sample_start_trace_ns: u64,
    sample_end_trace_ns: u64,
    active_scroll_duration_ns: u64,
    measured_metal_presentations: u64,
    metal_reported_empty_slots: u64,
    metal_missed_slot_ms_per_second: f64,
    apple_frame_lifetime_matches: u64,
    app_tagged_surface_matches: u64,
    app_surface_matches_bracket_sample: bool,
    conflicting_apple_lifetimes: u64,
    trace_health: TraceHealth,
    unmatched_app_hitches: u64,
    correlated_app_hitches: Vec<Hitch>,
    observed_hitch_time_ns: u64,
    observed_hitch_ms_per_second: f64,
    complete_apple_frame_lifetime_coverage: bool,
    threshold_ms_per_second: f64,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    hard_check_pass: Option<bool>,
    target_exit_status: i32,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum TraceHealth {
    NoLossReported,
    LossReported,
    Unavailable,
}

fn validate(bytes: &[u8], raw: &str) -> Result<(Status, f64, Value)> {
    let observation: Observation = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid Apple observer JSON: {error}"))?;
    let capture = scroll::parse(raw, Some(observation.target_exit_status))?;
    let metal = &capture.dropped.details;
    let start = observation.sample_start_trace_ns;
    let end = observation.sample_end_trace_ns;
    let elapsed = end
        .checked_sub(start)
        .filter(|&value| value > 0)
        .ok_or("Invalid Apple observer sample interval")?;
    if observation.method != "Apple Instruments Animation Hitches"
        || observation.app_pid == 0
        || observation.display.as_deref().is_some_and(str::is_empty)
        || observation.display.is_some() != observation.surface_display.is_some()
        || observation.host_origin_ns.checked_add(start) != metal["start_ns"].as_u64()
        || observation.host_origin_ns.checked_add(end) != metal["end_ns"].as_u64()
        || observation.active_scroll_duration_ns != elapsed
        || metal["displayed_frames"] != observation.measured_metal_presentations
        || metal["dropped_frames"] != observation.metal_reported_empty_slots
        || observation.apple_frame_lifetime_matches > observation.measured_metal_presentations
        || observation.app_tagged_surface_matches > observation.apple_frame_lifetime_matches
        || observation.threshold_ms_per_second != TARGET
    {
        return Err("Invalid or inconsistent Apple observer evidence".into());
    }
    let mut spans = Vec::new();
    let mut swaps = std::collections::BTreeMap::new();
    for hitch in &observation.correlated_app_hitches {
        let beginning = i128::from(hitch.trace_start_ns);
        let finish = beginning + i128::from(hitch.duration_ns);
        let lo = beginning.max(i128::from(start));
        let hi = finish.min(i128::from(end));
        if hi <= lo || hitch.swap_id == 0 || hitch.surface_id == 0 || observation.display.is_none()
        {
            return Err("Invalid correlated Apple hitch interval".into());
        }
        let identity = (beginning, finish, hitch.surface_id);
        if let Some(previous) = swaps.insert(hitch.swap_id, identity) {
            if previous != identity {
                return Err("Conflicting correlated Apple hitch intervals for one swap".into());
            }
            continue;
        }
        spans.push((lo as u64, hi as u64));
    }
    spans.sort_unstable();
    let mut last = start;
    let mut total = 0;
    for (lo, hi) in spans {
        if hi > last {
            total += hi - last.max(lo);
            last = hi;
        }
    }
    let ratio = 1000.0 * total as f64 / elapsed as f64;
    let expected = metal["expected_slots"]
        .as_u64()
        .ok_or("Missing Metal sample slot count")?;
    let missed_slot_time = 1000.0 * observation.metal_reported_empty_slots as f64 / expected as f64;
    let built_in = observation
        .surface_display
        .as_deref()
        .is_some_and(|display| {
            display == "Built-In Display"
                || display
                    .strip_prefix("Built-In Display ")
                    .is_some_and(|number| {
                        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                    })
        });
    let complete = observation.apple_frame_lifetime_matches
        == observation.measured_metal_presentations
        && observation.app_tagged_surface_matches > 0
        && observation.app_surface_matches_bracket_sample
        && observation.conflicting_apple_lifetimes == 0
        && observation.unmatched_app_hitches == 0
        && built_in
        && observation.trace_health == TraceHealth::NoLossReported;
    let (status, hard_pass) = if ratio > TARGET {
        (Status::Fail, Some(false))
    } else if complete && capture.reference_issues.is_empty() && missed_slot_time <= TARGET {
        (Status::Pass, Some(true))
    } else {
        (Status::Error, None)
    };
    if observation.observed_hitch_time_ns != total
        || (observation.observed_hitch_ms_per_second - ratio).abs() > 1e-9
        || (observation.metal_missed_slot_ms_per_second - missed_slot_time).abs() > 1e-9
        || observation.complete_apple_frame_lifetime_coverage != complete
        || observation.hard_check_pass != (ratio > TARGET).then_some(false)
    {
        return Err("Apple observer summary disagrees with correlated hitch intervals".into());
    }
    let mut details: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    details["hard_check_pass"] = json!(hard_pass);
    details["metal_reference_issues"] = json!(capture.reference_issues);
    details["metal_capture"] = capture.dropped.details;
    Ok((status, ratio, details))
}

#[cfg(test)]
#[path = "apple_tests.rs"]
mod tests;
