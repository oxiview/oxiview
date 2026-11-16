//! Explicit native diagnostics. Never substitutes predicted time for presentedTime.
mod cadence;

use oxiview_base::Point;
use oxiview_core::{FrameTiming, LatencyRecord};
use oxiview_platform_macos::MacOsControl;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

pub(super) struct Frame {
    pub pointer: Point,
    pub fullscreen: bool,
    pub drawable: u64,
    pub slot: Duration,
    pub paint_started: Duration,
    pub timing: FrameTiming,
    pub record: Option<LatencyRecord>,
}

fn displayed(control: &MacOsControl, drawable: u64) -> Result<Duration, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        for (id, time) in control
            .diagnostic_presented_drawables()
            .map_err(|e| e.to_string())?
        {
            if id == drawable {
                if time == 0.0 {
                    return Err(format!(
                        "MTLDrawable {id} not displayed (presented handler reported zero presentedTime)"
                    ));
                }
                return Duration::try_from_secs_f64(time).map_err(|e| e.to_string());
            }
        }
        if Instant::now() >= deadline {
            return Err("no nonzero MTLDrawable.presentedTime within 5s; visible window, WindowServer and GPU required".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn sample(
    control: &MacOsControl,
    rx: &Receiver<Frame>,
    point: Point,
) -> Result<(Frame, f64), String> {
    let capture = control
        .diagnostic_post_pointer(point)
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let frame = loop {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("input-consuming frame unavailable: {e}"))?;
        if frame.pointer == point
            && frame
                .record
                .is_some_and(|r| r.input_capture_timestamp == capture)
        {
            break frame;
        }
    };
    let time = displayed(control, frame.drawable)?;
    let record = frame.record.ok_or("missing input attribution")?;
    let latency = time
        .checked_sub(record.input_capture_timestamp)
        .ok_or("presentation precedes capture")?
        .as_secs_f64()
        * 1000.0;
    Ok((frame, latency))
}

pub(super) fn latency(
    control: &MacOsControl,
    rx: &Receiver<Frame>,
    count: usize,
    json: bool,
) -> Result<bool, String> {
    // Startup resize may be submitted before the window is visible. A warm-up
    // pointer frame establishes a real display-link interval and presentation.
    std::thread::sleep(Duration::from_secs(1));
    let (mut previous, _) = sample(control, rx, Point::new(40.0, 40.0))?;
    let interval = previous.timing.refresh_interval;
    if interval.is_zero() {
        return Err("zero display refresh interval".into());
    }
    let mut values = Vec::with_capacity(count);
    let mut submits = Vec::with_capacity(count);
    for i in 0..count {
        // Twenty evenly spaced phases, permuted by a coprime stride. Extrapolate
        // from the last display-link target because the link pauses while idle.
        let phase = ((i * 7) % 20) as f64 / 20.0;
        let anchor = previous.slot;
        let now = control.diagnostic_now();
        let cycles = now.saturating_sub(anchor).as_secs_f64() / interval.as_secs_f64();
        let target = anchor + interval.mul_f64(cycles.floor() + 2.0 + phase);
        let delay = target.saturating_sub(control.diagnostic_now());
        std::thread::sleep(delay);
        let point = Point::new(100.0 + (i % 400) as f64, 100.0 + (i % 200) as f64);
        let (frame, value) = sample(control, rx, point)?;
        if frame.timing.refresh_interval != interval {
            return Err(
                "display refresh interval changed during measurement; rerun on a stable display"
                    .into(),
            );
        }
        submits.push(
            frame
                .record
                .and_then(|r| r.submit_latency())
                .ok_or("invalid submission timestamp")?
                .as_secs_f64()
                * 1000.0,
        );
        previous = frame;
        values.push(value);
    }
    values.sort_by(f64::total_cmp);
    submits.sort_by(f64::total_cmp);
    // Nearest-rank empirical percentiles, including small sample counts.
    let percentile = |percent: usize| values[(count * percent).div_ceil(100) - 1];
    let p50 = percentile(50);
    let p95 = percentile(95);
    let refresh = interval.as_secs_f64() * 1000.0;
    let submit_p95 = submits[(count * 95).div_ceil(100) - 1];
    let pass = submit_p95 <= refresh;
    let result = serde_json::json!({
        "samples": count, "p50_ms": p50, "p95_ms": p95, "max_ms": values[count - 1],
        "refresh_interval_ms": refresh, "p95_frames": p95 / refresh, "pass": pass,
        "submit_p50_ms": submits[(count * 50).div_ceil(100) - 1], "submit_p95_ms": submit_p95,
        "submit_max_ms": submits[count - 1],
        "compositor_offset_ms": control.diagnostic_compositor_offset() * 1000.0,
        "pixel_gate": "requires human-recorded baseline; checked by xtask bench"
    });
    if json {
        println!("{result}");
    } else {
        println!("Input-to-screen diagnostics: {result}");
    }
    Ok(pass)
}

pub(super) fn idle(
    control: &MacOsControl,
    presents: &AtomicU64,
    input_events: &AtomicU64,
    duration: Duration,
    json: bool,
) -> Result<bool, String> {
    // Exclude startup, window activation and display-link disarming.
    std::thread::sleep(Duration::from_secs(1));
    // Drain initial presentation callback records before the measured sleep.
    control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?;
    let initial_inputs = input_events.load(Ordering::SeqCst);
    let initial_frames = presents.load(Ordering::SeqCst);
    let initial_events = control.frame_events().map_err(|e| e.to_string())?;
    let native_start = control.diagnostic_now();
    let cpu_start = control.diagnostic_cpu_time().map_err(|e| e.to_string())?;
    let start = Instant::now();
    std::thread::sleep(duration);
    let elapsed = start.elapsed().as_secs_f64();
    let cpu = control
        .diagnostic_cpu_time()
        .map_err(|e| e.to_string())?
        .checked_sub(cpu_start)
        .ok_or("CPU time moved backwards")?
        .as_secs_f64()
        / elapsed
        * 100.0;
    let native_end = control.diagnostic_now();
    if input_events.load(Ordering::SeqCst) != initial_inputs {
        return Err("user input during idle measurement; rerun without touching the mouse".into());
    }
    let submitted_frames = presents.load(Ordering::SeqCst) - initial_frames;
    let frames = control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|(_, time)| {
            *time >= native_start.as_secs_f64() && *time <= native_end.as_secs_f64()
        })
        .count();
    let events = control.frame_events().map_err(|e| e.to_string())? - initial_events;
    let pass = frames == 0 && submitted_frames == 0 && events == 0 && cpu <= 0.1;
    let result = serde_json::json!({
        "frames": frames, "submitted_frames": submitted_frames, "display_link_events": events, "cpu_percent": cpu,
        "interval_seconds": elapsed, "pass": pass
    });
    if json {
        println!("{result}");
    } else {
        println!("Idle diagnostics: {result}");
    }
    Ok(pass)
}

