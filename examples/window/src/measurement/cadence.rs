//! Sustained input checks the drawable pool under load, with actual presentation
//! feedback as well as submissions. This is not a fullscreen latency baseline.
use super::{Frame, MacOsControl, Point};
use std::{
    collections::BTreeSet,
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

#[derive(serde::Serialize)]
pub(super) struct Cadence {
    submitted_frames: usize,
    displayed_frames: usize,
    submitted_fps: f64,
    displayed_fps: f64,
    refresh_interval_ms: f64,
    submit_gap_p95_ms: f64,
    display_gap_p95_ms: f64,
    paint_submit_p95_ms: f64,
    input_submit_p50_ms: f64,
    input_submit_p95_ms: f64,
    input_submit_over_refresh_percent: f64,
    pub cadence_pass: bool,
    pub paint_submit_within_refresh: bool,
    pub input_submit_within_refresh: bool,
}

fn percentile(values: &mut [f64], percent: usize) -> Result<f64, String> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err("missing or invalid cadence samples".into());
    }
    values.sort_by(f64::total_cmp);
    Ok(values[(values.len() * percent).div_ceil(100) - 1])
}

fn gap_p95(times: &mut [Duration]) -> Result<f64, String> {
    times.sort_unstable();
    let mut gaps: Vec<_> = times
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).as_secs_f64() * 1000.0)
        .collect();
    percentile(&mut gaps, 95)
}

impl Cadence {
    fn summarize(
        mut submits: Vec<Duration>,
        mut displays: Vec<Duration>,
        mut latency: Vec<f64>,
        mut paint: Vec<f64>,
        elapsed: Duration,
        interval: Duration,
    ) -> Result<Self, String> {
        if elapsed.is_zero()
            || interval.is_zero()
            || submits.len() != latency.len()
            || submits.len() != paint.len()
        {
            return Err("invalid cadence interval or input attribution".into());
        }
        let submitted_fps = submits.len() as f64 / elapsed.as_secs_f64();
        let displayed_fps = displays.len() as f64 / elapsed.as_secs_f64();
        let refresh_interval_ms = interval.as_secs_f64() * 1000.0;
        let submit_gap_p95_ms = gap_p95(&mut submits)?;
        let display_gap_p95_ms = gap_p95(&mut displays)?;
        let paint_submit_p95_ms = percentile(&mut paint, 95)?;
        let input_submit_p50_ms = percentile(&mut latency, 50)?;
        let input_submit_p95_ms = percentile(&mut latency, 95)?;
        let input_submit_over_refresh_percent = 100.0
            * latency
                .iter()
                .filter(|&&value| value > refresh_interval_ms)
                .count() as f64
            / latency.len() as f64;
        // These are cadence tolerances for a local diagnostic under OS jitter,
        // not a relaxation of the separate input-to-submit latency hard target.
        let minimum_fps = 0.8 / interval.as_secs_f64();
        Ok(Self {
            submitted_frames: submits.len(),
            displayed_frames: displays.len(),
            submitted_fps,
            displayed_fps,
            refresh_interval_ms,
            submit_gap_p95_ms,
            display_gap_p95_ms,
            paint_submit_p95_ms,
            input_submit_p50_ms,
            input_submit_p95_ms,
            input_submit_over_refresh_percent,
            cadence_pass: submitted_fps >= minimum_fps
                && displayed_fps >= minimum_fps
                && submit_gap_p95_ms <= 1.5 * refresh_interval_ms
                && display_gap_p95_ms <= 1.5 * refresh_interval_ms,
            paint_submit_within_refresh: paint_submit_p95_ms <= refresh_interval_ms,
            input_submit_within_refresh: input_submit_p95_ms <= refresh_interval_ms,
        })
    }
}

