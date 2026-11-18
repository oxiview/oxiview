//! Canceling a pointer press must reset the bound visual state without clicking.

use oxiview_base::Point;
use oxiview_core::Scheduler;
use oxiview_platform::{PhysicalSize, PointerButton, WindowEvent};
use oxiview_scene::EncodedNode;
use snake::{
    application::{Action, Config, Counters, SnakeApp},
    game::Phase,
};
use std::{
    error::Error,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn has_pressed_opacity(app: &mut SnakeApp, config: Config) -> TestResult<bool> {
    let painted = app.render(
        PhysicalSize {
            width: config.width,
            height: config.height,
        },
        1.0,
    )?;
    Ok(painted
        .scene
        .encode(painted.root)?
        .nodes
        .iter()
        .any(|node| matches!(node,EncodedNode::BeginOpacity(value) if (*value-0.78).abs()<1e-9)))
}
fn pointer_button(position: Point, pressed: bool, time: u64) -> WindowEvent {
    WindowEvent::PointerButton {
        button: PointerButton::Primary,
        pressed,
        position,
        timestamp: Duration::from_millis(time),
    }
}
fn canceled_press(interruption: WindowEvent, restore_activation: bool) -> TestResult<()> {
    let config = Config::default();
    let counters = Arc::new(Counters::default());
    let mut app = SnakeApp::new(config, Arc::clone(&counters))?;
    let mut scheduler = Scheduler::default();
    assert!(!has_pressed_opacity(&mut app, config)?);
    let bounds = app.control_bounds(Action::Primary)?;
    let position = Point::new(
        bounds.origin().x() + bounds.size().width() / 2.0,
        bounds.origin().y() + bounds.size().height() / 2.0,
    );
    app.process(
        &WindowEvent::PointerMoved {
            position,
            timestamp: Duration::from_millis(1),
        },
        &mut scheduler,
    )?;
    app.process(&pointer_button(position, true, 2), &mut scheduler)?;
    assert_eq!(app.snapshot().phase, Phase::Ready);
    assert!(
        has_pressed_opacity(&mut app, config)?,
        "a real primary-button Down must show the pressed style"
    );

    app.process(&interruption, &mut scheduler)?;
    assert_eq!(app.snapshot().phase, Phase::Ready);
    assert!(
        !has_pressed_opacity(&mut app, config)?,
        "Cancel must remove the pressed opacity even when phase/difficulty are unchanged"
    );
    if restore_activation {
        app.process(
            &WindowEvent::ActivationChanged {
                active: true,
                timestamp: Duration::from_millis(4),
            },
            &mut scheduler,
        )?;
    }
    app.process(&pointer_button(position, false, 5), &mut scheduler)?;
    assert_eq!(
        app.snapshot().phase,
        Phase::Ready,
        "Up following a canceled press must not activate Start"
    );
    assert_eq!(app.next_deadline(), None);
    assert!(!has_pressed_opacity(&mut app, config)?);

    app.process(&pointer_button(position, true, 6), &mut scheduler)?;
    app.process(&pointer_button(position, false, 7), &mut scheduler)?;
    assert_eq!(
        app.snapshot().phase,
        Phase::Running,
        "the control must accept a subsequent complete click"
    );
    assert!(app.next_deadline().is_some());
    assert!(!has_pressed_opacity(&mut app, config)?);
    assert_eq!(counters.rejected_commands.load(Ordering::SeqCst), 0);
    assert_eq!(counters.screen_mounts.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn losing_activation_cancels_ready_button_visuals_without_starting() -> TestResult<()> {
    canceled_press(
        WindowEvent::ActivationChanged {
            active: false,
            timestamp: Duration::from_millis(3),
        },
        true,
    )
}
#[test]
fn input_interruption_cancels_ready_button_visuals_without_starting() -> TestResult<()> {
    canceled_press(
        WindowEvent::PointerInterrupted {
            timestamp: Duration::from_millis(3),
        },
        false,
    )
}
