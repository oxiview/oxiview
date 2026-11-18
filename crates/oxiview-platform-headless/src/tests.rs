use super::*;
use oxiview_base::Size;
use std::sync::{Barrier, mpsc};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn attributes() -> WindowAttributes {
    WindowAttributes {
        title: "headless".into(),
        logical_size: Size::new(100.0, 50.0),
        resize_timeout: ms(100),
    }
}
fn token(event: WindowEvent) -> ResizeToken {
    match event {
        WindowEvent::Resized { token, .. } => token,
        other => panic!("expected resize: {other:?}"),
    }
}
fn run(test: impl FnOnce(Box<dyn WindowEndpoint>, Driver) + Send) {
    let (platform, driver) = HeadlessPlatform::new(ms(10)).unwrap();
    thread::scope(|scope| {
        platform
            .run(|ctx| {
                assert_eq!(ctx.capabilities(), capabilities());
                let mut created = ctx.create_window(attributes())?;
                assert_eq!(created.main.id(), created.endpoint.id());
                // No initial resize can appear before initialization returns.
                assert_eq!(created.endpoint.poll_event()?, None);
                scope.spawn(move || {
                    let mut endpoint = created.endpoint;
                    let initial_event = endpoint.wait_event().unwrap();
                    assert!(matches!(initial_event, WindowEvent::Resized { predicted_presentation_timestamp, refresh_interval, .. } if predicted_presentation_timestamp == ms(10) && refresh_interval == ms(10)));
                    let initial = token(initial_event);
                    endpoint.acknowledge_resize(initial).unwrap();
                    test(endpoint, driver);
                });
                Ok(())
            })
            .unwrap();
    });
}

#[test]
fn idle_and_coalesced_vsync_policy_surface_and_clock() {
    run(|mut e, d| {
        assert_eq!(e.capabilities(), capabilities());
        assert!(e.native_surface_handle().unwrap().is_none());
        d.advance_by(ms(10_000)).unwrap();
        assert_eq!(d.now().unwrap(), e.now());
        assert_eq!(e.poll_event().unwrap(), None);
        assert_eq!(d.counters(e.id()).unwrap().frames, 0);
        e.request_frame().unwrap();
        e.request_frame().unwrap();
        for policy in [FramePolicy::ContinuousWindow, FramePolicy::ContinuousLayer] {
            assert!(
                matches!(e.set_frame_policy(policy), Err(Error::UnsupportedFramePolicy(p)) if p == policy)
            );
        }
        e.set_frame_policy(FramePolicy::OnDemand).unwrap();
        d.advance_by(ms(9)).unwrap();
        assert_eq!(e.poll_event().unwrap(), None);
        d.advance_by(ms(100)).unwrap();
        assert_eq!(
            e.wait_event().unwrap(),
            WindowEvent::Frame {
                predicted_presentation_timestamp: ms(10_010),
                refresh_interval: ms(10),
            }
        );
        assert_eq!(d.counters(e.id()).unwrap().frames, 1);
        e.set_frame_policy(FramePolicy::OnDemand).unwrap();
        d.advance_by(ms(1000)).unwrap();
        assert_eq!(e.poll_event().unwrap(), None);
        e.request_frame().unwrap();
        d.advance_to(ms(11_110)).unwrap();
        assert!(
            matches!(e.wait_event().unwrap(), WindowEvent::Frame { predicted_presentation_timestamp, .. } if predicted_presentation_timestamp == ms(11_110))
        );
    });
}

#[test]
fn injected_events_preserve_capture_time_and_fifo() {
    run(|mut e, d| {
        let p = Point::new(2.0, 3.0);
        d.advance_to(ms(20)).unwrap();
        d.pointer_moved(e.id(), p, Some(ms(3))).unwrap();
        d.pointer_button(e.id(), PointerButton::Primary, true, p, None)
            .unwrap();
        d.key(e.id(), KeyCode::KeyA, true, true, Some(ms(2)))
            .unwrap();
        d.request_redraw(e.id()).unwrap();
        d.request_user_close(e.id()).unwrap();
        assert_eq!(
            e.wait_event().unwrap(),
            WindowEvent::PointerMoved {
                position: p,
                timestamp: ms(3)
            }
        );
        assert_eq!(
            e.wait_event().unwrap(),
            WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed: true,
                position: p,
                timestamp: ms(20)
            }
        );
        assert_eq!(
            e.wait_event().unwrap(),
            WindowEvent::Key {
                code: KeyCode::KeyA,
                pressed: true,
                repeat: true,
                timestamp: ms(2)
            }
        );
        assert_eq!(e.wait_event().unwrap(), WindowEvent::RedrawRequested);
        assert_eq!(e.wait_event().unwrap(), WindowEvent::CloseRequested);
        assert_eq!(e.poll_event().unwrap(), None);
        assert_eq!(d.counters(e.id()).unwrap().frames, 0);
    });
}

