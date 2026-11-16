//! Strict native-window measurement protocol; no window is opened by unit tests.
use super::*;
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Latency {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    refresh_interval_ms: f64,
    p95_frames: f64,
    submit_p50_ms: f64,
    submit_p95_ms: f64,
    submit_max_ms: f64,
    compositor_offset_ms: f64,
    pixel_gate: String,
    pass: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Idle {
    frames: u64,
    submitted_frames: u64,
    display_link_events: u64,
    cpu_percent: f64,
    interval_seconds: f64,
    pass: bool,
}
fn nonnegative(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && *v >= 0.0)
}
impl Latency {
    fn hard_pass(&self) -> bool {
        self.submit_p95_ms <= self.refresh_interval_ms
    }
    fn parse(text: &str) -> Result<Self> {
        let v: Self =
            serde_json::from_str(text).map_err(|e| format!("invalid latency JSON: {e}"))?;
        if v.samples == 0
            || !nonnegative(&[
                v.submit_p50_ms,
                v.submit_p95_ms,
                v.submit_max_ms,
                v.compositor_offset_ms,
                v.p50_ms,
                v.p95_ms,
                v.max_ms,
                v.refresh_interval_ms,
                v.p95_frames,
            ])
            || v.refresh_interval_ms == 0.0
            || v.submit_p50_ms > v.submit_p95_ms
            || v.submit_p95_ms > v.submit_max_ms
            || v.pixel_gate != "requires human-recorded baseline; checked by xtask bench"
            || v.p50_ms > v.p95_ms
            || v.p95_ms > v.max_ms
            || (v.p95_frames - v.p95_ms / v.refresh_interval_ms).abs() > 1e-9
            || v.pass != v.hard_pass()
        {
            return Err("invalid or inconsistent latency measurement".into());
        }
        Ok(v)
    }
}
impl Idle {
    fn hard_pass(&self) -> bool {
        self.frames == 0
            && self.submitted_frames == 0
            && self.display_link_events == 0
            && self.cpu_percent <= 0.1
    }
    fn parse(text: &str) -> Result<Self> {
        let v: Self = serde_json::from_str(text).map_err(|e| format!("invalid idle JSON: {e}"))?;
        if !nonnegative(&[v.cpu_percent, v.interval_seconds])
            || v.interval_seconds == 0.0
            || v.pass != v.hard_pass()
        {
            return Err("invalid or inconsistent idle measurement".into());
        }
        Ok(v)
    }
}

pub(super) struct Measurement {
    pub value: f64,
    pub target: Option<f64>,
    pub refresh_interval_ms: Option<f64>,
    pub hard_pass: Option<bool>,
    pub details: serde_json::Value,
}
fn environment(platform: &str) -> Result<()> {
    if platform != "macos" {
        return Err("input_to_pixel_ms and idle_cpu_percent: unsupported platform; native measurements require macOS 14+, WindowServer and a Metal GPU".into());
    }
    Ok(())
}
fn exit_status(code: Option<i32>) -> Result<()> {
    match code {
        Some(0 | 1) => Ok(()),
        _ => Err(format!(
            "native window measurement environment error (exit {code:?}); macOS 14+, a visible WindowServer session and Metal GPU are required"
        )),
    }
}
fn run(root: &Path, mode: &str, amount: &str, name: &str) -> Result<(String, Option<i32>)> {
    let out_path = root.join(format!("target/xtask/bench-{name}.json"));
    let err_path = root.join(format!("target/xtask/bench-{name}.log"));
    let mut child = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "run",
            "--release",
            "-p",
            "window",
            "--",
            mode,
            amount,
            "--json",
        ])
        .current_dir(root)
        .stdout(fs::File::create(&out_path).map_err(|e| e.to_string())?)
        .stderr(fs::File::create(&err_path).map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "native {name} measurement timed out after 180s; display session/GPU required; see {}",
                err_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    exit_status(status.code()).map_err(|e| format!("{e}; see {}", err_path.display()))?;
    Ok((
        fs::read_to_string(out_path).map_err(|e| e.to_string())?,
        status.code(),
    ))
}
pub(super) fn measure(root: &Path) -> Result<BTreeMap<String, Measurement>> {
    environment(std::env::consts::OS)?;
    let mut rounds = Vec::new();
    for round in 1..=3 {
        let (text, code) = run(
            root,
            "--measure-latency",
            "120",
            &format!("latency-{round}"),
        )?;
        let latency = Latency::parse(&text)?;
        if latency.samples != 120 || code != Some(i32::from(!latency.pass)) {
            return Err("latency sample count or process exit disagrees with JSON".into());
        }
        rounds.push(latency);
    }
    let mut measurements = aggregate_latency(&rounds)?;
    let (text, code) = run(root, "--idle-check", "5", "idle")?;
    let idle = Idle::parse(&text)?;
    if idle.interval_seconds < 5.0 || code != Some(i32::from(!idle.pass)) {
        return Err("idle interval or process exit disagrees with JSON".into());
    }
    measurements.insert(
        "idle_cpu_percent".into(),
        Measurement {
            value: idle.cpu_percent,
            target: Some(0.1),
            refresh_interval_ms: None,
            hard_pass: Some(idle.hard_pass()),
            details: serde_json::to_value(idle).map_err(|e| e.to_string())?,
        },
    );
    Ok(measurements)
}

