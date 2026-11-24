//! Validate the native list workload from its per-drawable Metal presentation records.

use super::{Result, native};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    process::Command,
};

pub(super) const METRIC: &str = "list_100k_scroll_dropped_frames";
pub(super) const TARGET: f64 = 0.0;
pub(super) const FPS_METRIC: &str = "list_100k_scroll_presented_fps";
pub(super) const FPS_TARGET: f64 = 120.0;
const WARMUP_SECONDS: u64 = 2;
const SAMPLE_SECONDS: u64 = 10;
const NS_PER_SECOND: u64 = 1_000_000_000;
const NS_PER_MS: f64 = 1_000_000.0;
const MIN_INTERVAL_NS: u64 = 1_000_000;
const MAX_INTERVAL_NS: u64 = 100_000_000;
const ROWS: u64 = 100_000;
const STRIDE_POINTS: f64 = 12.0;
const LIST_ROW_POINTS: f64 = 34.0;
const REFERENCE_SLOTS: u64 = 1200;
// Use the same timestamp correlation tolerance as the Apple observer.
const PRESENTATION_MATCH_NS: u64 = 200_000;
const VIEWER_ARGS: &[&str] = &["--measure-scroll", "--json"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    drawable: u64,
    submitted_ns: u64,
    refresh_ns: u64,
    first_key: u64,
    offset_y: f64,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    presented_ns: Option<u64>,
    #[serde(default)]
    target_ns: Option<u64>,
    #[serde(default)]
    scene_fingerprint: Option<u64>,
    #[serde(default)]
    paint_sequence: Option<u64>,
    #[serde(default)]
    row_top_y: Option<f64>,
}

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Display {
    id: u32,
    built_in: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScrollReport {
    rows: u64,
    warmup_seconds: u64,
    sample_seconds: u64,
    stride_points: f64,
    pixel_width: u32,
    pixel_height: u32,
    scale_factor: f64,
    phase_ns: u64,
    start_ns: u64,
    end_ns: u64,
    interval_ns: u64,
    refresh_interval_ms: f64,
    expected_slots: u64,
    occupied_slots: u64,
    displayed_frames: usize,
    submitted_frames: usize,
    dropped_frames: u64,
    missing_feedback: usize,
    zero_timestamps: usize,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    first_key: Option<u64>,
    #[serde(deserialize_with = "Deserialize::deserialize")]
    last_key: Option<u64>,
    valid: bool,
    pass: bool,
    frames: Vec<Frame>,
    #[serde(default)]
    display: Option<Display>,
    #[serde(default)]
    display_ticks_ns: Option<Vec<u64>>,
}

pub(super) struct Capture {
    pub dropped: native::Measurement,
    pub fps: native::Measurement,
    pub reference_issues: Vec<&'static str>,
}

pub(super) fn valid_refresh_ms(ms: f64) -> bool {
    (MIN_INTERVAL_NS as f64 / NS_PER_MS..=MAX_INTERVAL_NS as f64 / NS_PER_MS).contains(&ms)
}

pub(super) fn measure(root: &Path, stem: &str) -> Result<Capture> {
    if !cfg!(target_os = "macos") {
        return Err(
            "Native list scrolling requires macOS 14+, WindowServer and a Metal GPU".into(),
        );
    }
    let (relative, directory) = native::scroll_directory(root, stem)?;
    let result = (|| {
        let viewer = native::release_viewer(root, &directory)?;
        let hash = native::executable_sha256(&viewer)?;
        let provenance = serde_json::json!({
            "artifact_directory": relative,
            "executable": native::VIEWER_EXECUTABLE,
            "executable_sha256": hash,
            "arguments": VIEWER_ARGS,
        });
        fs::write(directory.join("provenance.json"), provenance.to_string())
            .map_err(|error| error.to_string())?;
        let mut command = Command::new(&viewer);
        command
            .args(VIEWER_ARGS)
            .env_remove("OXIVIEW_METAL_TIMING_FILE");
        let (raw, code) = native::capture_command(
            root,
            &mut command,
            &directory.join("viewer.json"),
            &directory.join("viewer.log"),
        )?;
        if native::executable_sha256(&viewer)? != hash {
            return Err("The release executable changed during the capture".into());
        }
        let mut capture = parse(&raw, code)?;
        capture.dropped.details["provenance"] = provenance.clone();
        capture.fps.details["provenance"] = provenance;
        let evidence = serde_json::json!({
            "dropped_frames": {
                "value": capture.dropped.value,
                "hard_check_pass": capture.dropped.hard_pass,
                "details": capture.dropped.details,
            },
            "presented_fps": {
                "value": capture.fps.value,
                "hard_check_pass": capture.fps.hard_pass,
                "details": capture.fps.details,
            },
            "target_exit_status": code,
        });
        fs::write(directory.join("result.json"), evidence.to_string())
            .map_err(|error| error.to_string())?;
        Ok(capture)
    })();
    result.map_err(|error: String| format!("{error}; see {}", relative.display()))
}

pub(super) fn parse(text: &str, code: Option<i32>) -> Result<Capture> {
    if !matches!(code, Some(0..=2)) {
        return Err(format!("Native scroll capture failed (exit {code:?})"));
    }
    let report: ScrollReport = serde_json::from_str(text)
        .map_err(|error| format!("Invalid native scroll JSON (exit {code:?}): {error}"))?;
    let capture = report.validate()?;
    if code != Some(i32::from(capture.dropped.hard_pass == Some(false))) {
        return Err(format!(
            "Native scroll capture exit {code:?} disagrees with the drawable records"
        ));
    }
    Ok(capture)
}

impl ScrollReport {
    fn fps(&self, hard_pass: bool, reference_issues: &[&str]) -> native::Measurement {
        let elapsed_ns = self.end_ns - self.start_ns;
        let elapsed = elapsed_ns as f64 / NS_PER_SECOND as f64;
        let display_hz = self.expected_slots as f64 / elapsed;
        native::Measurement {
            value: self.occupied_slots as f64 / elapsed,
            target: Some(FPS_TARGET),
            refresh_interval_ms: Some(self.refresh_interval_ms),
            hard_pass: Some(hard_pass),
            details: serde_json::json!({
                "method": "advancing content in Metal on-screen presentation slots",
                "display_hz": display_hz,
                "measured_duration_ns": elapsed_ns,
                "occupied_slots": self.occupied_slots,
                "expected_slots": self.expected_slots,
                "zero_timestamps": self.zero_timestamps,
                "reference_issues": reference_issues,
            }),
        }
    }

    fn validate(&self) -> Result<Capture> {
        let interval = self.interval_ns;
        let refresh_ms = interval as f64 / NS_PER_MS;
        if !(MIN_INTERVAL_NS..=MAX_INTERVAL_NS).contains(&interval)
            || self.phase_ns == 0
            || (self.refresh_interval_ms - refresh_ms).abs() > 1e-9
        {
            return Err("Invalid native scroll cadence metadata".into());
        }
        if self.rows != ROWS
            || self.warmup_seconds != WARMUP_SECONDS
            || self.sample_seconds != SAMPLE_SECONDS
            || self.stride_points != STRIDE_POINTS
            || !self.scale_factor.is_finite()
            || self.scale_factor <= 0.0
            || (f64::from(self.pixel_width) - 1120.0 * self.scale_factor).abs() > 0.5
            || (f64::from(self.pixel_height) - 720.0 * self.scale_factor).abs() > 0.5
        {
            return Err("Invalid native scroll workload or display geometry".into());
        }

        let warmup = (WARMUP_SECONDS * NS_PER_SECOND).div_ceil(interval) * interval;
        let sample_ns = SAMPLE_SECONDS * NS_PER_SECOND;
        let expected = (sample_ns + interval / 2) / interval;
        let start = self
            .phase_ns
            .checked_add(warmup)
            .ok_or("Scroll start overflow")?;
        let end = start
            .checked_add(expected * interval)
            .ok_or("Scroll end overflow")?;
        let tail = end
            .checked_add(interval)
            .ok_or("Scroll capture tail overflow")?;
        if self.start_ns != start || self.end_ns != end || self.expected_slots != expected {
            return Err("Native scroll bounds or expected slots disagree with the fixed duration and cadence".into());
        }

        if self
            .frames
            .first()
            .is_none_or(|frame| frame.submitted_ns >= start)
            || self
                .frames
                .last()
                .is_none_or(|frame| frame.submitted_ns < tail)
            || self
                .frames
                .windows(2)
                .any(|pair| pair[0].submitted_ns >= pair[1].submitted_ns)
        {
            return Err("Native scroll capture does not span warm-up through the post-sample tail in submission order".into());
        }

        let mut occupied = BTreeSet::new();
        let mut displayed = Vec::new();
        let mut have_phase_anchor = false;
        let mut max_phase_error = 0;
        let mut submitted = 0;
        let mut missing = 0;
        let mut zero = 0;
        for (index, frame) in self.frames.iter().enumerate() {
            // CAMetalLayer drawable IDs begin at zero and increase on each acquisition.
            if (index > 0 && self.frames[index - 1].drawable >= frame.drawable)
                || frame.submitted_ns == 0
                || !(1..=ROWS).contains(&frame.first_key)
                || !frame.offset_y.is_finite()
                || frame.offset_y < 0.0
            {
                return Err(format!(
                    "Invalid native scroll drawable or content at record {index}"
                ));
            }
            if frame.submitted_ns < tail && frame.refresh_ns.abs_diff(interval) > interval / 1000 {
                return Err(format!(
                    "Native scroll refresh interval changed at record {index}"
                ));
            }
            let measured = (start..end).contains(&frame.submitted_ns);
            submitted += usize::from(measured);
            missing += usize::from(frame.submitted_ns < end && frame.presented_ns.is_none());
            zero += usize::from(measured && frame.presented_ns == Some(0));

            let Some(presented) = frame.presented_ns.filter(|&ns| ns != 0) else {
                continue;
            };
            if presented < frame.submitted_ns {
                return Err(format!(
                    "Native scroll drawable was presented before submission at record {index}"
                ));
            }
            have_phase_anchor |= presented == self.phase_ns;
            if presented < tail {
                let remainder = presented.abs_diff(self.phase_ns) % interval;
                let phase_error = remainder.min(interval - remainder);
                max_phase_error = max_phase_error.max(phase_error);
            }
            let first_boundary = start - interval / 2;
            if let Some(delta) = presented.checked_sub(first_boundary) {
                let slot = delta / interval;
                if slot < expected {
                    occupied.insert(slot);
                    displayed.push((presented, index));
                }
            }
        }
        if !have_phase_anchor || missing != 0 || submitted == 0 {
            return Err(format!(
                "Incomplete native scroll capture (phase anchor: {have_phase_anchor}, {missing} missing callbacks before interval end, {submitted} measured submissions)"
            ));
        }

        // Feedback may arrive out of acquisition order. Content must advance in
        // actual presentation order even when several drawables land in one slot.
        displayed.sort_unstable();
        let first_key = displayed
            .first()
            .map(|&(_, index)| self.frames[index].first_key);
        let last_key = displayed
            .last()
            .map(|&(_, index)| self.frames[index].first_key);
        if !first_key
            .zip(last_key)
            .is_some_and(|(first, last)| first < last)
            || displayed.windows(2).any(|pair| {
                let first = &self.frames[pair[0].1];
                let next = &self.frames[pair[1].1];
                let steps = (next.offset_y - first.offset_y) / STRIDE_POINTS;
                steps < 1.0
                    || (steps - steps.round()).abs() > 1e-9
                    || next.first_key < first.first_key
            })
        {
            return Err("Native scroll did not advance the displayed content in order".into());
        }

        let missing_slots: Vec<_> = (0..expected)
            .filter(|slot| !occupied.contains(slot))
            .collect();
        let dropped = missing_slots.len() as u64;
        let occupied_count = expected - dropped;
        let hard_pass = dropped == 0 && zero == 0;
        if self.occupied_slots != occupied_count
            || self.displayed_frames != displayed.len()
            || self.submitted_frames != submitted
            || self.dropped_frames != dropped
            || self.missing_feedback != 0
            || self.zero_timestamps != zero
            || self.first_key != first_key
            || self.last_key != last_key
            || !self.valid
            || self.pass != hard_pass
        {
            return Err(format!(
                "Native scroll summary disagrees with drawable records (expected {expected} slots, occupied {occupied_count}, dropped {dropped}, {zero} zero timestamps)"
            ));
        }

        let mut reference_issues = Vec::new();
        if expected != REFERENCE_SLOTS {
            reference_issues.push("The sample did not contain exactly 1,200 reference slots");
        }
        if self.scale_factor != 2.0 || self.pixel_width != 2240 || self.pixel_height != 1440 {
            reference_issues.push("The reference geometry is 2240 × 1440 pixels at 2× scale");
        }
        if max_phase_error > interval / 4 {
            reference_issues.push("Presentations exceed the fixed cadence phase tolerance");
        }
        if displayed.len() as u64 != occupied_count {
            reference_issues.push("Multiple drawables were reported in one display slot");
        }
        if displayed.iter().any(|&(_, index)| {
            let frame = &self.frames[index];
            frame.offset_y >= ROWS as f64 * LIST_ROW_POINTS
                || frame.first_key != 1 + (frame.offset_y / LIST_ROW_POINTS) as u64
                || frame
                    .paint_sequence
                    .is_none_or(|step| step == 0 || frame.offset_y != step as f64 * STRIDE_POINTS)
                || frame.row_top_y
                    != Some((frame.first_key - 1) as f64 * LIST_ROW_POINTS - frame.offset_y)
        }) {
            reference_issues.push(
                "The painted row geometry or displacement does not match the fixed scroll sequence",
            );
        }
        if hard_pass
            && displayed.windows(2).any(|pair| {
                self.frames[pair[1].1].offset_y - self.frames[pair[0].1].offset_y != STRIDE_POINTS
            })
        {
            reference_issues
                .push("A clean presentation sequence must advance by exactly 12 points per slot");
        }
        if !self
            .display
            .as_ref()
            .is_some_and(|display| display.id != 0 && display.built_in)
        {
            reference_issues.push("A stable built-in display identity is required");
        }
        let (clock_covers_sample, clock_details) = self.inspect_clock(tail, &displayed, hard_pass);
        if !clock_covers_sample {
            reference_issues
                .push("Raw OS display targets do not corroborate every reference opportunity");
        }
        let mut scenes = BTreeSet::new();
        if displayed.iter().any(|&(_, index)| {
            self.frames[index]
                .scene_fingerprint
                .is_none_or(|scene| !scenes.insert(scene))
        }) {
            reference_issues.push("The displayed collection scene is missing or repeats");
        }
        let mut fps = self.fps(hard_pass && reference_issues.is_empty(), &reference_issues);
        fps.details["display_clock"] = clock_details;
        let mut per_second = [0_u64; SAMPLE_SECONDS as usize];
        for slot in &occupied {
            if let Some(bucket) = per_second.get_mut((slot * SAMPLE_SECONDS / expected) as usize) {
                *bucket += 1;
            }
        }
        fps.details["occupied_slots_per_second"] = serde_json::json!(per_second);
        let longest_gap = displayed.windows(2).map(|pair| pair[1].0 - pair[0].0).max();
        fps.details["maximum_display_interval_ms"] =
            serde_json::json!(longest_gap.map(|ns| ns as f64 / NS_PER_MS));
        let dropped = native::Measurement {
            value: dropped as f64,
            target: Some(TARGET),
            refresh_interval_ms: Some(refresh_ms),
            hard_pass: Some(hard_pass),
            details: serde_json::json!({
                "rows": self.rows, "stride_points": self.stride_points,
                "pixel_width": self.pixel_width, "pixel_height": self.pixel_height,
                "scale_factor": self.scale_factor, "warmup_seconds": self.warmup_seconds,
                "sample_seconds": self.sample_seconds, "measured_duration_ns": end - start,
                "phase_ns": self.phase_ns, "start_ns": start, "end_ns": end,
                "interval_ns": interval, "refresh_interval_ms": refresh_ms,
                "max_phase_error_ns": max_phase_error,
                "expected_slots": expected, "occupied_slots": occupied_count,
                "displayed_frames": displayed.len(), "submitted_frames": submitted,
                "dropped_frames": dropped, "missing_slots": missing_slots,
                "missing_feedback": missing, "zero_timestamps": zero,
                "first_key": first_key, "last_key": last_key,
                "display": self.display, "display_ticks": self.display_ticks_ns.as_ref().map(Vec::len),
                "reference_issues": reference_issues
            }),
        };
        Ok(Capture {
            dropped,
            fps,
            reference_issues,
        })
    }

    fn inspect_clock(&self, tail: u64, displayed: &[(u64, usize)], clean: bool) -> (bool, Value) {
        let Some(ticks) = &self.display_ticks_ns else {
            return (false, Value::Null);
        };
        if ticks.is_empty() || ticks.windows(2).any(|pair| pair[0] >= pair[1]) {
            return (false, Value::Null);
        }

        let interval = self.interval_ns;
        let tolerance = interval / 4;
        let warm_mid = self.phase_ns + (self.start_ns - self.phase_ns) / 2;
        let warm_frame = |from, to| {
            self.frames
                .iter()
                .any(|f| f.presented_ns.is_some_and(|at| at > from && at < to))
        };
        // Warm-up and tail prove the clock was running across both edges.
        let mut corroborates_sample = ticks
            .first()
            .is_some_and(|&at| at <= self.phase_ns + tolerance)
            && ticks.last().is_some_and(|&at| at >= tail - tolerance)
            && warm_frame(self.phase_ns, warm_mid)
            && warm_frame(warm_mid, self.start_ns);

        let first_boundary = self.start_ns - interval / 2;
        let begin = ticks.partition_point(|&tick| tick < self.start_ns - tolerance);
        let end = ticks.partition_point(|&tick| tick <= self.end_ns + tolerance);
        let active = &ticks[begin..end];
        corroborates_sample &= active.len() as u64 == self.expected_slots + 1;
        let mut covered_nominal_slots = 0_u64;
        let mut last_covered_slot = None;
        for (index, &tick) in active.iter().enumerate() {
            let slot = (tick - first_boundary) / interval;
            let on_phase = tick.abs_diff(self.start_ns + slot * interval) <= tolerance;
            corroborates_sample &= on_phase && slot == index as u64;
            if index > 0 {
                corroborates_sample &=
                    (tick - active[index - 1]).abs_diff(interval) <= 2 * PRESENTATION_MATCH_NS;
            }
            // The closing tick corroborates the sample edge. Count the slots inside it.
            if on_phase && slot < self.expected_slots && last_covered_slot != Some(slot) {
                covered_nominal_slots += 1;
                last_covered_slot = Some(slot);
            }
        }

        let mut previous_source_index = None;
        let mut matched_presentations = 0_usize;
        let mut measured_offsets = 0;
        let mut tick_offsets = BTreeMap::<_, usize>::new();
        for &(presented, index) in displayed {
            let source_index = self.frames[index]
                .target_ns
                .and_then(|target| ticks.binary_search(&target).ok());
            let slot = (presented - first_boundary) / interval;
            corroborates_sample &= active.get(slot as usize).zip(source_index).is_some_and(
                |(&opportunity, source)| {
                    let target = ticks[source];
                    presented.abs_diff(opportunity) <= PRESENTATION_MATCH_NS
                        && previous_source_index.is_none_or(|previous| source > previous)
                        && target <= opportunity
                        && (!clean || target == opportunity)
                },
            );
            previous_source_index = source_index;

            // Match feedback by timestamp so an absent clock tick cannot shift later matches.
            let next = ticks.partition_point(|&tick| tick < presented);
            let before = next
                .checked_sub(1)
                .map_or(u64::MAX, |index| presented - ticks[index]);
            let after = ticks.get(next).map_or(u64::MAX, |&tick| tick - presented);
            if before.min(after) <= PRESENTATION_MATCH_NS {
                matched_presentations += 1;
                // A unique nearest tick supplies the displayed index for the displacement.
                if let Some(source) = source_index
                    && before != after
                {
                    let displayed_index = if before < after { next - 1 } else { next };
                    let displacement = displayed_index as i64 - source as i64;
                    *tick_offsets.entry(displacement).or_default() += 1;
                    measured_offsets += 1;
                }
            }
        }

        (
            corroborates_sample,
            serde_json::json!({
                "covered_nominal_slots": covered_nominal_slots,
                "positive_presentations": displayed.len(),
                "matched_presentations": matched_presentations,
                "source_to_display_tick_offsets": tick_offsets,
                "unmeasured_tick_offsets": displayed.len() - measured_offsets,
            }),
        )
    }
}

#[cfg(test)]
#[path = "scroll_tests.rs"]
pub(super) mod tests;