#[test]
fn pointer_exit_and_interruption_remain_distinct_and_preserve_fifo() {
    run(|mut endpoint, driver| {
        let id = endpoint.id();
        let outside = Point::new(-24.0, 85.0);
        driver.advance_to(ms(20)).unwrap();
        driver.pointer_left(id, Some(ms(4))).unwrap();
        driver.pointer_moved(id, outside, Some(ms(5))).unwrap();
        driver
            .pointer_button(id, PointerButton::Primary, false, outside, Some(ms(6)))
            .unwrap();
        driver.activation_changed(id, false, Some(ms(7))).unwrap();
        driver.visibility_changed(id, false, Some(ms(8))).unwrap();
        driver.pointer_interrupted(id, None).unwrap();
        driver.visibility_changed(id, true, None).unwrap();
        driver.activation_changed(id, true, None).unwrap();

        let expected = [
            WindowEvent::PointerLeft { timestamp: ms(4) },
            WindowEvent::PointerMoved {
                position: outside,
                timestamp: ms(5),
            },
            WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed: false,
                position: outside,
                timestamp: ms(6),
            },
            WindowEvent::ActivationChanged {
                active: false,
                timestamp: ms(7),
            },
            WindowEvent::VisibilityChanged {
                visible: false,
                timestamp: ms(8),
            },
            WindowEvent::PointerInterrupted { timestamp: ms(20) },
            WindowEvent::VisibilityChanged {
                visible: true,
                timestamp: ms(20),
            },
            WindowEvent::ActivationChanged {
                active: true,
                timestamp: ms(20),
            },
        ];
        for event in expected {
            assert_eq!(endpoint.wait_event().unwrap(), event);
        }
        assert_eq!(endpoint.poll_event().unwrap(), None);
        assert_eq!(driver.counters(id).unwrap().frames, 0);
    });
}

#[test]
fn lifecycle_input_is_rejected_after_window_destruction() {
    run(|mut endpoint, driver| {
        let id = endpoint.id();
        endpoint.request_close().unwrap();
        assert_eq!(endpoint.wait_event().unwrap(), WindowEvent::Destroyed);
        for result in [
            driver.pointer_left(id, None),
            driver.activation_changed(id, false, None),
            driver.visibility_changed(id, false, None),
            driver.pointer_interrupted(id, None),
        ] {
            assert!(matches!(
                result,
                Err(DriverError::Platform(Error::WindowDestroyed))
            ));
        }
    });
}