fn aggregate_latency(rounds: &[Latency]) -> Result<BTreeMap<String, Measurement>> {
    let [a, b, c] = rounds else {
        return Err("expected three latency rounds".into());
    };
    for round in [b, c] {
        same_display(Some(a.refresh_interval_ms), Some(round.refresh_interval_ms))?;
    }
    let median = |values: [f64; 3]| {
        let mut values = values;
        values.sort_by(f64::total_cmp);
        values[1]
    };
    let details = serde_json::json!({"aggregation":"median of three p95 values", "rounds":rounds});
    Ok(BTreeMap::from([
        (
            "input_to_submit_ms".into(),
            Measurement {
                value: median([a.submit_p95_ms, b.submit_p95_ms, c.submit_p95_ms]),
                target: Some(a.refresh_interval_ms),
                refresh_interval_ms: Some(a.refresh_interval_ms),
                hard_pass: Some(rounds.iter().all(Latency::hard_pass)),
                details: details.clone(),
            },
        ),
        (
            "input_to_pixel_ms".into(),
            Measurement {
                value: median([a.p95_ms, b.p95_ms, c.p95_ms]),
                target: None,
                refresh_interval_ms: Some(a.refresh_interval_ms),
                hard_pass: None,
                details,
            },
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    const LATENCY: &str = r#"{"samples":120,"p50_ms":4,"p95_ms":10,"max_ms":12,"refresh_interval_ms":10,"p95_frames":1,"submit_p50_ms":4,"submit_p95_ms":10,"submit_max_ms":12,"compositor_offset_ms":10,"pixel_gate":"requires human-recorded baseline; checked by xtask bench","pass":true}"#;
    const IDLE: &str = r#"{"frames":0,"submitted_frames":0,"display_link_events":0,"cpu_percent":0.1,"interval_seconds":5,"pass":true}"#;
    #[test]
    fn three_round_median_preserves_failures_and_rejects_display_changes() {
        let mut rounds: Vec<_> = (0..3).map(|_| Latency::parse(LATENCY).unwrap()).collect();
        rounds[0].p95_ms = 40.0;
        rounds[1].p95_ms = 20.0;
        rounds[2].p95_ms = 30.0;
        rounds[0].submit_p95_ms = 11.0; // One bad hard check must not be hidden.
        rounds[1].submit_p95_ms = 4.0;
        rounds[2].submit_p95_ms = 5.0;
        let result = aggregate_latency(&rounds).unwrap();
        assert_eq!(result["input_to_pixel_ms"].value, 30.0);
        assert_eq!(result["input_to_submit_ms"].value, 5.0);
        assert_eq!(result["input_to_submit_ms"].hard_pass, Some(false));
        assert_eq!(
            result["input_to_pixel_ms"].details["rounds"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(aggregate_latency(&rounds[..2]).is_err());
        rounds[2].refresh_interval_ms = 16.666667;
        assert!(aggregate_latency(&rounds).is_err());
    }
    #[test]
    fn strict_json_and_boundaries() {
        assert!(Latency::parse(LATENCY).unwrap().hard_pass());
        let mut latency = Latency::parse(LATENCY).unwrap();
        latency.submit_p95_ms = 10.00001;
        assert!(!latency.hard_pass());
        assert!(Idle::parse(IDLE).unwrap().hard_pass());
        let mut idle = Idle::parse(IDLE).unwrap();
        idle.cpu_percent = 0.11;
        assert!(!idle.hard_pass());
        idle.cpu_percent = 0.1;
        idle.frames = 1;
        assert!(!idle.hard_pass());
        idle.frames = 0;
        idle.display_link_events = 1;
        assert!(!idle.hard_pass());
        for invalid in [
            LATENCY.replace("\"samples\":120,", ""),
            LATENCY.replace("120", "0"),
            LATENCY.replace("\"p95_ms\":10", "\"p95_ms\":null"),
            LATENCY.replace("\"p95_ms\":10", "\"p95_ms\":1e999"),
            LATENCY.replace("\"p95_ms\":10", "\"p95_ms\":-1"),
            LATENCY.replace("\"p95_frames\":1", "\"p95_frames\":2"),
            LATENCY.replace("true", "false"),
            format!("{LATENCY}\n{LATENCY}"),
        ] {
            assert!(Latency::parse(&invalid).is_err(), "{invalid}");
        }
        for invalid in [
            IDLE.replace("\"frames\":0,", ""),
            IDLE.replace("\"cpu_percent\":0.1", "\"cpu_percent\":null"),
            IDLE.replace("\"cpu_percent\":0.1", "\"cpu_percent\":1e999"),
            IDLE.replace("\"interval_seconds\":5", "\"interval_seconds\":0"),
            IDLE.replace("\"frames\":0", "\"frames\":1"),
            IDLE.replace("\"display_link_events\":0", "\"display_link_events\":1"),
        ] {
            assert!(Idle::parse(&invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn environment_errors_are_exit_two() {
        for error in [
            environment("linux"),
            environment("windows"),
            exit_status(Some(2)),
            exit_status(None),
        ] {
            let mut report = crate::report::Report::default();
            report.checks.push(crate::report::CheckResult::error(
                "bench",
                error.unwrap_err(),
            ));
            assert_eq!(report.exit_code(), 2);
        }
        assert!(environment("macos").is_ok());
        assert!(exit_status(Some(0)).is_ok());
        assert!(exit_status(Some(1)).is_ok());
    }
}
