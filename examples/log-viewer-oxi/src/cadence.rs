//! Assigns actual Metal display timestamps to refresh slots in a fixed capture interval.

use serde::Serialize;
use std::collections::BTreeSet;

pub(crate) const WARMUP_NS: u64 = 2_000_000_000;
pub(crate) const SAMPLE_NS: u64 = 10_000_000_000;
pub(crate) const STRIDE: f64 = 12.0;
pub(crate) const WINDOW_WIDTH: f64 = 1120.0;
pub(crate) const WINDOW_HEIGHT: f64 = 720.0;

#[derive(Debug, Serialize)]
pub(crate) struct Frame {
    pub drawable: u64,
    pub paint_sequence: u64,
    pub target_ns: u64,
    pub scene_fingerprint: u64,
    pub submitted_ns: u64,
    pub refresh_ns: u64,
    pub first_key: u64,
    pub offset_y: f64,
    pub row_top_y: f64,
    pub presented_ns: Option<u64>,
}
#[derive(Serialize)]
pub(crate) struct Report {
    pub rows: usize,
    pub warmup_seconds: u64,
    pub sample_seconds: u64,
    pub stride_points: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub scale_factor: f64,
    pub phase_ns: u64,
    pub start_ns: u64,
    pub end_ns: u64,
    pub interval_ns: u64,
    pub refresh_interval_ms: f64,
    pub expected_slots: u64,
    pub occupied_slots: u64,
    pub displayed_frames: usize,
    pub submitted_frames: usize,
    pub dropped_frames: u64,
    pub missing_feedback: usize,
    pub zero_timestamps: usize,
    pub first_key: Option<u64>,
    pub last_key: Option<u64>,
    pub valid: bool,
    pub pass: bool,
    pub frames: Vec<Frame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    pub display_ticks_ns: Vec<u64>,
}

