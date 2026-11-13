use super::*;
fn timing(kind: FrameKind) -> FrameTiming {
    FrameTiming {
        predicted_presentation_timestamp: Duration::from_millis(20),
        refresh_interval: Duration::from_millis(10),
        kind,
    }
}
#[test]
fn idle_invalidation_redraw_and_coalescing() {
    let mut s = Scheduler::default();
    assert_eq!(s.schedule(), Schedule::default());
    s.invalidate();
    assert!(s.schedule().request_frame);
    s.redraw();
    assert!(!s.schedule().request_frame);
    assert!(s.frame());
    assert_eq!(s.produced(timing(FrameKind::Vsync)), None);
    assert_eq!(s.schedule(), Schedule::default());
    assert!(!s.frame());
    s.redraw();
    assert!(s.schedule().request_frame);
}
#[test]
fn earliest_deadline_expires_once() {
    let mut s = Scheduler::default();
    s.request_at(Duration::from_millis(30));
    s.request_at(Duration::from_millis(10));
    assert_eq!(s.schedule().deadline, Some(Duration::from_millis(10)));
    s.deadline_reached(Duration::from_millis(9));
    assert!(!s.schedule().request_frame);
    s.deadline_reached(Duration::from_millis(10));
    assert_eq!(
        s.schedule(),
        Schedule {
            request_frame: true,
            deadline: None
        }
    );
}
#[test]
fn resize_consumes_input_without_duplicate_vsync_and_zero_suspends() {
    let mut s = Scheduler::default();
    s.invalidate();
    s.input(Duration::from_millis(8));
    s.input(Duration::from_millis(3));
    assert!(s.schedule().request_frame);
    assert!(s.resize(PhysicalSize {
        width: 8,
        height: 8
    }));
    let record = s.produced(timing(FrameKind::Resize));
    assert_eq!(
        record.map(|r| r.latency()),
        Some(Some(Duration::from_millis(17)))
    );
    assert!(!s.frame());
    assert!(!s.resize(PhysicalSize {
        width: 0,
        height: 8
    }));
    s.request_at(Duration::ZERO);
    s.input(Duration::from_millis(6));
    assert_eq!(s.schedule(), Schedule::default());
    assert!(!s.frame());
    assert!(s.resize(PhysicalSize {
        width: 8,
        height: 8
    }));
    assert!(s.produced(timing(FrameKind::Resize)).is_some());
    assert!(s.produced(timing(FrameKind::Vsync)).is_none());
}
#[test]
fn input_that_does_not_invalidate_is_not_attributed() {
    let mut s = Scheduler::default();
    s.input(Duration::from_millis(1));
    s.invalidate();
    s.input(Duration::from_millis(5));
    let record = s.produced(timing(FrameKind::Vsync));
    assert_eq!(
        record.map(|r| r.input_capture_timestamp),
        Some(Duration::from_millis(5))
    );
}
