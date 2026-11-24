use super::*;

fn survey() -> (Value, Value) {
    let raw = scroll::tests::fixture_120(None);
    let observer = observation_for(&raw);
    (raw, observer)
}

fn observation_for(raw: &Value) -> Value {
    let epoch = 99_000_000_000;
    let start = raw["start_ns"].as_u64().expect("start") - epoch;
    let end = raw["end_ns"].as_u64().expect("end") - epoch;
    let count = raw["displayed_frames"].as_u64().expect("presented frames");
    let dropped = raw["dropped_frames"].as_u64().expect("dropped frames");
    let expected = raw["expected_slots"].as_u64().expect("expected slots");
    json!({
        "method": "Apple Instruments Animation Hitches", "app_pid": 123, "display": "Display 1",
        "surface_display": "Built-In Display 1", "trace_health": "no_loss_reported",
        "host_origin_ns": epoch, "sample_start_trace_ns": start, "sample_end_trace_ns": end,
        "active_scroll_duration_ns": end - start, "measured_metal_presentations": count,
        "metal_reported_empty_slots": dropped, "metal_missed_slot_ms_per_second": 1000.0 * dropped as f64 / expected as f64,
        "apple_frame_lifetime_matches": count, "conflicting_apple_lifetimes": 0,
        "app_tagged_surface_matches": count, "app_surface_matches_bracket_sample": true,
        "unmatched_app_hitches": 0,
        "correlated_app_hitches": [], "observed_hitch_time_ns": 0,
        "observed_hitch_ms_per_second": 0.0, "complete_apple_frame_lifetime_coverage": true,
        "threshold_ms_per_second": 1.2, "hard_check_pass": null, "target_exit_status": i32::from(raw["pass"] == false),
    })
}

fn validate_fixture(raw: &Value, observer: &Value) -> Result<(Status, f64, Value)> {
    validate(observer.to_string().as_bytes(), &raw.to_string())
}

#[test]
fn complete_coverage_is_required_to_pass() {
    let (raw, mut observer) = survey();
    assert_eq!(
        validate_fixture(&raw, &observer).expect("complete").0,
        Status::Pass
    );

    observer["apple_frame_lifetime_matches"] = json!(1199);
    observer["app_tagged_surface_matches"] = json!(1199);
    assert!(validate_fixture(&raw, &observer).is_err());
    observer["complete_apple_frame_lifetime_coverage"] = json!(false);
    observer["hard_check_pass"] = Value::Null;
    assert_eq!(
        validate_fixture(&raw, &observer).expect("incomplete").0,
        Status::Error
    );
    observer["app_tagged_surface_matches"] = json!(0);
    assert_eq!(
        validate_fixture(&raw, &observer).expect("inconclusive").0,
        Status::Error
    );
}

#[test]
fn clipped_and_overlapping_hitches_establish_a_lower_bound() {
    let (raw, mut observer) = survey();
    let start = observer["sample_start_trace_ns"].as_u64().expect("start");
    let elapsed = observer["active_scroll_duration_ns"]
        .as_u64()
        .expect("elapsed");
    observer["correlated_app_hitches"] = json!([
        {"trace_start_ns": -5_000_000, "duration_ns": start + 20_000_000, "swap_id": 123, "surface_id": 9},
        {"trace_start_ns": -5_000_000, "duration_ns": start + 20_000_000, "swap_id": 123, "surface_id": 9},
        {"trace_start_ns": start + 10_000_000, "duration_ns": 10_000_000, "swap_id": 125, "surface_id": 16},
    ]);
    observer["observed_hitch_time_ns"] = json!(20_000_000);
    observer["observed_hitch_ms_per_second"] = json!(20_000_000_000.0 / elapsed as f64);
    observer["apple_frame_lifetime_matches"] = json!(1199);
    observer["app_tagged_surface_matches"] = json!(1000);
    observer["complete_apple_frame_lifetime_coverage"] = json!(false);
    observer["hard_check_pass"] = json!(false);
    assert_eq!(
        validate_fixture(&raw, &observer)
            .expect("observed failure")
            .0,
        Status::Fail
    );
    observer["hard_check_pass"] = json!(true);
    assert!(validate_fixture(&raw, &observer).is_err());
}

#[test]
fn rejects_mismatched_raw_data_timebase_and_exit_code() {
    let (raw, observer) = survey();
    for (key, value) in [
        ("target_exit_status", json!(1)),
        ("host_origin_ns", json!(0)),
        ("sample_start_trace_ns", json!(0)),
        ("observed_hitch_ms_per_second", json!(0.1)),
        ("measured_metal_presentations", json!(1100)),
        ("method", json!("Metal subset")),
    ] {
        let mut corrupted = observer.clone();
        corrupted[key] = value;
        assert!(validate_fixture(&raw, &corrupted).is_err(), "{key}");
    }
}

#[test]
fn a_below_budget_apple_value_cannot_hide_a_metal_stall() {
    let raw = scroll::tests::fixture(Some(20));
    let mut observer = observation_for(&raw);

    let (status, ratio, _) = validate_fixture(&raw, &observer).expect("valid evidence");
    assert_eq!(status, Status::Error);
    assert_eq!(ratio, 0.0);
    observer["hard_check_pass"] = json!(true);
    assert!(validate_fixture(&raw, &observer).is_err());
}

