use super::*;
use serde_json::{Value, json};

const INTERVAL: u64 = 16_666_667;
const PHASE: u64 = 100_000_000_000;
const WARMUP_SLOTS: u64 = 120;
const SLOTS: u64 = 600;
const START: u64 = PHASE + WARMUP_SLOTS * INTERVAL;
const END: u64 = START + SLOTS * INTERVAL;
const SUBMIT_LEAD: u64 = 1_000_000;
const FIRST: usize = WARMUP_SLOTS as usize;

fn parse_drop(text: &str, code: Option<i32>) -> Result<native::Measurement> {
    parse(text, code).map(|capture| capture.dropped)
}

pub(in crate::bench) fn fixture(dropped_slot: Option<u64>) -> Value {
    at_interval(dropped_slot, INTERVAL)
}

fn at_interval(dropped_slot: Option<u64>, interval: u64) -> Value {
    let warmup_slots = (2 * NS_PER_SECOND).div_ceil(interval);
    let slots = (10 * NS_PER_SECOND + interval / 2) / interval;
    let start = PHASE + warmup_slots * interval;
    let end = start + slots * interval;
    let dropped_index = dropped_slot.map(|slot| warmup_slots + slot);
    let key = |index| ((index + 1) * 12) / 34 + 1;
    let ticks: Vec<_> = (0..warmup_slots + slots + 4)
        .map(|index| PHASE + index * interval)
        .collect();
    let frames: Vec<_> = ticks
        .iter()
        .copied()
        .enumerate()
        .map(|(index, presented)| (index as u64, presented))
        .filter(|&(index, _)| Some(index) != dropped_index)
        .map(|(index, presented)| {
            json!({
                "drawable": 2 * index, "submitted_ns": presented - SUBMIT_LEAD,
                "refresh_ns": interval, "first_key": key(index),
                "offset_y": (index + 1) * 12, "presented_ns": presented,
                "target_ns": presented, "scene_fingerprint": index + 1, "paint_sequence": index + 1,
                "row_top_y": -((((index + 1) * 12) % 34) as f64),
            })
        })
        .collect();
    let dropped = u64::from(dropped_slot.is_some());
    let first = warmup_slots + u64::from(dropped_slot == Some(0));
    let last = warmup_slots + slots - 1 - u64::from(dropped_slot == Some(slots - 1));
    json!({
        "rows": 100000, "warmup_seconds": 2, "sample_seconds": 10, "stride_points": 12,
        "pixel_width": 2240, "pixel_height": 1440, "scale_factor": 2.0,
        "phase_ns": PHASE, "start_ns": start, "end_ns": end, "interval_ns": interval,
        "refresh_interval_ms": interval as f64 / NS_PER_MS, "expected_slots": slots, "occupied_slots": slots - dropped,
        "displayed_frames": slots - dropped,
        "submitted_frames": slots - u64::from(dropped_slot.is_some_and(|slot| slot > 0)),
        "dropped_frames": dropped, "missing_feedback": 0, "zero_timestamps": 0,
        "first_key": key(first), "last_key": key(last), "valid": true, "pass": dropped == 0,
        "frames": frames, "display": {"id": 1, "built_in": true}, "display_ticks_ns": ticks,
    })
}