#[test]
fn resize_ack_supersession_zero_size_and_main_thread_wait() {
    let (platform, d) = HeadlessPlatform::new(ms(10)).unwrap();
    thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        let controller = d.clone();
        scope.spawn(move || {
            let (id, initial) = rx.recv().unwrap();
            controller.wait_until_blocked(id, WaitKind::Resize).unwrap();
            // Frame requests remain serviceable while the simulated main thread waits.
            controller.advance_by(ms(10)).unwrap();
            assert_eq!(
                controller.wait_resize(id, initial).unwrap(),
                ResizeOutcome::Acknowledged
            );
        });
        platform
            .run(|ctx| {
                let created = ctx.create_window(attributes())?;
                let id = created.main.id();
                // Exercise a blocking main-thread wait after startup using a separate
                // controller designated as main for resize, as the port has no pump callback.
                let d = d.clone();
                scope.spawn(move || {
                    let mut e = created.endpoint;
                    let initial = token(e.wait_event().unwrap());
                    let waiter = d.clone();
                    e.request_frame().unwrap();
                    thread::scope(|scope| {
                        scope.spawn(move || {
                            tx.send((id, initial)).unwrap();
                            assert_eq!(
                                waiter.wait_resize(id, initial).unwrap(),
                                ResizeOutcome::Acknowledged
                            );
                        });
                        d.wait_until_blocked(id, WaitKind::Resize).unwrap();
                        assert!(matches!(e.wait_event().unwrap(), WindowEvent::Frame { .. }));
                        e.acknowledge_resize(initial).unwrap();
                    });
                    assert!(matches!(
                        e.acknowledge_resize(initial),
                        Err(Error::InvalidResizeToken)
                    ));
                    assert!(matches!(
                        e.acknowledge_resize(ResizeToken::new(WindowId::new(999), 1)),
                        Err(Error::InvalidResizeToken)
                    ));
                    let size = PhysicalSize {
                        width: 0,
                        height: 0,
                    };
                    let first = d.resize(id, size, 2.0).unwrap();
                    assert_eq!(
                        e.wait_event().unwrap(),
                        WindowEvent::ScaleFactorChanged { scale_factor: 2.0 }
                    );
                    assert_eq!(
                        e.wait_event().unwrap(),
                        WindowEvent::Resized {
                            display_sync: false,
                            physical_size: size,
                            scale_factor: 2.0,
                            predicted_presentation_timestamp: ms(20),
                            refresh_interval: ms(10),
                            token: first
                        }
                    );
                    let second = d.resize(id, size, 2.0).unwrap();
                    assert_ne!(first, second);
                    assert_eq!(d.wait_resize(id, first).unwrap(), ResizeOutcome::Superseded);
                    assert!(matches!(
                        e.acknowledge_resize(first),
                        Err(Error::InvalidResizeToken)
                    ));
                    assert_eq!(token(e.wait_event().unwrap()), second);
                    e.acknowledge_resize(second).unwrap();
                    assert_eq!(
                        d.resize_outcome(id, second).unwrap(),
                        Some(ResizeOutcome::Acknowledged)
                    );
                    assert_eq!(d.counters(id).unwrap().resize_acknowledgements, 2);
                });
                Ok(())
            })
            .unwrap();
    });
}

#[test]
fn virtual_timeout_releases_wait_and_invalidates_token() {
    run(|mut e, d| {
        let id = e.id();
        let t = d
            .resize(
                id,
                PhysicalSize {
                    width: 20,
                    height: 30,
                },
                1.0,
            )
            .unwrap();
        assert_eq!(token(e.wait_event().unwrap()), t);
        thread::scope(|scope| {
            let waiter = d.clone();
            let handle = scope.spawn(move || waiter.wait_resize(id, t).unwrap());
            d.wait_until_blocked(id, WaitKind::Resize).unwrap();
            d.advance_by(ms(99)).unwrap();
            assert_eq!(d.resize_outcome(id, t).unwrap(), None);
            d.advance_by(ms(1)).unwrap();
            assert_eq!(handle.join().unwrap(), ResizeOutcome::TimedOut);
        });
        assert_eq!(d.counters(id).unwrap().resize_timeouts, 1);
        assert!(matches!(
            e.acknowledge_resize(t),
            Err(Error::InvalidResizeToken)
        ));
        d.advance_by(ms(1000)).unwrap();
        assert_eq!(d.counters(id).unwrap().resize_timeouts, 1);
    });
}

#[test]
fn deadline_waits_virtual_time_event_precedence_and_termination() {
    run(|mut e, d| {
        let id = e.id();
        thread::scope(|scope| {
            let d2 = d.clone();
            scope.spawn(move || {
                d2.wait_until_blocked(id, WaitKind::Event).unwrap();
                d2.advance_to(ms(25)).unwrap();
            });
            assert_eq!(e.wait_event_until(ms(25)).unwrap(), None);
        });
        thread::scope(|scope| {
            let d2 = d.clone();
            scope.spawn(move || {
                d2.wait_until_blocked(id, WaitKind::Event).unwrap();
                d2.request_redraw(id).unwrap();
            });
            assert_eq!(
                e.wait_event_until(ms(100)).unwrap(),
                Some(WindowEvent::RedrawRequested)
            );
        });
        d.request_redraw(id).unwrap();
        assert_eq!(
            e.wait_event_until(Duration::ZERO).unwrap(),
            Some(WindowEvent::RedrawRequested)
        );
        e.request_close().unwrap();
        e.request_close().unwrap();
        assert_eq!(
            e.wait_event_until(Duration::ZERO).unwrap(),
            Some(WindowEvent::Destroyed)
        );
        assert!(matches!(
            e.wait_event_until(Duration::ZERO),
            Err(Error::WindowDestroyed)
        ));
        assert!(matches!(e.wait_event(), Err(Error::WindowDestroyed)));
        assert!(matches!(e.poll_event(), Err(Error::WindowDestroyed)));
        assert!(matches!(
            e.native_surface_handle(),
            Err(Error::WindowDestroyed)
        ));
        assert!(matches!(e.request_frame(), Err(Error::WindowDestroyed)));
        assert!(matches!(e.waker().wake(), Err(Error::WindowDestroyed)));
    });
}