#[derive(Serialize)]
pub(crate) struct Display {
    pub id: u32,
    pub built_in: bool,
}
#[derive(Clone, Copy)]
pub(crate) struct Window {
    phase: u64,
    pub start: u64,
    pub end: u64,
    period: u64,
    count: u64,
}
impl Window {
    pub fn new(phase: u64, period: u64) -> Option<Self> {
        if !(1_000_000..=100_000_000).contains(&period) {
            return None;
        }
        let start = phase.checked_add(WARMUP_NS.div_ceil(period).checked_mul(period)?)?;
        let count = (SAMPLE_NS + period / 2) / period;
        Some(Self {
            phase,
            start,
            end: start.checked_add(count.checked_mul(period)?)?,
            period,
            count,
        })
    }
    fn display_slot(self, t: u64) -> Option<u64> {
        // Bin around the calibrated refresh phase, including the two edge slots.
        let slot = t.checked_add(self.period / 2)?.checked_sub(self.start)? / self.period;
        (slot < self.count).then_some(slot)
    }
    pub fn report(
        self,
        frames: Vec<Frame>,
        width: u32,
        height: u32,
        scale: f64,
        rows: usize,
    ) -> Report {
        let mut displayed: Vec<_> = frames
            .iter()
            .filter_map(|f| {
                let t = f.presented_ns?;
                self.display_slot(t).map(|slot| (t, slot, f))
            })
            .collect();
        displayed.sort_by_key(|(t, _, _)| *t);
        let occupied: BTreeSet<_> = displayed.iter().map(|(_, slot, _)| *slot).collect();
        let measured: Vec<_> = frames
            .iter()
            .filter(|f| (self.start..self.end).contains(&f.submitted_ns))
            .collect();
        let missing = measured.iter().filter(|f| f.presented_ns.is_none()).count();
        let zero = measured
            .iter()
            .filter(|f| f.presented_ns == Some(0))
            .count();
        let first_key = displayed.first().map(|(_, _, f)| f.first_key);
        let last_key = displayed.last().map(|(_, _, f)| f.first_key);
        let stable = measured
            .iter()
            .all(|f| f.refresh_ns.abs_diff(self.period) <= self.period / 1000);
        let advanced = displayed.windows(2).all(|w| {
            let steps = (w[1].2.offset_y - w[0].2.offset_y) / STRIDE;
            steps >= 1.0
                && (steps - steps.round()).abs() <= 1e-9
                && w[1].2.first_key >= w[0].2.first_key
        });
        let positions = displayed.iter().all(|(_, _, f)| {
            f.paint_sequence > 0
                && f.offset_y == f.paint_sequence as f64 * STRIDE
                && f.first_key == (f.offset_y / crate::view::LIST_HEIGHT) as u64 + 1
                && f.row_top_y == (f.first_key - 1) as f64 * crate::view::LIST_HEIGHT - f.offset_y
        });
        let scenes: BTreeSet<_> = displayed
            .iter()
            .map(|(_, _, f)| f.scene_fingerprint)
            .collect();
        let complete = frames.first().is_some_and(|f| f.submitted_ns < self.start)
            && frames
                .last()
                .is_some_and(|f| f.submitted_ns >= self.end + self.period);
        let valid = complete
            && stable
            && rows == 100_000
            && (f64::from(width) - WINDOW_WIDTH * scale).abs() <= 0.5
            && (f64::from(height) - WINDOW_HEIGHT * scale).abs() <= 0.5
            && scale.is_finite()
            && scale > 0.
            && missing == 0
            && !measured.is_empty()
            && advanced
            && positions
            && scenes.len() == displayed.len()
            && first_key.zip(last_key).is_some_and(|(a, b)| b > a);
        let occupied_slots = occupied.len() as u64;
        let dropped = self.count - occupied_slots;
        Report {
            rows,
            warmup_seconds: 2,
            sample_seconds: 10,
            stride_points: STRIDE,
            pixel_width: width,
            pixel_height: height,
            scale_factor: scale,
            phase_ns: self.phase,
            start_ns: self.start,
            end_ns: self.end,
            interval_ns: self.period,
            refresh_interval_ms: self.period as f64 / 1e6,
            expected_slots: self.count,
            occupied_slots,
            displayed_frames: displayed.len(),
            submitted_frames: measured.len(),
            dropped_frames: dropped,
            missing_feedback: missing,
            zero_timestamps: zero,
            first_key,
            last_key,
            valid,
            pass: valid && dropped == 0 && zero == 0,
            frames,
            display: None,
            display_ticks_ns: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const DT: u64 = 10_000_000;
    fn fixture() -> (Window, Vec<Frame>) {
        let w = Window::new(DT, DT).expect("test cadence");
        let frames = (1..=w.end / DT + 4)
            .map(|n| Frame {
                drawable: n,
                paint_sequence: n,
                target_ns: n * DT,
                scene_fingerprint: n,
                refresh_ns: DT,
                submitted_ns: n * DT - 1_000_000,
                presented_ns: Some(n * DT),
                first_key: n * 12 / 34 + 1,
                offset_y: n as f64 * 12.,
                row_top_y: -(((n * 12) % 34) as f64),
            })
            .collect();
        (w, frames)
    }
    #[test]
    fn full_interval_counts_edges_and_internal_gap() {
        let (w, f) = fixture();
        assert!(w.report(f, 2240, 1440, 2., 100000).pass);
        for when in [w.start, w.start + 500 * DT, w.end - DT] {
            let (_, mut f) = fixture();
            f.retain(|f| f.presented_ns != Some(when));
            let r = w.report(f, 2240, 1440, 2., 100000);
            assert!(r.valid);
            assert_eq!(r.dropped_frames, 1);
            assert!(!r.pass);
        }
    }
    #[test]
    fn rejects_incomplete_feedback_refresh_change_and_stationary_content() {
        for case in 0..7 {
            let (w, mut f) = fixture();
            let n = f.len() / 2;
            match case {
                0 => f[n].presented_ns = None,
                1 => f[n].refresh_ns = 2 * DT,
                2 => f[n].offset_y = f[n - 1].offset_y,
                3 => f.truncate(n),
                4 => f[n].row_top_y += 1.0,
                5 => f[n].scene_fingerprint = f[n - 1].scene_fingerprint,
                _ => f[n].paint_sequence += 1,
            };
            let r = w.report(f, 2240, 1440, 2., 100000);
            assert!(!r.valid && !r.pass);
        }
    }
}