#[test]
fn legacy_and_incomplete_reference_evidence_cannot_certify_120_hz() {
    let mut legacy = fixture_120(None);
    let object = legacy.as_object_mut().expect("report");
    object.remove("display");
    object.remove("display_ticks_ns");
    for frame in object["frames"].as_array_mut().expect("frames") {
        let frame = frame.as_object_mut().expect("frame");
        frame.remove("target_ns");
        frame.remove("scene_fingerprint");
    }
    let capture = parse(&legacy.to_string(), Some(0)).expect("historical evidence");
    assert_eq!(capture.dropped.value, 0.0);
    assert_eq!(capture.dropped.hard_pass, Some(true));
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(capture.fps.details.get("display_clock"), Some(&Value::Null));
    let mut raw = fixture_120(None);
    raw["display"]["built_in"] = json!(false);
    let capture = parse(&raw.to_string(), Some(0)).expect("diagnostic evidence");
    assert_eq!(capture.fps.hard_pass, Some(false));
    let mut raw = fixture_120(None);
    raw["display_ticks_ns"]
        .as_array_mut()
        .expect("ticks")
        .remove(301);
    let capture = parse(&raw.to_string(), Some(0)).expect("missing tick");
    assert_eq!(capture.dropped.hard_pass, Some(true));
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(
        capture.fps.details["display_clock"],
        json!({
            "covered_nominal_slots": 1199,
            "positive_presentations": 1200,
            "matched_presentations": 1199,
            "source_to_display_tick_offsets": {"0": 1199},
            "unmeasured_tick_offsets": 1,
        })
    );
    let mut raw = fixture_120(None);
    raw["display_ticks_ns"]
        .as_array_mut()
        .expect("ticks")
        .swap(301, 302);
    let capture = parse(&raw.to_string(), Some(0)).expect("unordered clock");
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(capture.fps.details.get("display_clock"), Some(&Value::Null));
    let mut raw = fixture_120(None);
    raw["frames"][301]["scene_fingerprint"] = raw["frames"][300]["scene_fingerprint"].clone();
    assert_eq!(
        parse(&raw.to_string(), Some(0))
            .expect("repeated scene")
            .fps
            .hard_pass,
        Some(false)
    );
}

#[test]
fn a_warmup_clock_gap_does_not_change_the_measured_slots() {
    let mut raw = fixture_120(None);
    // These three callbacks precede the sample by more than a second.
    // The real display clock still brackets the warm-up and the post-sample tail.
    raw["display_ticks_ns"]
        .as_array_mut()
        .expect("clock")
        .drain(26..29);
    assert_eq!(
        parse(&raw.to_string(), Some(0))
            .expect("full measured scroll")
            .fps
            .hard_pass,
        Some(true)
    );

    let start = raw["start_ns"].as_u64().expect("start");
    raw["display_ticks_ns"]
        .as_array_mut()
        .expect("clock")
        .retain(|tick| tick.as_u64() != Some(start));
    assert_eq!(
        parse(&raw.to_string(), Some(0))
            .expect("missing first opportunity")
            .fps
            .hard_pass,
        Some(false)
    );
}

#[test]
fn the_scene_witness_matches_the_painted_row_and_applied_distance() {
    let source = fixture_120(None);
    let full = parse(&source.to_string(), Some(0)).expect("reference frame sequence");
    assert_eq!(
        full.fps.details["occupied_slots_per_second"],
        json!(vec![120; 10])
    );
    for (field, value) in [("row_top_y", json!(99.0)), ("paint_sequence", json!(12))] {
        let mut stale = source.clone();
        stale["frames"][301][field] = value;
        let capture = parse(&stale.to_string(), Some(0)).expect("read incomplete evidence");
        assert_eq!(capture.fps.hard_pass, Some(false), "{field}");
    }
}

pub(in crate::bench) fn fixture_120(dropped_slot: Option<u64>) -> Value {
    at_interval(dropped_slot, 8_333_333)
}

#[test]
fn actual_feedback_counts_both_edges_and_internal_gaps() {
    let pass = parse_drop(&fixture(None).to_string(), Some(0)).expect("complete evidence");
    assert_eq!(pass.value, 0.0);
    assert_eq!(pass.hard_pass, Some(true));
    assert_eq!(pass.details["measured_duration_ns"], SLOTS * INTERVAL);
    assert_eq!(pass.details["occupied_slots"], SLOTS);
    for slot in [0, 217, SLOTS - 1] {
        let report = fixture(Some(slot));
        let fail = parse_drop(&report.to_string(), Some(1)).expect("valid measured miss");
        assert_eq!(fail.value, 1.0);
        assert_eq!(fail.hard_pass, Some(false));
        assert_eq!(fail.details["missing_slots"], json!([slot]));
        assert!(parse_drop(&report.to_string(), Some(0)).is_err());
    }
}