pub(super) fn measure(
    control: &MacOsControl,
    rx: &Receiver<Frame>,
    fullscreen: bool,
) -> Result<Cadence, String> {
    while rx.try_recv().is_ok() {}
    // Calibrate from an input-consuming frame, not startup's default interval.
    let capture = control
        .diagnostic_post_pointer(Point::new(40.0, 40.0))
        .map_err(|e| e.to_string())?;
    let timeout = Instant::now() + Duration::from_secs(5);
    let interval = loop {
        let frame = rx
            .recv_timeout(timeout.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("cadence warm-up frame unavailable: {e}"))?;
        if frame.fullscreen == fullscreen
            && frame
                .record
                .is_some_and(|r| r.input_capture_timestamp == capture)
        {
            break frame.timing.refresh_interval;
        }
    };
    let spacing = interval / 2;
    if spacing.is_zero() {
        return Err("zero cadence posting interval".into());
    }
    control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?;
    let start = control.diagnostic_now();
    let first_post = Instant::now();
    let deadline = first_post + Duration::from_secs(3);
    let mut next_post = first_post;
    let mut captures = BTreeSet::new();
    let mut index = 0;
    // One second fills all 60 overlay slots even on a 60Hz display. The next
    // two seconds continuously feed AppKit at twice the observed refresh rate.
    while next_post < deadline {
        std::thread::sleep(next_post.saturating_duration_since(Instant::now()));
        if Instant::now() >= deadline {
            break;
        }
        let point = Point::new(
            100.0 + f64::from(index % 100) * 3.0,
            140.0 + f64::from(index % 70) * 2.0,
        );
        captures.insert(
            control
                .diagnostic_post_pointer(point)
                .map_err(|e| e.to_string())?,
        );
        index += 1;
        // Do not burst-replay missed posts if the measurement thread was delayed.
        next_post = (next_post + spacing).max(Instant::now());
    }
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    let end = control.diagnostic_now();
    let start = start + Duration::from_secs(1);
    // Drain late callbacks without including the drain interval in the rate.
    std::thread::sleep(interval.mul_f64(8.0).max(Duration::from_millis(200)));
    let mut submits = Vec::new();
    let mut latency = Vec::new();
    let mut paint = Vec::new();
    let mut ids = BTreeSet::new();
    for frame in rx.try_iter() {
        if frame.fullscreen != fullscreen {
            continue;
        }
        if let Some(record) = frame.record
            && captures.contains(&record.input_capture_timestamp)
        {
            if (frame.timing.refresh_interval.as_secs_f64() - interval.as_secs_f64()).abs()
                > interval.as_secs_f64() * 0.001
            {
                return Err("display interval changed during cadence check".into());
            }
            // Include IDs from the warm-up: a frame submitted just before the
            // measurement window can be displayed inside it.
            ids.insert(frame.drawable);
            if record.submission_timestamp >= start && record.submission_timestamp <= end {
                submits.push(record.submission_timestamp);
                paint.push(
                    record
                        .submission_timestamp
                        .checked_sub(frame.paint_started)
                        .ok_or("submission precedes paint")?
                        .as_secs_f64()
                        * 1000.0,
                );
                latency.push(
                    record
                        .submit_latency()
                        .ok_or("invalid capture time")?
                        .as_secs_f64()
                        * 1000.0,
                );
            }
        }
    }
    let displays = control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|(id, time)| ids.contains(id) && time.is_finite() && *time > 0.0)
        .filter_map(|(_, time)| Duration::try_from_secs_f64(time).ok())
        .filter(|time| *time >= start && *time <= end)
        .collect();
    Cadence::summarize(submits, displays, latency, paint, end - start, interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustained_rate_rejects_half_rate_and_hidden_presentation_drops() -> Result<(), String> {
        for interval in [
            Duration::from_nanos(16_666_667),
            Duration::from_nanos(8_333_333),
        ] {
            let times: Vec<_> = (1..=120).map(|i| interval * i).collect();
            let half: Vec<_> = times.iter().copied().step_by(2).collect();
            let elapsed = interval * 120;
            assert!(
                Cadence::summarize(
                    times.clone(),
                    times.clone(),
                    vec![1.0; 120],
                    vec![1.0; 120],
                    elapsed,
                    interval
                )?
                .cadence_pass
            );
            assert!(
                !Cadence::summarize(
                    half.clone(),
                    half.clone(),
                    vec![1.0; 60],
                    vec![1.0; 60],
                    elapsed,
                    interval
                )?
                .cadence_pass
            );
            assert!(
                !Cadence::summarize(
                    times.clone(),
                    half,
                    vec![1.0; 120],
                    vec![1.0; 120],
                    elapsed,
                    interval
                )?
                .cadence_pass
            );
            assert!(
                Cadence::summarize(
                    times.clone(),
                    Vec::new(),
                    vec![1.0; 120],
                    vec![1.0; 120],
                    elapsed,
                    interval
                )
                .is_err()
            );
            let refresh_ms = interval.as_secs_f64() * 1000.0;
            let delayed = Cadence::summarize(
                times.clone(),
                times.clone(),
                vec![2.0 * refresh_ms; 120],
                vec![1.1 * refresh_ms; 120],
                elapsed,
                interval,
            )?;
            assert!(delayed.cadence_pass);
            assert!(!delayed.paint_submit_within_refresh);
            assert!(!delayed.input_submit_within_refresh);
            assert_eq!(delayed.input_submit_over_refresh_percent, 100.0);
            let input_wait = Cadence::summarize(
                times.clone(),
                times,
                vec![2.0 * refresh_ms; 120],
                vec![1.0; 120],
                elapsed,
                interval,
            )?;
            assert!(input_wait.cadence_pass);
            assert!(input_wait.paint_submit_within_refresh);
            assert!(!input_wait.input_submit_within_refresh);
        }
        Ok(())
    }
}
