use super::*;
use serde_json::json;

fn fixture_meta(dir: &Path, head: &str, at: u64) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let meta = json!({"measured_at":at,"head":head,"dirty":false,"machine":"Fixture",
        "label":[format!("Run-{at}"),format!("测量-{at}")]});
    fs::write(dir.join("run.json"), meta.to_string()).map_err(|e| e.to_string())
}

fn fixture(dir: &Path, head: &str, at: u64, native: bool) -> Result<()> {
    fixture_meta(dir, head, at)?;
    let (name, data) = if native {
        let metric = |name: &str, unit: &str, value: f64| json!({"name":name,"current":{"unit":unit,"value":value}});
        let mut idle = metric("idle_cpu_percent", "%", 0.01);
        idle["details"] = json!({"interval_seconds":30.0,"submitted_frames":0,"frames":0,"display_link_events":0});
        let mut metrics = vec![
            idle,
            metric("input_to_submit_ms", "ms", 3.25),
            metric("input_to_pixel_ms", "ms", 27.0),
        ];
        for n in [1, 128, 1000] {
            metrics.push(metric(&format!("render/sparse_ui/{n}"), "ns", 2_500_000.0));
        }
        ("bench.json", json!({"metrics":metrics}))
    } else {
        (
            "animation-window.json",
            json!({"conditions":{"objects":128,"pixel_width":1920,"pixel_height":1200,
            "scale":2.0,"fullscreen":false},"warmup_seconds":1.0,"sample_seconds":5.0,
            "displayed_fps":119.6,"display_gap_p95_ms":8.335}),
        )
    };
    fs::write(dir.join(name), data.to_string()).map_err(|e| e.to_string())
}

#[test]
fn sixth_campaign_rotates_both_translations_and_preserves_history() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace")?;
    let head = git(repo, &["rev-parse", "HEAD"])?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let root = repo
        .join("target/xtask")
        .join(format!("perf-report-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("target/xtask")).map_err(|e| e.to_string())?;
    for file in ["README.md", "README_ZH.md"] {
        fs::write(root.join(file), format!("intro\n{BEGIN}\n{END}\noutro\n"))
            .map_err(|e| e.to_string())?;
    }
    for at in 1..=5 {
        fixture(
            &root.join(RUNS).join(format!("{at}-{}", &head[..7])),
            &head,
            at,
            at % 2 == 1,
        )?;
    }
    let input = root.join("incoming");
    fixture_meta(&input, &head, 6)?;
    let measure = input.join("m7-scroll.json");
    let scroll = json!({"rows":100_000,"samples":[
        {"occupied_slots":1192,"expected_slots":1200,"measured_duration_ns":10_000_000_000_u64},
        {"occupied_slots":1162,"expected_slots":1200,"measured_duration_ns":10_000_000_000_u64,
         "apple":{"observed_hitch_time_ns":8_333_333,"complete_apple_frame_lifetime_coverage":false}}
    ]});
    fs::write(&measure, scroll.to_string()).map_err(|e| e.to_string())?;
    let update = PerfReport {
        check: false,
        add: Some(input),
        start: None,
        machine: None,
    };
    update.update(&root)?;
    update.update(&root)?;
    check(&root)?;
    for (file, title, hitch) in [
        ("README.md", "Run", "0.83 ms/s (lower bound)"),
        ("README_ZH.md", "测量", "0.83 ms/s（下限）"),
    ] {
        let body = fs::read_to_string(root.join(file)).map_err(|e| e.to_string())?;
        assert!(body.starts_with("intro\n") && body.ends_with("\noutro\n"));
        assert!(!body.contains(&format!("{title}-1")));
        assert!(body.contains("1192 / 1200 | 119.2 FPS") && body.contains(hitch));
        for i in 2..=6 {
            assert!(body.contains(&format!("{title}-{i}")));
        }
    }
    let history = fs::read_to_string(root.join(HISTORY)).map_err(|e| e.to_string())?;
    assert!(history.contains("1162 / 1200 | 0.83 ms/s (lower bound)"));
    for i in 1..=6 {
        assert!(history.contains(&format!("## Run-{i} ")));
    }
    assert_eq!(read_archive(&root)?.len(), 6);
    let mut changed = fs::read_to_string(&measure).map_err(|e| e.to_string())?;
    changed.push('\n');
    fs::write(measure, changed).map_err(|e| e.to_string())?;
    assert!(update.update(&root).is_err());
    check(&root)?;
    fs::write(root.join("README.md"), "stale").map_err(|e| e.to_string())?;
    assert!(check(&root).is_err());
    fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