#[test]
fn a_duplicate_slot_cannot_fill_a_gap_and_zero_is_a_hard_miss() {
    let mut report = fixture(None);
    report["frames"][FIRST + 1]["presented_ns"] = json!(START + 2 * INTERVAL - 1);
    report["occupied_slots"] = json!(SLOTS - 1);
    report["dropped_frames"] = json!(1);
    report["pass"] = json!(false);
    let measured = parse_drop(&report.to_string(), Some(1)).expect("duplicate refresh slot");
    assert_eq!(measured.details["displayed_frames"], SLOTS);
    assert_eq!(measured.details["missing_slots"], json!([1]));

    // An extra submitted drawable failed to display, despite every slot being covered.
    let mut report = fixture(None);
    let mut zeroed = report["frames"][FIRST + 1].clone();
    zeroed["drawable"] = json!(2 * (FIRST + 1) + 1);
    zeroed["submitted_ns"] = json!(START + INTERVAL);
    zeroed["presented_ns"] = json!(0);
    report["frames"]
        .as_array_mut()
        .expect("raw frames")
        .insert(FIRST + 2, zeroed);
    report["submitted_frames"] = json!(SLOTS + 1);
    report["zero_timestamps"] = json!(1);
    report["pass"] = json!(false);
    let measured = parse_drop(&report.to_string(), Some(1)).expect("observed zero Metal timestamp");
    assert_eq!(measured.value, 0.0);
    assert_eq!(measured.hard_pass, Some(false));
}

#[test]
fn invalid_feedback_content_cadence_and_summary_cannot_pass() {
    let frame = FIRST + 60;
    for (field, value) in [
        ("presented_ns", Value::Null),
        ("presented_ns", json!(START + 59 * INTERVAL)),
        ("refresh_ns", json!(2 * INTERVAL)),
        ("offset_y", json!(0)),
        ("offset_y", json!((frame + 1) * 12 + 1)),
        ("first_key", json!(1)),
        ("first_key", json!(100001)),
        ("drawable", json!(0)),
    ] {
        let mut report = fixture(None);
        report["frames"][frame][field] = value;
        assert!(parse_drop(&report.to_string(), Some(0)).is_err(), "{field}");
    }

    let mut report = fixture(None);
    report["frames"][0]["presented_ns"] = json!(PHASE + 1000);
    assert!(
        parse_drop(&report.to_string(), Some(0))
            .err()
            .expect("missing calibration")
            .contains("phase anchor: false")
    );

    let mut report = fixture(None);
    report["frames"]
        .as_array_mut()
        .expect("raw frames")
        .truncate(FIRST + 350);
    assert!(parse_drop(&report.to_string(), Some(0)).is_err());

    let mut report = fixture(None);
    report["frames"][0]
        .as_object_mut()
        .expect("raw frame")
        .remove("presented_ns");
    assert!(parse_drop(&report.to_string(), Some(0)).is_err());

    for (field, value) in [
        ("rows", json!(99999)),
        ("stride_points", json!(1)),
        ("pixel_height", json!(1000)),
        ("scale_factor", json!(0)),
        ("sample_seconds", json!(9)),
        ("warmup_seconds", json!(1)),
        ("refresh_interval_ms", json!(8.333333)),
        ("end_ns", json!(END - INTERVAL)),
        ("start_ns", json!(START + INTERVAL)),
        ("expected_slots", json!(SLOTS - 1)),
        ("occupied_slots", json!(SLOTS - 1)),
        ("displayed_frames", json!(SLOTS - 1)),
        ("submitted_frames", json!(SLOTS - 1)),
        ("zero_timestamps", json!(1)),
        ("valid", json!(false)),
        ("pass", json!(false)),
    ] {
        let mut report = fixture(None);
        report[field] = value;
        assert!(parse_drop(&report.to_string(), Some(0)).is_err(), "{field}");
    }

    let forged = {
        let mut report = fixture(Some(42));
        report["pass"] = json!(true);
        report["dropped_frames"] = json!(0);
        report["occupied_slots"] = json!(SLOTS);
        report["displayed_frames"] = json!(SLOTS);
        report["submitted_frames"] = json!(SLOTS);
        report
    };
    assert!(
        parse_drop(&forged.to_string(), Some(0))
            .err()
            .expect("reject forged pass")
            .contains("dropped 1")
    );
    for code in [Some(1), Some(2), Some(101), None] {
        assert!(parse_drop(&fixture(None).to_string(), code).is_err());
    }
}

