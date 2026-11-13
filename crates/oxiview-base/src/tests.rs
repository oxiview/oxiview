use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[test]
#[allow(clippy::panic)] // Assertions report geometry contract failures only in this test.
fn geometry_and_affine_contracts() -> Result<(), Error> {
    let point = Point::new(2.0, 3.0);
    let size = Size::new(4.0, 5.0);
    let rect = Rect::from_origin_size(point, size);
    assert_eq!(rect.edges(), [2.0, 3.0, 6.0, 8.0]);
    assert_eq!(rect.origin(), point);
    assert_eq!(rect.size(), size);
    assert_eq!(
        CornerRadii::new(1.0, 2.0, 3.0, 4.0).values(),
        [1.0, 2.0, 3.0, 4.0]
    );
    let scale = Affine::new([2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    let translate = Affine::new([1.0, 0.0, 0.0, 1.0, 5.0, 7.0]);
    let composed = scale.then(translate);
    assert_eq!(composed.transform_point(point), Point::new(9.0, 16.0));
    let round_trip = composed
        .inverse()?
        .transform_point(composed.transform_point(point));
    assert!((round_trip.x() - point.x()).abs() < 1e-12);
    assert!((round_trip.y() - point.y()).abs() < 1e-12);
    for coefficients in [[0.0; 6], [f64::NAN; 6], [f64::INFINITY; 6]] {
        assert_eq!(
            Affine::new(coefficients).inverse(),
            Err(Error::NonInvertibleTransform)
        );
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Assertions check reference color values and round trips.
fn color_conversion() {
    let gray = Color::from_rgba8(128, 128, 128, 64);
    let linear = gray.to_linear();
    assert!((linear[0] - 0.2158605).abs() < 1e-6);
    assert_eq!(linear[3], 64.0 / 255.0);
    for value in [0, 1, 10, 11, 128, 254, 255] {
        let color = Color::from_rgba8(value, value, value, 128);
        let restored = Color::from_linear(color.to_linear());
        for (actual, expected) in restored.to_srgb().into_iter().zip(color.to_srgb()) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
    assert_eq!(Color::from_linear([0.0, 0.0, 0.0, 0.0]).to_srgb(), [0.0; 4]);
    assert_eq!(Color::from_srgb([1.0; 4]).to_linear(), [1.0; 4]);
}

#[test]
#[allow(clippy::panic)] // Assertions check that manual services advance only on caller input.
fn deterministic_services() -> Result<(), Error> {
    let mut clock = ManualClock::new(Duration::from_secs(3));
    assert_eq!(clock.now(), clock.now());
    clock.set(Duration::from_secs(1));
    assert_eq!(clock.now(), Duration::from_secs(1));
    let mut rng = ManualRng::new([7, 42]);
    assert_eq!(rng.next_u64()?, 7);
    assert_eq!(rng.next_u64()?, 42);
    assert_eq!(rng.next_u64(), Err(Error::RandomValuesExhausted));
    rng.push(99);
    assert_eq!(rng.next_u64()?, 99);
    let state = Arc::new(AtomicUsize::new(0));
    let mut scheduler = ManualScheduler::new();
    for digit in [1, 2] {
        let state = Arc::clone(&state);
        scheduler.schedule(Box::new(move || {
            state
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some(v * 10 + digit))
                .ok();
        }));
    }
    assert_eq!(state.load(Ordering::SeqCst), 0);
    assert_eq!(scheduler.pending(), 2);
    assert!(scheduler.run_next());
    assert_eq!(state.load(Ordering::SeqCst), 1);
    assert!(scheduler.run_next());
    assert_eq!(state.load(Ordering::SeqCst), 12);
    assert!(!scheduler.run_next());
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Smoke-test the explicitly selected, test-only system adapters.
fn system_adapters_are_available_to_unit_tests() -> Result<(), Box<dyn std::error::Error>> {
    let clock = system::SystemClock::new();
    let before = clock.now();
    assert!(clock.now() >= before);
    let _value = system::SystemRng.next_u64()?;
    let (sender, receiver) = std::sync::mpsc::channel();
    system::SystemScheduler.schedule(Box::new(move || {
        let _ = sender.send(1);
    }));
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5))?, 1);
    Ok(())
}

struct CountingSubscriber(Arc<AtomicUsize>);
impl tracing::Subscriber for CountingSubscriber {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
#[allow(clippy::panic)] // Assertions verify event delivery, nested installation, and restoration.
fn tracing_initialization_nests_and_restores() {
    let outer_count = Arc::new(AtomicUsize::new(0));
    let inner_count = Arc::new(AtomicUsize::new(0));
    let _outer = init_tracing(CountingSubscriber(Arc::clone(&outer_count)));
    tracing::info!("outer event");
    {
        let _inner = init_tracing(CountingSubscriber(Arc::clone(&inner_count)));
        tracing::info!("inner event");
    }
    tracing::info!("restored outer event");
    assert_eq!(outer_count.load(Ordering::SeqCst), 2);
    assert_eq!(inner_count.load(Ordering::SeqCst), 1);
}