#[test]
fn selects_the_actual_cargo_binary_path() {
    let binary = std::env::current_dir()
        .expect("absolute fixture directory")
        .join("custom-target/release/log-viewer-oxi");
    let line = json!({
        "reason": "compiler-artifact", "target": {"name": "log-viewer-oxi", "kind": ["bin"]},
        "executable": binary,
    });
    let messages = format!("{{\"reason\":\"compiler-message\"}}\n{line}\n");
    assert_eq!(
        native::viewer_executable(messages.as_bytes()).expect("viewer"),
        binary
    );
    assert!(native::viewer_executable(format!("{messages}{line}").as_bytes()).is_err());
}

#[test]
fn phase_mismatch_does_not_erase_proven_hitch_time() {
    let (mut raw, mut observer) = survey();
    let frame = &mut raw["frames"][300]["presented_ns"];
    *frame = json!(frame.as_u64().expect("timestamp") + 3_000_000);
    assert_eq!(
        validate_fixture(&raw, &observer).expect("inconclusive").0,
        Status::Error
    );
    let start = observer["sample_start_trace_ns"].as_u64().expect("start");
    let elapsed = observer["active_scroll_duration_ns"]
        .as_u64()
        .expect("elapsed");
    observer["correlated_app_hitches"] = json!([
        {"trace_start_ns": start, "duration_ns": 20_000_000, "swap_id": 123, "surface_id": 9},
    ]);
    observer["observed_hitch_time_ns"] = json!(20_000_000);
    observer["observed_hitch_ms_per_second"] = json!(20_000_000_000.0 / elapsed as f64);
    observer["hard_check_pass"] = json!(false);
    let (status, _, details) = validate_fixture(&raw, &observer).expect("above budget");
    assert_eq!(status, Status::Fail);
    assert!(
        details["metal_reference_issues"]
            .to_string()
            .contains("phase")
    );
}

#[test]
fn a_quiet_trace_requires_reference_rate_display_and_trace_health() {
    let raw = scroll::tests::fixture(None);
    assert_eq!(
        validate_fixture(&raw, &observation_for(&raw))
            .expect("60 Hz")
            .0,
        Status::Error
    );
    let (raw, observer) = survey();
    for (key, value) in [
        ("trace_health", json!("loss_reported")),
        ("app_surface_matches_bracket_sample", json!(false)),
        ("surface_display", json!("external-1")),
        ("unmatched_app_hitches", json!(1)),
        ("conflicting_apple_lifetimes", json!(1)),
    ] {
        let mut mutated = observer.clone();
        mutated[key] = value;
        mutated["complete_apple_frame_lifetime_coverage"] = json!(false);
        assert_eq!(
            validate_fixture(&raw, &mutated).expect("inconclusive").0,
            Status::Error
        );
    }
}

#[test]
fn one_empty_slot_can_be_below_the_separate_apple_budget() {
    let raw = scroll::tests::fixture_120(Some(20));
    let native = scroll::parse(&raw.to_string(), Some(1)).expect("native capture");
    assert_eq!(native.fps.hard_pass, Some(false));
    let (status, ratio, _) = validate_fixture(&raw, &observation_for(&raw)).expect("valid trace");
    assert_eq!(status, Status::Pass);
    assert_eq!(ratio, 0.0);
}

#[test]
#[ignore = "requires a local manifest of previously captured evidence and offline observer results"]
fn replay_saved_native_and_apple_captures() {
    #[derive(Deserialize)]
    struct SavedCapture {
        raw: String,
        exit_code: i32,
        occupied_slots: u64,
        apple: Option<String>,
        apple_hitch_ns: Option<u64>,
    }
    let manifest = std::env::var_os("OXIVIEW_M7_REPLAY_MANIFEST").expect("replay manifest path");
    let cases: Vec<SavedCapture> =
        serde_json::from_slice(&fs::read(manifest).expect("read manifest")).expect("manifest JSON");
    for case in cases {
        let raw = fs::read_to_string(&case.raw).expect("read viewer report");
        let capture = scroll::parse(&raw, Some(case.exit_code)).expect("historical capture");
        assert_eq!(
            capture.dropped.details["occupied_slots"], case.occupied_slots,
            "{}",
            case.raw
        );
        assert_eq!(
            capture.fps.hard_pass,
            Some(false),
            "legacy reference evidence"
        );
        if let Some(apple) = case.apple {
            let bytes = fs::read(apple).expect("read observer result");
            let (status, _, details) = validate(&bytes, &raw).expect("Apple trace handoff");
            assert_eq!(
                Some(
                    details["observed_hitch_time_ns"]
                        .as_u64()
                        .expect("hitch ns")
                ),
                case.apple_hitch_ns
            );
            assert_eq!(status, Status::Fail);
            assert!(
                details["metal_reference_issues"]
                    .to_string()
                    .contains("phase")
            );
        }
    }
}
#[cfg(target_os = "macos")]
#[test]
fn apple_observer_checks_run_without_opening_a_window() {
    let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("m7_hitches");
    let output = Command::new("/usr/bin/python3")
        .args(["-I", "-B", "-m", "unittest", "discover", "-s"])
        .arg(package)
        .args(["-p", "test_*.py"])
        .output()
        .expect("run the Apple observer's offline tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