#[test]
fn cross_thread_wakes_coalesce_and_unblock_both_waits() {
    run(|mut e, d| {
        let id = e.id();
        let w = e.waker();
        thread::scope(|scope| {
            scope.spawn(|| {
                d.wait_until_blocked(id, WaitKind::Event).unwrap();
                w.wake().unwrap();
            });
            assert_eq!(e.wait_event().unwrap(), WindowEvent::Wake);
        });
        let barrier = Arc::new(Barrier::new(3));
        thread::scope(|scope| {
            for w in [w.clone(), e.waker()] {
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    w.wake().unwrap();
                });
            }
            barrier.wait();
        });
        assert_eq!(e.poll_event().unwrap(), Some(WindowEvent::Wake));
        assert_eq!(e.poll_event().unwrap(), None);
        thread::scope(|scope| {
            scope.spawn(|| {
                d.wait_until_blocked(id, WaitKind::Event).unwrap();
                w.wake().unwrap();
            });
            assert_eq!(
                e.wait_event_until(ms(100)).unwrap(),
                Some(WindowEvent::Wake)
            );
        });
        drop(e);
        assert!(matches!(w.wake(), Err(Error::WindowDestroyed)));
    });
}

#[test]
fn main_close_exit_and_initialization_error_cleanup() {
    for mode in 0..3 {
        let (p, _) = HeadlessPlatform::new(ms(10)).unwrap();
        thread::scope(|scope| {
            let result = p.run(|ctx| {
                let mut first = ctx.create_window(attributes())?;
                let second = ctx.create_window(attributes())?;
                assert_ne!(first.main.id(), second.main.id());
                if mode == 0 {
                    first.main.request_close()?;
                    first.main.request_close()?;
                    let mut main = second.main;
                    main.request_close()?;
                } else if mode == 1 {
                    ctx.request_exit()?;
                    ctx.request_exit()?;
                    assert!(ctx.create_window(attributes()).is_err());
                }
                for mut e in [first.endpoint, second.endpoint] {
                    scope.spawn(move || {
                        assert_eq!(e.wait_event().unwrap(), WindowEvent::Destroyed);
                        assert!(matches!(e.wait_event(), Err(Error::WindowDestroyed)));
                    });
                }
                if mode == 2 {
                    Err(Error::OperationFailed("initialization failed".into()))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), mode == 2);
        });
    }
}

#[test]
fn invalid_configuration_and_clock_are_rejected_without_changes() {
    assert!(HeadlessPlatform::new(Duration::ZERO).is_err());
    let (p, _) = HeadlessPlatform::new(ms(10)).unwrap();
    p.run(|ctx| {
        for mut a in [attributes(), attributes(), attributes()] {
            a.title = "bad\0title".into();
            assert!(matches!(
                ctx.create_window(a),
                Err(Error::InvalidWindowAttributes(_))
            ));
        }
        let mut a = attributes();
        a.logical_size = Size::new(f64::NAN, 0.0);
        assert!(ctx.create_window(a).is_err());
        let mut a = attributes();
        a.resize_timeout = Duration::ZERO;
        assert!(ctx.create_window(a).is_err());
        Ok(())
    })
    .unwrap();
    run(|e, d| {
        d.advance_by(ms(2)).unwrap();
        assert!(d.advance_to(ms(1)).is_err());
        assert!(d.advance_by(Duration::MAX).is_err());
        assert_eq!(d.now().unwrap(), ms(2));
        assert!(
            d.resize(
                e.id(),
                PhysicalSize {
                    width: 1,
                    height: 1
                },
                f64::NAN
            )
            .is_err()
        );
    });
}

#[test]
fn poisoned_state_is_an_explicit_error() {
    let (p, d) = HeadlessPlatform::new(ms(10)).unwrap();
    thread::scope(|scope| {
        let shared = d.shared.clone();
        assert!(
            scope
                .spawn(move || {
                    let _guard = shared.state.lock().unwrap();
                    panic!("intentional poison");
                })
                .join()
                .is_err()
        );
    });
    assert!(matches!(d.now(), Err(DriverError::Poisoned)));
    assert!(matches!(p.run(|_| Ok(())), Err(Error::OperationFailed(_))));
}

#[test]
fn timeout_trace_contains_window_token_and_timeout() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Capture(Arc<AtomicBool>);
    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Fields(Vec<String>);
            impl tracing::field::Visit for Fields {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push(format!("{}={value:?}", field.name()));
                }
            }
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            let text = fields.0.join(" ");
            if text.contains("window=WindowId")
                && text.contains("token=ResizeToken")
                && text.contains("timeout=100ms")
                && text.contains("headless resize timed out")
            {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    run(|mut e, d| {
        let captured = Arc::new(AtomicBool::new(false));
        let dispatch = tracing::Dispatch::new(Capture(captured.clone()));
        let t = d
            .resize(
                e.id(),
                PhysicalSize {
                    width: 1,
                    height: 1,
                },
                1.0,
            )
            .unwrap();
        assert_eq!(token(e.wait_event().unwrap()), t);
        tracing::dispatcher::with_default(&dispatch, || d.advance_by(ms(100)).unwrap());
        assert!(captured.load(Ordering::SeqCst));
    });
}

