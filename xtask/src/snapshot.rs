//! Review and explicitly accept pending snapshots; not included in verify.

use crate::{check::Check, report::Status};
use serde::Deserialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    name: String,
    crate_name: String,
    crate_root: PathBuf,
    reason: String,
    comparison: Option<Metrics>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    passed: bool,
    dimensions_match: bool,
    diff_pixels: u64,
    diff_ratio: f64,
    max_channel_diff: u8,
}

struct Entry {
    pending: Pending,
    directory: PathBuf,
    baseline: PathBuf,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

fn directories(path: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            result.push(entry.path());
        }
    }
    result.sort();
    Ok(result)
}

// Reject symlink directories so records cannot redirect acceptance outside the caller's root.
fn validate_destination(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(
                "Snapshot paths must be ordinary relative paths inside the workspace".into(),
            );
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "Snapshot paths must not contain symlinks: {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}

fn pending_entries(root: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let artifacts = validate_destination(root, Path::new("target/xtask/snapshots"))?;
    for crate_dir in directories(&artifacts)? {
        for directory in directories(&crate_dir)? {
            let source = match fs::read(directory.join("pending.json")) {
                Ok(source) => source,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            };
            let pending: Pending = serde_json::from_slice(&source)
                .map_err(|e| format!("{}: {e}", directory.display()))?;
            if !valid_name(&pending.name)
                || !valid_name(&pending.crate_name)
                || directory.file_name() != Some(pending.name.as_ref())
                || crate_dir.file_name() != Some(pending.crate_name.as_ref())
            {
                return Err(format!(
                    "Snapshot name does not match artifact path: {}",
                    directory.display()
                ));
            }
            let crate_root = validate_destination(root, &pending.crate_root)?;
            if !crate_root.join("Cargo.toml").is_file() {
                return Err(format!(
                    "Snapshot caller is missing Cargo.toml: {}",
                    crate_root.display()
                ));
            }
            let baseline = validate_destination(
                root,
                &pending
                    .crate_root
                    .join("tests/snapshots")
                    .join(format!("{}.png", pending.name)),
            )?;
            entries.push(Entry {
                pending,
                directory,
                baseline,
            });
        }
    }
    Ok(entries)
}

pub(crate) fn has_pending(root: &Path) -> bool {
    pending_entries(root).is_ok_and(|entries| !entries.is_empty())
}

fn process(root: &Path, accept: Option<&str>) -> Result<String, String> {
    let entries = pending_entries(root)?;
    let selected: Vec<_> = entries
        .iter()
        .filter(|entry| {
            accept.is_none_or(|name| {
                name == "all"
                    || entry.pending.name == name
                    || format!("{}/{}", entry.pending.crate_name, entry.pending.name) == name
            })
        })
        .collect();
    if let Some(name) = accept {
        if selected.is_empty() {
            return Err(format!("No pending snapshot: {name}"));
        }
        if name != "all" && selected.len() > 1 {
            return Err(format!(
                "Duplicate snapshot name: {name}; select with <crate>/<name>"
            ));
        }
        // Check all source files before writing to avoid partial acceptance from obviously corrupted records.
        for entry in &selected {
            let actual = entry.directory.join("actual.png");
            let meta = fs::symlink_metadata(&actual).map_err(|e| e.to_string())?;
            if !meta.file_type().is_file() {
                return Err(format!(
                    "actual must be a regular file: {}",
                    actual.display()
                ));
            }
        }
    }
    let mut log = String::new();
    for entry in selected {
        let item = &entry.pending;
        log.push_str(&format!(
            "{}/{}: {}\n",
            item.crate_name, item.name, item.reason
        ));
        if let Some(metrics) = &item.comparison {
            log.push_str(&format!(
                "  passed={}, dimensions_equal={}, different_pixels={}, difference_ratio={:.6}, max_channel_difference={}\n",
                metrics.passed,
                metrics.dimensions_match,
                metrics.diff_pixels,
                metrics.diff_ratio,
                metrics.max_channel_diff
            ));
        } else {
            log.push_str("  Difference metrics: no baseline available for comparison\n");
        }
        log.push_str(&format!(
            "  baseline: {}\n  actual: {}\n  diff: {}\n  pending: {}\n",
            entry.baseline.display(),
            entry.directory.join("actual.png").display(),
            entry.directory.join("diff.png").display(),
            entry.directory.join("pending.json").display()
        ));
        if accept.is_some() {
            let existed = entry.baseline.exists();
            fs::create_dir_all(
                entry
                    .baseline
                    .parent()
                    .ok_or("Baseline path has no parent directory")?,
            )
            .map_err(|e| format!("{log}Snapshot acceptance failed: {e}"))?;
            fs::copy(entry.directory.join("actual.png"), &entry.baseline)
                .map_err(|e| format!("{log}Snapshot acceptance failed: {e}"))?;
            log.push_str(&format!(
                "  {} baseline file: {}\n",
                if existed { "Overwritten" } else { "Created" },
                entry.baseline.display()
            ));
            fs::remove_file(entry.directory.join("pending.json"))
                .map_err(|e| format!("{log}Snapshot acceptance failed: {e}"))?;
        }
    }
    if log.is_empty() {
        log.push_str("No pending snapshots.\n");
    }
    Ok(log)
}

pub(crate) struct SnapshotCheck {
    pub accept: Option<String>,
}

impl Check for SnapshotCheck {
    fn name(&self) -> &str {
        "snapshot"
    }
    fn run(&self, root: &Path) -> (Status, String) {
        match process(root, self.accept.as_deref()) {
            Ok(log) => (Status::Pass, log),
            Err(error) => (Status::Error, error),
        }
    }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
