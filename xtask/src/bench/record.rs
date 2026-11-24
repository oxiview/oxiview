//! Preserve prior benchmark data when recording only part of a machine baseline.

use super::{Metric, Result};
use serde::de::IgnoredAny;
use std::{collections::BTreeMap, fs, io::Write, ops::Range, path::Path, time::SystemTime};

pub(super) fn read(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

// Locate values in an already validated JSON object using serde_json's token
// boundaries. Retaining the original text preserves exact decimal values,
// including ones that would be rounded in a deserialize/serialize round trip.
fn members(object: &str) -> Result<BTreeMap<String, Range<usize>>> {
    let mut rest = object
        .strip_prefix('{')
        .ok_or("Expected a baseline JSON object")?;
    let mut fields = BTreeMap::new();
    loop {
        rest = rest.trim_start();
        if rest.starts_with('}') {
            return Ok(fields);
        }
        let mut key = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let name = key
            .next()
            .ok_or("Missing baseline member name")?
            .map_err(|e| e.to_string())?;
        rest = rest[key.byte_offset()..]
            .trim_start()
            .strip_prefix(':')
            .ok_or("Missing baseline member separator")?
            .trim_start();
        let mut value = serde_json::Deserializer::from_str(rest).into_iter::<IgnoredAny>();
        value
            .next()
            .ok_or("Missing baseline member value")?
            .map_err(|e| e.to_string())?;
        let start = object.len() - rest.len();
        let end = start + value.byte_offset();
        fields.insert(name, start..end);
        rest = object[end..].trim_start();
        if let Some(next) = rest.strip_prefix(',') {
            rest = next;
        }
    }
}

pub(super) fn merge_measured(
    original: &str,
    measured: &BTreeMap<String, Metric>,
) -> Result<String> {
    let object = original.trim_start();
    let leading = original.len() - object.len();
    let fields = members(object)?;
    let metrics = fields.get("metrics").ok_or("Missing baseline metrics")?;
    let offset = leading + metrics.start;
    let existing = members(&object[metrics.clone()])?;
    let mut edits = Vec::new();
    let mut additions = Vec::new();
    for (name, metric) in measured {
        if let Some(range) = existing.get(name) {
            let start = offset + range.start;
            let fields = members(&original[start..offset + range.end])?;
            let value = fields
                .get("value")
                .ok_or("Missing recorded benchmark value")?;
            edits.push((
                start + value.start..start + value.end,
                serde_json::to_string(&metric.value).map_err(|e| e.to_string())?,
            ));
            if let Some(refresh) = metric.refresh_interval_ms {
                let refresh = serde_json::to_string(&refresh).map_err(|e| e.to_string())?;
                if let Some(field) = fields.get("refresh_interval_ms") {
                    edits.push((start + field.start..start + field.end, refresh));
                } else {
                    let end = start + fields.values().map(|range| range.end).max().unwrap_or(1);
                    edits.push((
                        end..end,
                        format!(",\n      \"refresh_interval_ms\": {refresh}"),
                    ));
                }
            }
        } else {
            let name = serde_json::to_string(name).map_err(|e| e.to_string())?;
            let metric = serde_json::to_string_pretty(metric).map_err(|e| e.to_string())?;
            additions.push(format!("    {name}: {}", metric.replace('\n', "\n    ")));
        }
    }
    if !additions.is_empty() {
        let end = offset + existing.values().map(|range| range.end).max().unwrap_or(1);
        let separator = if existing.is_empty() { "" } else { "," };
        edits.push((end..end, format!("{separator}\n{}", additions.join(",\n"))));
    }
    edits.sort_unstable_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut output = original.to_owned();
    for (range, replacement) in edits {
        output.replace_range(range, &replacement);
    }
    Ok(output)
}

pub(super) fn write(
    artifacts: &Path,
    path: &Path,
    original: Option<&str>,
    text: &str,
) -> Result<()> {
    let lock = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(artifacts.join("bench-record.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    if read(path)?.as_deref() != original {
        return Err("Baseline changed during measurement; rerun the recording".into());
    }
    let parent = path.parent().ok_or("Missing baseline directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let stage = path.with_extension(format!("recording-{}-{nonce}", std::process::id()));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage)
        .map_err(|e| e.to_string())?;
    let result = output
        .write_all(text.as_bytes())
        .and_then(|()| output.sync_all())
        .and_then(|()| {
            drop(output);
            fs::rename(&stage, path)
        });
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result.map_err(|e| format!("{}: {e}", path.display()))
}
