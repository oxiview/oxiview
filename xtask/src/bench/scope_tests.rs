use super::*;
use serde_json::Value;
use std::{
    path::PathBuf,
    process::{ExitStatus, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

pub(super) const DIVAN_M2: &str = "layout fastest │ slowest │ median │ mean │ samples │ iters
╰─ initial_layout │ │ │ │ │
   ├─ 100 1 µs │ 20 µs │ 2 µs │ 2 µs │ 40 │ 40
   ├─ 1000 1 ms │ 3 ms │ 2 ms │ 2 ms │ 40 │ 40
   ╰─ 10000 1 ms │ 3 ms │ 2 ms │ 2 ms │ 40 │ 40
ui_tree fastest │ slowest │ median │ mean │ samples │ iters
├─ arena_edits 1 ns │ 3 ns │ 2 ns │ 2 ns │ 100 │ 1000
├─ clean_preparation 1 ns │ 3 ns │ 2 ns │ 2 ns │ 100 │ 1000
├─ contained_layout_change 1 ns │ 3 ns │ 2 ns │ 2 ns │ 40 │ 40
├─ hit_testing 1 ns │ 3 ns │ 2 ns │ 2 ns │ 100 │ 1000
╰─ paint_only_update 1 ns │ 3 ns │ 2 ns │ 2 ns │ 40 │ 40";

const DIVAN_OTHER: &str = "render fastest │ slowest │ median │ mean │ samples │ iters
╰─ sparse_ui │ │ │ │ │
   ╰─ 1 1 ms │ 3 ms │ 2 ms │ 2 ms │ 20 │ 20";
const OTHER: &str = "render/sparse_ui/1";
const FIRST: &str = "layout/initial_layout/100";
const DIVAN_M3: &str = "components fastest │ slowest │ median │ mean │ samples │ iters
├─ batched_memo_diamond 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 10000
├─ bound_style_update 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 10000
├─ keyed_reorder_32 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 1000
╰─ show_mount_and_dispose 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 5000";
const DIVAN_M4: &str = "text_pipeline fastest │ slowest │ median │ mean │ samples │ iters
├─ cold_label_glyphs_2x 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 2560
├─ shape_rich_paragraph 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 5120
├─ shape_wrapped_paragraph 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 5120
╰─ warm_label_glyphs_2x 1 µs │ 3 µs │ 2 µs │ 2 µs │ 100 │ 51200";

const DIVAN_M5: &str = "interaction fastest │ slowest │ median │ mean │ samples │ iters
├─ tab_256 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
├─ action_eight_contexts 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
╰─ color_token_16_of_256 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80";

const DIVAN_M6: &str = "text_editing fastest │ slowest │ median │ mean │ samples │ iters
├─ replace_and_undo_4k 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
├─ cached_wrapped_geometry_32_lines 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
╰─ preedit_16_updates_then_commit 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80";

const DIVAN_M7: &str = "m7 fastest │ slowest │ median │ mean │ samples │ iters
├─ m7_variable_extent_update_lookup_ns 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
├─ m7_virtual_list_100k_advance_ns 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80
╰─ m7_virtual_table_diagonal_ns 1 µs │ 3 µs │ 2 µs │ 2 µs │ 80 │ 80";

fn fixture_machine() -> Machine {
    Machine {
        model: "Fixture model".into(),
        cpu: "Fixture CPU".into(),
        memory_bytes: 16 << 30,
        os: "Fixture OS".into(),
        rustc: "Fixture Rust".into(),
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("home directory");
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(home).join(format!(
            ".tmp/codex-oxiview-m2-test-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("target/xtask")).expect("fixture artifacts");
        Self(root)
    }

    fn path(&self) -> PathBuf {
        self.0
            .join("perf/baselines")
            .join(format!("{}.json", machine_id(&fixture_machine())))
    }

    fn artifact(&self, name: &str) -> PathBuf {
        self.0.join("target/xtask").join(name)
    }

    fn seed_baseline(&self) -> String {
        let mut machine = fixture_machine();
        machine.os = "Prior OS, with a comma and }".into();
        let mut metrics = pending();
        metrics.get_mut("idle_cpu_percent").expect("idle").value = Some(0.025);
        metrics.get_mut("input_to_submit_ms").expect("submit").value = Some(4.0);
        let pixel = metrics.get_mut("input_to_pixel_ms").expect("pixel");
        pixel.value = Some(25.0);
        pixel.refresh_interval_ms = Some(8.333333);
        pixel.regression_percent = 15.0;
        for name in [FIRST, OTHER] {
            metrics.insert(
                name.into(),
                Metric {
                    value: Some(1000.0),
                    unit: "ns".into(),
                    direction: Direction::Lower,
                    regression_percent: 3.25,
                    target: None,
                    refresh_interval_ms: None,
                },
            );
        }
        let baseline = Baseline {
            schema_version: 1,
            machine_id: machine_id(&machine),
            machine,
            recorded_at_unix_seconds: 42,
            metrics,
        };
        let text = serde_json::to_string_pretty(&baseline)
            .expect("serialize fixture")
            .replace("0.025,", "0.02457525210674720001,");
        let path = self.path();
        fs::create_dir_all(path.parent().expect("baseline directory"))
            .expect("create baseline dir");
        fs::write(path, &text).expect("write original fixture baseline");
        text
    }

    fn run(&self, scope: Scope, record: bool, rounds: [&str; 3]) -> (Result<Status>, String) {
        self.run_with_scroll(scope, record, rounds, None)
    }

    fn run_with_scroll(
        &self,
        scope: Scope,
        record: bool,
        rounds: [&str; 3],
        raw: Option<(&Value, i32)>,
    ) -> (Result<Status>, String) {
        let check = BenchCheck { scope, record };
        let mut log = String::new();
        let mut rounds = rounds.into_iter();
        let outcome = check.process(
            &self.0,
            &mut log,
            || Ok(fixture_machine()),
            |command| {
                assert!(
                    scope != Scope::M7Native,
                    "native-only scope must never run Divan"
                );
                let expected: &[&str] = match scope {
                    Scope::Full => &["bench", "--workspace", "--benches", "--locked"],
                    Scope::M2 => &[
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
                        "--",
                        "--exact",
                        "layout::initial_layout::100",
                        "layout::initial_layout::1000",
                        "layout::initial_layout::10000",
                        "ui_tree::arena_edits",
                        "ui_tree::clean_preparation",
                        "ui_tree::contained_layout_change",
                        "ui_tree::hit_testing",
                        "ui_tree::paint_only_update",
                    ],
                    Scope::M3 => &[
                        "bench",
                        "--locked",
                        "-p",
                        "oxiview-core",
                        "--bench",
                        "components",
                        "--",
                        "--exact",
                        "components::batched_memo_diamond",
                        "components::bound_style_update",
                        "components::show_mount_and_dispose",
                        "components::keyed_reorder_32",
                    ],
                    Scope::M4 => &[
                        "bench",
                        "--locked",
                        "-p",
                        "oxiview-text",
                        "--bench",
                        "text_pipeline",
                        "--",
                        "--exact",
                        "text_pipeline::shape_wrapped_paragraph",
                        "text_pipeline::shape_rich_paragraph",
                        "text_pipeline::cold_label_glyphs_2x",
                        "text_pipeline::warm_label_glyphs_2x",
                    ],
                    Scope::M5 => &[
                        "bench",
                        "--locked",
                        "-p",
                        "oxiview-core",
                        "--bench",
                        "interaction",
                        "--",
                        "--exact",
                        "interaction::tab_256",
                        "interaction::action_eight_contexts",
                        "interaction::color_token_16_of_256",
                    ],
                    Scope::M6 => &[
                        "bench",
                        "--locked",
                        "-p",
                        "oxiview-text",
                        "--bench",
                        "text_editing",
                        "--",
                        "--exact",
                        "text_editing::replace_and_undo_4k",
                        "text_editing::cached_wrapped_geometry_32_lines",
                        "text_editing::preedit_16_updates_then_commit",
                    ],
                    Scope::M7 => &[
                        "bench",
                        "--locked",
                        "-p",
                        "oxiview-widgets",
                        "--bench",
                        "m7",
                        "--",
                        "--exact",
                        "m7::m7_variable_extent_update_lookup_ns",
                        "m7::m7_virtual_list_100k_advance_ns",
                        "m7::m7_virtual_table_diagonal_ns",
                    ],
                    Scope::M7Native => &[],
                };
                assert!(command.get_args().eq(expected.iter().copied()));
                for (key, _) in std::env::vars_os() {
                    if key.to_string_lossy().starts_with("DIVAN_") {
                        assert!(command.get_envs().any(|(k, v)| k == key && v.is_none()));
                    }
                }
                Ok(Output {
                    status: ExitStatus::default(),
                    stdout: rounds
                        .next()
                        .expect("one invocation per round")
                        .as_bytes()
                        .to_vec(),
                    stderr: b"synthetic cargo stderr".to_vec(),
                })
            },
            |_| {
                assert!(
                    scope == Scope::Full,
                    "scoped comparisons must never call the native runner"
                );
                Ok([
                    ("input_to_submit_ms", 4.0, 8.333333, Some(8.333333)),
                    ("input_to_pixel_ms", 25.0, 0.0, Some(8.333333)),
                    ("idle_cpu_percent", 0.02, 0.1, None),
                ]
                .into_iter()
                .map(|(name, value, target, refresh_interval_ms)| {
                    (
                        name.into(),
                        native::Measurement {
                            value,
                            target: Some(target),
                            refresh_interval_ms,
                            hard_pass: Some(true),
                            details: Value::Null,
                        },
                    )
                })
                .collect())
            },
            |_, stem| {
                assert!(scope == Scope::Full || scope == Scope::M7Native);
                assert_eq!(
                    stem,
                    if scope == Scope::Full {
                        "bench-scroll"
                    } else {
                        "bench-m7-native-scroll"
                    }
                );
                let (text, exit) = raw.map_or_else(
                    || (scroll::tests::fixture_120(None).to_string(), 0),
                    |(report, exit)| (report.to_string(), exit),
                );
                fs::write(self.artifact(&format!("{stem}.json")), &text).expect("raw fixture JSON");
                fs::write(self.artifact(&format!("{stem}.log")), "fixture stderr")
                    .expect("raw fixture stderr");
                scroll::parse(&text, Some(exit))
            },
        );
        (outcome, log)
    }

    fn report(&self, scope: Scope) -> Value {
        serde_json::from_slice(&fs::read(self.artifact(scope.results_file())).expect("result file"))
            .expect("valid result JSON")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn metrics(report: &Value) -> &[Value] {
    report["metrics"].as_array().expect("metric array")
}

fn result_for<'a>(report: &'a Value, name: &str) -> &'a Value {
    metrics(report)
        .iter()
        .find(|entry| entry["name"] == name)
        .expect("result for metric")
}

#[test]
fn m2_compares_only_eight_medians_and_uses_distinct_artifacts() {
    let fixture = Fixture::new();
    for artifact in Scope::Full.artifacts() {
        fs::write(fixture.artifact(artifact), "full artifact").expect("seed full artifact");
    }
    let nine = DIVAN_M2.replace("2 µs", "9 µs");
    let four = DIVAN_M2.replace("2 µs", "4 µs");
    let (outcome, log) = fixture.run(Scope::M2, false, [&nine, DIVAN_M2, &four]);
    assert_eq!(outcome, Ok(Status::Fail));
    assert!(log.contains("Scope: M2 layout and UI tree (8 metrics)"));
    assert!(log.contains("cargo xtask bench --m2 --record"));
    assert!(!fixture.path().exists());
    let report = fixture.report(Scope::M2);
    assert_eq!(report["scope"], "m2");
    assert_eq!(metrics(&report).len(), 8);
    assert!(
        metrics(&report)
            .iter()
            .all(|entry| entry["status"] == "missing_baseline")
    );
    assert_eq!(result_for(&report, FIRST)["current"]["value"], 4000.0);
    for round in 1..=3 {
        let raw = fs::read_to_string(fixture.artifact(&format!("bench-m2-{round}.log")))
            .expect("scoped raw log");
        assert!(raw.contains("synthetic cargo stderr"));
    }
    for artifact in Scope::Full.artifacts() {
        assert_eq!(
            fs::read_to_string(fixture.artifact(artifact)).expect("full artifact"),
            "full artifact"
        );
    }
}

#[test]
fn m2_record_merges_only_its_values_and_retains_full_missing_measurement_checks() {
    let fixture = Fixture::new();
    let original = fixture.seed_baseline();
    assert_eq!(
        fixture.run(Scope::M2, true, [DIVAN_M2; 3]).0,
        Ok(Status::Pass)
    );
    let merged = fs::read_to_string(fixture.path()).expect("merged baseline");
    assert!(merged.contains("0.02457525210674720001,"));
    let mut after: Value = serde_json::from_str(&merged).expect("merged baseline JSON");
    let before: Value = serde_json::from_str(&original).expect("original baseline JSON");
    assert_eq!(after["metrics"][FIRST]["value"], 2000.0);
    assert_eq!(after["metrics"][FIRST]["regression_percent"], 3.25);
    for &name in M2_METRICS {
        assert!(after["metrics"][name]["value"].is_number());
        let fields = after["metrics"].as_object_mut().expect("metric object");
        fields.remove(name);
        if let Some(old) = before["metrics"].get(name) {
            fields.insert(name.into(), old.clone());
        }
    }
    assert_eq!(after, before);

    assert_eq!(
        fixture.run(Scope::M2, false, [DIVAN_M2; 3]).0,
        Ok(Status::Pass)
    );
    let regressed = DIVAN_M2.replace("2 µs", "2.1 µs");
    assert_eq!(
        fixture.run(Scope::M2, false, [&regressed; 3]).0,
        Ok(Status::Fail)
    );
    assert_eq!(
        result_for(&fixture.report(Scope::M2), FIRST)["status"],
        "fail"
    );
    let scoped_report =
        fs::read(fixture.artifact(Scope::M2.results_file())).expect("scoped report");
    for record in [false, true] {
        assert_eq!(
            fixture.run(Scope::Full, record, [DIVAN_M2; 3]).0,
            Ok(Status::Fail)
        );
        let report = fixture.report(Scope::Full);
        assert!(report.get("scope").is_none());
        assert_eq!(result_for(&report, OTHER)["status"], "missing_measurement");
    }
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("unchanged baseline"),
        merged
    );
    assert_eq!(
        fs::read(fixture.artifact(Scope::M2.results_file())).expect("scoped report"),
        scoped_report
    );
}

#[test]
fn each_milestone_records_only_its_metrics_and_preserves_earlier_baselines() {
    let fixture = Fixture::new();
    fixture.seed_baseline();
    assert_eq!(
        fixture.run(Scope::M2, true, [DIVAN_M2; 3]).0,
        Ok(Status::Pass)
    );
    let m2_report = fixture.report(Scope::M2);
    for (scope, label, names, rounds) in [
        (Scope::M3, "m3", M3_METRICS, DIVAN_M3),
        (Scope::M4, "m4", M4_METRICS, DIVAN_M4),
        (Scope::M5, "m5", M5_METRICS, DIVAN_M5),
        (Scope::M6, "m6", M6_METRICS, DIVAN_M6),
        (Scope::M7, "m7", M7_METRICS, DIVAN_M7),
    ] {
        let baseline = || -> Value {
            serde_json::from_slice(&fs::read(fixture.path()).expect("machine baseline"))
                .expect("valid baseline JSON")
        };
        let previous = baseline();
        assert_eq!(fixture.run(scope, false, [rounds; 3]).0, Ok(Status::Fail));
        let first = fixture.report(scope);
        assert_eq!(first["scope"], label);
        assert_eq!(metrics(&first).len(), names.len());
        assert!(
            metrics(&first)
                .iter()
                .all(|m| m["status"] == "missing_baseline")
        );
        assert_eq!(fixture.run(scope, true, [rounds; 3]).0, Ok(Status::Pass));
        let mut after = baseline();
        for &name in names {
            let metric = after["metrics"]
                .as_object_mut()
                .expect("metric object")
                .remove(name)
                .expect("recorded metric");
            assert_eq!(metric["value"], 2000.0);
        }
        assert_eq!(after, previous);
        assert_eq!(fixture.run(scope, false, [rounds; 3]).0, Ok(Status::Pass));
        assert_eq!(fixture.report(Scope::M2), m2_report);
        let slow = rounds.replace("2 µs", "3 µs");
        assert_eq!(fixture.run(scope, false, [&slow; 3]).0, Ok(Status::Fail));
    }
}

#[test]
fn missing_and_invalid_m2_rounds_cannot_record() {
    let fixture = Fixture::new();
    let original = fixture.seed_baseline();
    let partial = DIVAN_M2
        .lines()
        .filter(|line| !line.contains("hit_testing"))
        .collect::<Vec<_>>()
        .join("\n");
    let unexpected = format!("{DIVAN_M2}\n{DIVAN_OTHER}");
    assert_eq!(
        fixture.run(Scope::M2, true, [&partial; 3]).0,
        Ok(Status::Fail)
    );
    let report = fixture.report(Scope::M2);
    assert_eq!(metrics(&report).len(), 8);
    assert_eq!(
        result_for(&report, "ui_tree/hit_testing")["status"],
        "missing_measurement"
    );
    let (inconsistent, _) = fixture.run(Scope::M2, true, [DIVAN_M2, &partial, DIVAN_M2]);
    assert_eq!(
        inconsistent.expect_err("inconsistent rounds"),
        "Metric sets differ across the three runs"
    );
    let (invalid, _) = fixture.run(Scope::M2, true, ["invalid Divan"; 3]);
    assert_eq!(
        invalid.expect_err("invalid output"),
        "No Divan metrics parsed"
    );
    let (extra, _) = fixture.run(Scope::M2, true, [&unexpected; 3]);
    assert_eq!(
        extra.expect_err("out-of-scope output"),
        "Unexpected M2 metric: render/sparse_ui/1"
    );
    assert!(!fixture.artifact(Scope::M2.results_file()).exists());
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("unchanged baseline"),
        original
    );
}

#[test]
fn m2_first_record_creates_partial_machine_baseline_for_full_comparison() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.run(Scope::M2, true, [DIVAN_M2; 3]).0,
        Ok(Status::Pass)
    );
    let text = fs::read_to_string(fixture.path()).expect("new machine baseline");
    let baseline = serde_json::from_str::<Baseline>(&text).expect("baseline JSON");
    validate(&baseline).expect("baseline accepted by full bench");
    assert_eq!(baseline.metrics.len(), 8 + pending().len());
    for name in pending().keys() {
        assert!(baseline.metrics[name].value.is_none());
    }
    assert_eq!(
        fixture.run(Scope::M2, false, [DIVAN_M2; 3]).0,
        Ok(Status::Pass)
    );
    let full = format!("{DIVAN_M2}\n{DIVAN_OTHER}");
    assert_eq!(
        fixture.run(Scope::Full, false, [&full; 3]).0,
        Ok(Status::Fail)
    );
    let report = fixture.report(Scope::Full);
    for name in [
        OTHER,
        "input_to_submit_ms",
        "input_to_pixel_ms",
        "idle_cpu_percent",
        scroll::METRIC,
    ] {
        assert_eq!(result_for(&report, name)["status"], "missing_baseline");
    }
    assert_eq!(result_for(&report, scroll::METRIC)["current"]["value"], 0.0);
    assert_eq!(result_for(&report, scroll::METRIC)["hard_check_pass"], true);
}