#[test]
fn bad_phase_is_retained_in_the_diagnostic_capture() {
    let mut report = fixture_120(None);
    let frame = &mut report["frames"][300]["presented_ns"];
    *frame = json!(frame.as_u64().expect("timestamp") + 3_000_000);
    let capture = parse(&report.to_string(), Some(0)).expect("diagnostic capture");
    assert_eq!(capture.dropped.value, 0.0);
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert!(
        capture
            .reference_issues
            .iter()
            .any(|issue| issue.contains("phase"))
    );
}

#[test]
fn stable_120_fps_requires_the_display_rate_and_every_slot() {
    for (report, code, passed, fps) in [
        (fixture_120(None), 0, true, 120.0),
        (fixture(None), 0, false, 60.0),
        (at_interval(None, 8_340_284), 0, false, 119.9),
        (fixture_120(Some(22)), 1, false, 119.9),
    ] {
        let capture = parse(&report.to_string(), Some(code)).expect("valid Metal feedback");
        assert_eq!(capture.fps.hard_pass, Some(passed));
        assert!((capture.fps.value - fps).abs() < 1e-4);
        let presented = &report["displayed_frames"];
        assert_eq!(
            capture.fps.details["display_clock"],
            json!({
                "covered_nominal_slots": report["expected_slots"],
                "positive_presentations": presented,
                "matched_presentations": presented,
                "source_to_display_tick_offsets": {"0": presented},
                "unmeasured_tick_offsets": 0,
            })
        );
    }
}

#[test]
fn a_full_sample_can_display_three_ticks_after_its_source_targets() {
    let mut raw = fixture_120(None);
    let interval = raw["interval_ns"].as_u64().expect("interval");
    let frames = raw["frames"].as_array_mut().expect("frames");
    for frame in &mut frames[241..1441] {
        let presented = frame["presented_ns"]
            .as_u64()
            .expect("positive presentation");
        frame["target_ns"] = json!(presented - 3 * interval);
    }
    let presented = frames[301]["presented_ns"].as_u64().expect("presentation");
    frames[301]["presented_ns"] = json!(presented + PRESENTATION_MATCH_NS);

    let capture = parse(&raw.to_string(), Some(0)).expect("complete Metal feedback");
    assert_eq!(capture.dropped.hard_pass, Some(true));
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(
        capture.reference_issues,
        ["Raw OS display targets do not corroborate every reference opportunity"]
    );
    assert_eq!(
        capture.fps.details["display_clock"],
        json!({
            "covered_nominal_slots": 1200,
            "positive_presentations": 1200,
            "matched_presentations": 1200,
            "source_to_display_tick_offsets": {"3": 1200},
            "unmeasured_tick_offsets": 0,
        })
    );
}

#[test]
fn unrecorded_source_targets_leave_offsets_unmeasured() {
    let mut raw = fixture_120(None);
    let frames = raw["frames"].as_array_mut().expect("frames");
    frames[300]
        .as_object_mut()
        .expect("frame")
        .remove("target_ns");
    let target = frames[301]["target_ns"].as_u64().expect("source target");
    frames[301]["target_ns"] = json!(target + 1);

    let capture = parse(&raw.to_string(), Some(0)).expect("complete Metal feedback");
    assert_eq!(capture.dropped.hard_pass, Some(true));
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(
        capture.fps.details["display_clock"],
        json!({
            "covered_nominal_slots": 1200,
            "positive_presentations": 1200,
            "matched_presentations": 1200,
            "source_to_display_tick_offsets": {"0": 1198},
            "unmeasured_tick_offsets": 2,
        })
    );
}

#[test]
fn equidistant_display_ticks_leave_the_offset_unmeasured() {
    let mut raw = fixture_120(None);
    let frame = &mut raw["frames"][301];
    let tick = frame["presented_ns"].as_u64().expect("presentation");
    frame["presented_ns"] = json!(tick + PRESENTATION_MATCH_NS);
    raw["display_ticks_ns"]
        .as_array_mut()
        .expect("ticks")
        .insert(302, json!(tick + 2 * PRESENTATION_MATCH_NS));

    let capture = parse(&raw.to_string(), Some(0)).expect("complete Metal feedback");
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert_eq!(
        capture.fps.details["display_clock"],
        json!({
            "covered_nominal_slots": 1200,
            "positive_presentations": 1200,
            "matched_presentations": 1200,
            "source_to_display_tick_offsets": {"0": 1199},
            "unmeasured_tick_offsets": 1,
        })
    );
}