// Confirm an on-screen transition frame before injecting input. Some displays
// cannot provide that feedback and require manual verification, never a false
// automated pass. Fullscreen latency measurement is deferred to M10.
pub(super) fn fullscreen(
    control: &MacOsControl,
    rx: &Receiver<Frame>,
    json: bool,
) -> Result<bool, String> {
    std::thread::sleep(Duration::from_secs(2));
    let windowed = cadence::measure(control, rx, false)?;
    control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?;
    while rx.try_recv().is_ok() {}
    control
        .diagnostic_toggle_fullscreen()
        .map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(3));
    let transition_ids: std::collections::BTreeSet<_> = rx
        .try_iter()
        .filter(|frame| frame.fullscreen && frame.record.is_none())
        .map(|frame| frame.drawable)
        .collect();
    let feedback = control
        .diagnostic_presented_drawables()
        .map_err(|e| e.to_string())?;
    let displayed_resize = feedback
        .iter()
        .any(|(id, time)| transition_ids.contains(id) && time.is_finite() && *time > 0.0);
    if !displayed_resize {
        return Err("fullscreen transition produced no confirmed on-screen frame without pointer input; visual verification required".into());
    }
    let fullscreen = cadence::measure(control, rx, true)?;
    control
        .diagnostic_toggle_fullscreen()
        .map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(2));
    let restored = cadence::measure(control, rx, false)?;
    let cadence_pass = windowed.cadence_pass && fullscreen.cadence_pass && restored.cadence_pass;
    let paint_submit_within_refresh = windowed.paint_submit_within_refresh
        && fullscreen.paint_submit_within_refresh
        && restored.paint_submit_within_refresh;
    let input_submit_within_refresh = windowed.input_submit_within_refresh
        && fullscreen.input_submit_within_refresh
        && restored.input_submit_within_refresh;
    let result = serde_json::json!({"windowed":windowed,"fullscreen":fullscreen,"restored":restored,"transition_presented":displayed_resize,"visual_verification_required":true,"cadence_pass":cadence_pass,"paint_submit_within_refresh":paint_submit_within_refresh,"input_submit_within_refresh":input_submit_within_refresh,"pixel_latency_measured":false});
    if json {
        println!("{result}");
    } else {
        println!("Fullscreen diagnostics: {result}");
    }
    Ok(cadence_pass && paint_submit_within_refresh)
}