#[test]
fn native_scope_treats_historical_null_as_missing_and_records_only_the_native_gate() {
    let fixture = Fixture::new();
    let original = fixture.seed_baseline();
    let run = |record| fixture.run(Scope::M7Native, record, ["unused"; 3]);
    assert_eq!(run(false).0, Ok(Status::Fail));
    let report = fixture.report(Scope::M7Native);
    assert_eq!(report["scope"], "m7-native");
    assert_eq!(metrics(&report).len(), 2);
    let metric = result_for(&report, scroll::METRIC);
    assert_eq!(metric["status"], "missing_baseline");
    assert_eq!(metric["hard_check_pass"], true);
    assert!(metric["baseline_value"].is_null());
    assert_eq!(metric["current"]["value"], 0.0);
    assert_eq!(metric["details"]["expected_slots"], 1200);
    let fps = result_for(&report, scroll::FPS_METRIC);
    assert_eq!(fps["status"], "pass");
    assert_eq!(fps["hard_check_pass"], true);
    assert!(fps["baseline_value"].is_null());
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("baseline"),
        original
    );

    assert_eq!(run(true).0, Ok(Status::Pass));
    let merged = fs::read_to_string(fixture.path()).expect("merged baseline");
    assert!(merged.contains("0.02457525210674720001,"));
    let mut after: Value = serde_json::from_str(&merged).expect("merged baseline JSON");
    let before: Value = serde_json::from_str(&original).expect("prior baseline JSON");
    let measured = std::mem::replace(
        &mut after["metrics"][scroll::METRIC],
        before["metrics"][scroll::METRIC].clone(),
    );
    assert_eq!(after, before);
    assert_eq!(measured["value"], 0.0);
    assert_eq!(measured["target"], 0.0);
    assert_eq!(measured["refresh_interval_ms"], 8.333333);
    assert_eq!(run(false).0, Ok(Status::Pass));

    let mut different_display = merged.clone();
    let interval = different_display.rfind("8.333333").expect("native refresh");
    different_display.replace_range(interval..interval + "8.333333".len(), "16.666667");
    fs::write(fixture.path(), &different_display).expect("different recorded display");
    assert!(
        run(false)
            .0
            .expect_err("incompatible display")
            .contains("refresh interval differs")
    );
    assert_eq!(
        result_for(&fixture.report(Scope::M7Native), scroll::METRIC)["status"],
        "environment_error"
    );
    assert_eq!(run(true).0, Ok(Status::Pass));
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("replacement refresh"),
        merged
    );
}