#[test]
fn pending_wait_is_released_by_destruction() {
    let (p, d) = HeadlessPlatform::new(ms(10)).unwrap();
    thread::scope(|scope| {
        p.run(|ctx| {
            let created = ctx.create_window(attributes())?;
            let mut main = created.main;
            let id = main.id();
            scope.spawn(move || {
                let mut e = created.endpoint;
                assert_eq!(
                    e.wait_event_until(ms(100)).unwrap(),
                    Some(WindowEvent::Destroyed)
                );
                assert!(matches!(
                    e.wait_event_until(ms(100)),
                    Err(Error::WindowDestroyed)
                ));
            });
            d.wait_until_blocked(id, WaitKind::Event).unwrap();
            main.request_close()?;
            Ok(())
        })
        .unwrap();
    });
}

#[test]
fn dropping_main_alone_keeps_window_alive_and_drop_endpoint_closes_loop() {
    let (p, d) = HeadlessPlatform::new(ms(10)).unwrap();
    let (tx, rx) = mpsc::channel();
    thread::scope(|scope| {
        p.run(|ctx| {
            let created = ctx.create_window(attributes())?;
            drop(created.main);
            scope.spawn(move || {
                let mut e = created.endpoint;
                let initial = token(e.wait_event().unwrap());
                e.acknowledge_resize(initial).unwrap();
                d.request_redraw(e.id()).unwrap();
                assert_eq!(e.wait_event().unwrap(), WindowEvent::RedrawRequested);
                tx.send(e.waker()).unwrap();
                // Endpoint drop alone ends the loop.
            });
            Ok(())
        })
        .unwrap();
    });
    assert!(matches!(
        rx.recv().unwrap().wake(),
        Err(Error::WindowDestroyed)
    ));
}

#[test]
fn resize_prediction_between_and_on_boundaries() {
    run(|mut e, d| {
        for (now, prediction) in [(ms(3), ms(10)), (ms(10), ms(20))] {
            d.advance_to(now).unwrap();
            let token = d
                .resize(
                    e.id(),
                    PhysicalSize {
                        width: 12,
                        height: 8,
                    },
                    1.0,
                )
                .unwrap();
            assert!(
                matches!(e.wait_event().unwrap(), WindowEvent::Resized { predicted_presentation_timestamp, refresh_interval, .. } if predicted_presentation_timestamp == prediction && refresh_interval == ms(10))
            );
            e.acknowledge_resize(token).unwrap();
        }
    });
}
