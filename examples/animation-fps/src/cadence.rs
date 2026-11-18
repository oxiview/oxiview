//! Counts actual submissions and displayed drawables over the same clock interval.
use std::{collections::VecDeque, time::Duration};

#[derive(Clone, Copy, Debug)]
pub(super) struct Submission {
    pub id: u64,
    pub at: Duration,
    pub paint_to_submit: Duration,
}

#[derive(Clone, Copy, Debug)]
enum Display {
    Pending,
    Skipped,
    At(Duration),
}

#[derive(Default)]
pub(super) struct Timeline(VecDeque<(Submission, Display)>);

#[derive(Clone, Copy, Debug)]
#[cfg_attr(target_os = "macos", derive(serde::Serialize))]
pub(super) struct Rates {
    pub submitted_frames: usize,
    pub displayed_frames: usize,
    pub submitted_fps: f64,
    pub displayed_fps: Option<f64>,
    pub display_gap_p95_ms: Option<f64>,
    pub paint_to_submit_p95_ms: Option<f64>,
    pub submissions_not_displayed: usize,
}

impl Timeline {
    pub fn submit(&mut self, frame: Submission) {
        self.0.push_back((frame, Display::Pending));
    }

    pub fn feedback(&mut self, records: &[(u64, Duration)]) {
        for &(id, time) in records {
            // Metal callbacks can arrive out of order. An unknown ID can belong
            // to a previous segment or a drawable without a successful submission.
            if let Some((_, state)) = self.0.iter_mut().rev().find(|(frame, _)| frame.id == id)
                && matches!(state, Display::Pending)
            {
                *state = if time.is_zero() {
                    Display::Skipped
                } else {
                    Display::At(time)
                };
            }
        }
    }

    pub fn trim_before(&mut self, earliest: Duration) {
        while self.0.front().is_some_and(|(frame, _)| frame.at < earliest) {
            self.0.pop_front();
        }
    }

    pub fn has_pending_before(&self, end: Duration) -> bool {
        self.0
            .iter()
            .any(|(frame, state)| frame.at < end && matches!(state, Display::Pending))
    }

    pub fn rates(&self, start: Duration, end: Duration) -> Option<Rates> {
        let seconds = end.checked_sub(start)?.as_secs_f64();
        if seconds == 0.0 {
            return None;
        }
        let window = start..end;
        let mut paint = Vec::new();
        let mut displayed = Vec::new();
        let mut skipped = 0;
        let mut received_feedback = false;
        for (frame, state) in &self.0 {
            if window.contains(&frame.at) {
                paint.push(frame.paint_to_submit);
                received_feedback |= !matches!(state, Display::Pending);
                if matches!(state, Display::Skipped) {
                    skipped += 1;
                }
            }
            if let Display::At(at) = state
                && window.contains(at)
            {
                displayed.push(*at);
            }
        }
        displayed.sort_unstable();
        let mut gaps: Vec<_> = displayed.windows(2).map(|pair| pair[1] - pair[0]).collect();
        Some(Rates {
            submitted_frames: paint.len(),
            displayed_frames: displayed.len(),
            submitted_fps: paint.len() as f64 / seconds,
            displayed_fps: (!displayed.is_empty() || received_feedback)
                .then_some(displayed.len() as f64 / seconds),
            display_gap_p95_ms: p95_ms(&mut gaps),
            paint_to_submit_p95_ms: p95_ms(&mut paint),
            submissions_not_displayed: skipped,
        })
    }
}

fn p95_ms(values: &mut [Duration]) -> Option<f64> {
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).checked_sub(1)?;
    Some(values.get(index)?.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
    fn sample(id: u64, at: u64) -> Submission {
        Submission {
            id,
            at: ms(at),
            paint_to_submit: ms(3),
        }
    }

    #[test]
    fn out_of_order_feedback_crosses_window_edges_and_counts_real_displays()
    -> Result<(), &'static str> {
        let mut log = Timeline::default();
        for (id, at) in [(1, 990), (2, 1200), (3, 1600), (4, 1700), (5, 2000)] {
            log.submit(sample(id, at));
        }
        log.feedback(&[
            (4, ms(2010)),
            (2, Duration::ZERO),
            (3, ms(1660)),
            (1, ms(1000)),
            (3, ms(1660)),
        ]);
        let rate = log.rates(ms(1000), ms(2000)).ok_or("missing measurement")?;
        assert_eq!((rate.submitted_frames, rate.displayed_frames), (3, 2));
        assert_eq!((rate.submitted_fps, rate.displayed_fps), (3.0, Some(2.0)));
        assert_eq!(rate.display_gap_p95_ms, Some(660.0));
        assert_eq!(rate.paint_to_submit_p95_ms, Some(3.0));
        assert_eq!(rate.submissions_not_displayed, 1);
        assert!(!log.has_pending_before(ms(2000)));
        assert!(log.has_pending_before(ms(2001)));
        Ok(())
    }

    #[test]
    fn missing_feedback_never_yields_a_fabricated_display_rate() -> Result<(), &'static str> {
        let mut log = Timeline::default();
        log.submit(sample(0, 12));
        let rates = log
            .rates(Duration::ZERO, ms(1000))
            .ok_or("missing measurement")?;
        assert_eq!(rates.submitted_fps, 1.0);
        assert_eq!(rates.displayed_fps, None);
        assert_eq!(rates.display_gap_p95_ms, None);
        log.feedback(&[(0, Duration::ZERO)]);
        assert_eq!(
            log.rates(Duration::ZERO, ms(1000))
                .ok_or("missing measurement")?
                .displayed_fps,
            Some(0.0)
        );
        log.trim_before(ms(13));
        assert_eq!(
            log.rates(Duration::ZERO, ms(1000))
                .ok_or("missing measurement")?
                .submitted_frames,
            0
        );
        Ok(())
    }
}