#[test]
fn native_scope_can_create_a_baseline_without_overwriting_its_measurement() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.run(Scope::M7Native, true, ["unused"; 3]).0,
        Ok(Status::Pass)
    );
    let baseline: Baseline =
        serde_json::from_slice(&fs::read(fixture.path()).expect("new baseline"))
            .expect("baseline JSON");
    validate(&baseline).expect("compatible full baseline");
    assert_eq!(baseline.metrics.len(), pending().len());
    assert_eq!(baseline.metrics[scroll::METRIC].value, Some(0.0));
    for &name in LEGACY_NATIVE_METRICS {
        assert!(baseline.metrics[name].value.is_none());
    }
}

#[test]
fn both_native_scopes_keep_raw_evidence_and_never_record_a_hard_miss() {
    let fixture = Fixture::new();
    let original = fixture.seed_baseline();
    let divan = format!("{DIVAN_M2}\n{DIVAN_OTHER}");
    assert_eq!(
        fixture.run(Scope::Full, false, [&divan; 3]).0,
        Ok(Status::Fail)
    );
    let report = fixture.report(Scope::Full);
    assert_eq!(
        result_for(&report, scroll::METRIC)["status"],
        "missing_baseline"
    );
    assert_eq!(result_for(&report, scroll::METRIC)["current"]["value"], 0.0);
    for &name in LEGACY_NATIVE_METRICS {
        assert_eq!(result_for(&report, name)["status"], "pass");
    }

    let missed = scroll::tests::fixture_120(Some(0));
    let mut incomplete = missed.clone();
    incomplete["frames"][1]["presented_ns"] = Value::Null;
    for scope in [Scope::Full, Scope::M7Native] {
        let stem = scope.scroll_artifact_stem().expect("scroll artifacts");
        for record in [false, true] {
            assert_eq!(
                fixture
                    .run_with_scroll(scope, record, [&divan; 3], Some((&missed, 1)))
                    .0,
                Ok(Status::Fail)
            );
            let report = fixture.report(scope);
            let metric = result_for(&report, scroll::METRIC);
            assert_eq!(metric["status"], "hard_fail");
            assert_eq!(metric["current"]["value"], 1.0);
            assert_eq!(metric["hard_check_pass"], false);
            assert_eq!(metric["details"]["missing_slots"], serde_json::json!([0]));
            assert_eq!(
                fs::read_to_string(fixture.artifact(&format!("{stem}.json"))).expect("raw report"),
                missed.to_string()
            );

            let result = fixture
                .run_with_scroll(scope, record, [&divan; 3], Some((&incomplete, 2)))
                .0;
            assert!(
                result
                    .expect_err("incomplete capture")
                    .contains("missing callbacks")
            );
            let report = fixture.report(scope);
            let metric = result_for(&report, scroll::METRIC);
            assert_eq!(metric["status"], "environment_error");
            assert!(metric["hard_check_pass"].is_null());
            assert!(
                metric["error"]
                    .as_str()
                    .expect("failure detail")
                    .contains("missing callbacks")
            );
            assert_eq!(
                fs::read_to_string(fixture.artifact(&format!("{stem}.json"))).expect("raw report"),
                incomplete.to_string()
            );
            assert!(fixture.artifact(&format!("{stem}.log")).exists());
            assert_eq!(
                fs::read_to_string(fixture.path()).expect("untouched baseline"),
                original
            );
        }
    }
    assert_eq!(
        fixture.run(Scope::Full, true, [&divan; 3]).0,
        Ok(Status::Pass)
    );
    let baseline: Baseline =
        serde_json::from_slice(&fs::read(fixture.path()).expect("full baseline"))
            .expect("full baseline JSON");
    validate(&baseline).expect("compatible measured baseline");
    assert_eq!(baseline.metrics[scroll::METRIC].value, Some(0.0));
    assert_eq!(
        fixture.run(Scope::Full, false, [&divan; 3]).0,
        Ok(Status::Pass)
    );
}

#[test]
fn recording_rejects_a_baseline_changed_during_measurement() {
    let fixture = Fixture::new();
    let original = fixture.seed_baseline();
    let newer = original.replace("0.02457525210674720001", "0.01234567890123456789");
    fs::write(fixture.path(), &newer).expect("record from another process");
    let result = record::write(
        &fixture.artifact(""),
        &fixture.path(),
        Some(&original),
        &original,
    );
    assert!(
        result
            .expect_err("reject stale recording")
            .contains("changed during measurement")
    );
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("preserved concurrent baseline"),
        newer
    );
}