#[test]
fn physical_geometry_and_exact_visible_content_are_required() {
    let mut wrong_scale = fixture_120(None);
    wrong_scale["scale_factor"] = json!(1);
    wrong_scale["pixel_width"] = json!(1120);
    wrong_scale["pixel_height"] = json!(720);
    let mut wrong_step = fixture_120(None);
    for frame in wrong_step["frames"].as_array_mut().expect("frames") {
        let offset = frame["offset_y"].as_u64().expect("offset") * 2;
        frame["offset_y"] = json!(offset);
        frame["first_key"] = json!(offset / 34 + 1);
    }
    wrong_step["first_key"] = wrong_step["frames"][241]["first_key"].clone();
    wrong_step["last_key"] = wrong_step["frames"][1440]["first_key"].clone();
    let mut wrong_row = fixture_120(None);
    let first = wrong_row["first_key"].clone();
    for frame in &mut wrong_row["frames"].as_array_mut().expect("frames")[242..1440] {
        frame["first_key"] = first.clone();
    }
    for raw in [wrong_scale, wrong_step, wrong_row] {
        let capture = parse(&raw.to_string(), Some(0)).expect("diagnostic capture");
        assert_eq!(capture.fps.hard_pass, Some(false));
    }
}

#[test]
fn occupied_nominal_bins_do_not_hide_an_irregular_presentation_clock() {
    let mut shifted = fixture_120(None);
    let start = 241;
    let end = start + REFERENCE_SLOTS as usize;
    for (i, frame) in shifted["frames"].as_array_mut().expect("frames")[start..end]
        .iter_mut()
        .enumerate()
    {
        let at = frame["presented_ns"].as_u64().expect("timestamp");
        let submitted = frame["submitted_ns"].as_u64().expect("submission");
        frame["submitted_ns"] = json!(submitted - 1_000_000);
        frame["presented_ns"] = json!(if i % 2 == 0 {
            at + 1_900_000
        } else {
            at - 1_900_000
        });
    }
    let capture = parse(&shifted.to_string(), Some(0)).expect("diagnostic capture");
    assert_eq!(capture.dropped.details["occupied_slots"], 1200);
    assert_eq!(capture.fps.hard_pass, Some(false));
    for i in start..end {
        let at = shifted["frames"][i]["presented_ns"].clone();
        shifted["frames"][i]["target_ns"] = at.clone();
        shifted["display_ticks_ns"][i] = at;
    }
    let capture = parse(&shifted.to_string(), Some(0)).expect("diagnostic capture");
    assert_eq!(capture.fps.hard_pass, Some(false));
}

#[test]
fn an_extra_drawable_at_an_occupied_timestamp_invalidates_the_reference_run() {
    let mut raw = fixture_120(None);
    let mut duplicate = raw["frames"][241].clone();
    // Preserve acquisition order and ordinary advancing logical content while
    // reporting two different drawables at exactly the same host timestamp.
    duplicate["drawable"] = json!(duplicate["drawable"].as_u64().expect("drawable") + 1);
    duplicate["submitted_ns"] = json!(duplicate["submitted_ns"].as_u64().expect("submitted") + 1);
    let frames = raw["frames"].as_array_mut().expect("frames");
    frames.insert(242, duplicate);
    for frame in &mut frames[242..] {
        let offset = frame["offset_y"].as_u64().expect("offset") + 12;
        frame["offset_y"] = json!(offset);
        frame["first_key"] = json!(offset / 34 + 1);
    }
    raw["last_key"] = frames[1441]["first_key"].clone();
    raw["displayed_frames"] = json!(1201);
    let capture = parse(&raw.to_string(), Some(0)).expect("diagnostic capture");
    assert_eq!(capture.fps.hard_pass, Some(false));
    assert!(
        capture
            .reference_issues
            .iter()
            .any(|issue| issue.contains("Multiple drawables"))
    );
}
