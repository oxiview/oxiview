//! Initial and live system preferences reach the first headless frame in order.

use oxiview_app::run_headless;
use oxiview_base::{Color, Size};
use oxiview_core::{
    Application, Error, FrameTiming, NodeId, PaintedScene, Scheduler, UiTree, node,
};
use oxiview_platform::{
    PhysicalSize, SystemAppearance, SystemPreferences, WindowAttributes, WindowEvent, WindowWaker,
};
use oxiview_platform_headless::HeadlessPlatform;
use oxiview_style::Theme;
use std::{
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

const VIEWPORT: Size = Size::new(36.0, 24.0);
const THEME: &str = r##"
version = 1
[tokens.colors]
surface = "#f1f0e4"
[dark.colors]
surface = "#263244"
[rules.panel]
background = "@surface"
"##;

type FirstFrame = (Option<SystemAppearance>, Option<SystemAppearance>, Color);

struct App {
    tree: UiTree,
    panel: NodeId,
    initial_appearance: Option<SystemAppearance>,
    first_frame: Option<Sender<FirstFrame>>,
}

impl Application for App {
    fn initialize_preferences(&mut self, preferences: SystemPreferences) -> Result<(), Error> {
        self.tree.set_system_preferences(preferences)?;
        self.initial_appearance = self.tree.preference_signals().appearance()?;
        Ok(())
    }

    fn event(
        &mut self,
        event: &WindowEvent,
        scheduler: &mut Scheduler,
        _: &WindowWaker,
    ) -> Result<(), Error> {
        if self.tree.dispatch(event, VIEWPORT, |_, _, _| {})?.changed {
            scheduler.invalidate();
        }
        Ok(())
    }

    fn paint(
        &mut self,
        _: FrameTiming,
        _: PhysicalSize,
        scale: f64,
    ) -> Result<PaintedScene, Error> {
        let painted = self.tree.paint(VIEWPORT, scale)?;
        if let Some(sender) = self.first_frame.take() {
            sender
                .send((
                    self.initial_appearance,
                    self.tree.preference_signals().appearance()?,
                    self.tree.style(self.panel)?.background,
                ))
                .map_err(|error| Error::Application(error.to_string()))?;
        }
        Ok(painted)
    }
}

#[test]
fn startup_preference_change_selects_the_first_presented_palette()
-> Result<(), Box<dyn std::error::Error>> {
    let initial = SystemPreferences {
        appearance: Some(SystemAppearance::Light),
        ..Default::default()
    };
    let live = SystemPreferences {
        appearance: Some(SystemAppearance::Dark),
        ..initial
    };
    let refresh = Duration::from_millis(10);
    let (platform, driver) = HeadlessPlatform::new_with_preferences(refresh, initial)?;
    let injector = driver.clone();
    let (first_frame, observed) = mpsc::channel();
    let (ready, window) = mpsc::channel();
    let theme = Theme::from_toml(THEME)?;
    thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let close = scope.spawn(
            move || -> Result<FirstFrame, Box<dyn std::error::Error + Send + Sync>> {
                let id = window.recv_timeout(Duration::from_secs(10))?;
                let frame = observed.recv_timeout(Duration::from_secs(10));
                let closed = driver.request_user_close(id);
                let frame = frame?;
                closed?;
                Ok(frame)
            },
        );
        let run = run_headless(
            platform,
            WindowAttributes {
                title: "Theme preference fixture".into(),
                logical_size: VIEWPORT,
                resize_timeout: Duration::from_millis(100),
            },
            move || {
                let mut tree = UiTree::new()?;
                tree.set_theme(theme)?;
                let panel = tree.mount(tree.root(), node().theme_type("panel"))?;
                Ok(App {
                    tree,
                    panel,
                    initial_appearance: None,
                    first_frame: Some(first_frame),
                })
            },
            move |_, id, _| {
                ready
                    .send(id)
                    .map_err(|error| oxiview_platform::Error::OperationFailed(error.to_string()))?;
                injector
                    .preferences_changed(id, live)
                    .map_err(|error| oxiview_platform::Error::OperationFailed(error.to_string()))
            },
        );
        run?;
        let frame = close
            .join()
            .map_err(|_| std::io::Error::other("headless theme driver unwound"))?
            .map_err(std::io::Error::other)?;
        assert_eq!(
            frame,
            (
                initial.appearance,
                live.appearance,
                Color::from_rgba8(38, 50, 68, 255)
            )
        );
        Ok(())
    })
}
