//! CPU specimens produced by the same retained application and input handlers as the desktop.

use crate::{
    app::ViewerApp,
    model::{Batch, Column, Record},
};
use oxiview_base::Point;
use oxiview_core::{Application, FrameKind, FrameTiming, NodeId, Scheduler, TextSystem};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, PhysicalSize, PointerButton, WindowEvent, WindowWake,
    WindowWaker,
};
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::PixelSize;
use oxiview_testing::{CompareConfig, SnapshotHarness};
use oxiview_widgets::collection::CellKey;
use std::{path::Path, sync::Arc, time::Duration};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
struct Wake;
impl WindowWake for Wake {
    fn wake(&self) -> std::result::Result<(), oxiview_platform::Error> {
        Ok(())
    }
}
struct Fixture {
    app: ViewerApp,
    scheduler: Scheduler,
    waker: WindowWaker,
    tick: u64,
    size: PhysicalSize,
    scale: f64,
}
impl Fixture {
    fn new(scale: f64) -> Result<Self> {
        let mut fonts = TextSystem::without_system_fonts();
        fonts.register_font(
            include_bytes!("../../../crates/oxiview-text/tests/fonts/NotoSans.ttf").to_vec(),
        )?;
        let mut view = Self {
            app: ViewerApp::with_text(None, fonts)?,
            scheduler: Scheduler::default(),
            waker: WindowWaker::new(Arc::new(Wake)),
            tick: 0,
            size: PhysicalSize {
                width: (1120. * scale) as u32,
                height: (616. * scale) as u32,
            },
            scale,
        };
        let _ = view.frame()?;
        Ok(view)
    }
    fn time(&self) -> Duration {
        Duration::from_millis(self.tick)
    }
    fn frame(&mut self) -> Result<oxiview_core::PaintedScene> {
        Ok(self.app.paint(
            FrameTiming {
                kind: FrameKind::Resize,
                predicted_presentation_timestamp: self.time(),
                refresh_interval: Duration::from_millis(16),
            },
            self.size,
            self.scale,
        )?)
    }
    fn event(&mut self, event: WindowEvent) -> Result<()> {
        self.tick += 1;
        self.app.event(&event, &mut self.scheduler, &self.waker)?;
        Ok(())
    }
    fn click(&mut self, id: NodeId) -> Result<()> {
        let [x, y, r, b] = self.app.tree.bounds(id)?.edges();
        let position = Point::new((x + r) * 0.5, (y + b) * 0.5);
        for pressed in [true, false] {
            self.event(WindowEvent::PointerButton {
                button: PointerButton::Primary,
                pressed,
                position,
                timestamp: self.time(),
            })?;
        }
        Ok(())
    }
    fn key(&mut self, code: KeyCode, shift: bool) -> Result<()> {
        for pressed in [true, false] {
            self.event(WindowEvent::Key {
                code,
                logical: LogicalKey::Named(code),
                modifiers: KeyModifiers {
                    shift,
                    ..Default::default()
                },
                pressed,
                repeat: false,
                timestamp: self.time(),
            })?;
        }
        Ok(())
    }
    fn image(&mut self) -> Result<oxiview_testing::RgbaImage> {
        let frame = self.frame()?;
        let out = CpuRenderer.render(
            &frame.scene,
            frame.root,
            PixelSize {
                width: self.size.width,
                height: self.size.height,
            },
        )?;
        Ok(oxiview_testing::RgbaImage::new(
            out.width(),
            out.height(),
            out.pixels().to_vec(),
        )?)
    }
}

#[test]
fn live_log_viewer_at_three_scales() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let snapshots = SnapshotHarness::new(
        &root.join("../..").canonicalize()?,
        root,
        env!("CARGO_PKG_NAME"),
    )?;
    let mut failures = Vec::new();
    for (state, scale, name) in [
        (0, 1., "m7-expanded-list-1x"),
        (1, 1.5, "m7-table-selection-1_5x"),
        (2, 2., "m7-paused-follow-2x"),
    ] {
        let mut f = Fixture::new(scale)?;
        match state {
            0 => {
                f.click(
                    f.app
                        .list
                        .row_node(6)
                        .ok_or("sample row 6 is not mounted")?,
                )?;
                f.key(KeyCode::Enter, false)?;
                assert_eq!(f.app.active_key(), Some(6));
                assert!(f.app.list.height(5).is_some_and(|h| h > 40.));
                f.app.scroll_sample(13.5)?;
            }
            1 => {
                f.click(f.app.ui.mode.node(1).ok_or("Table control missing")?)?;
                assert!(f.app.is_table());
                f.click(
                    f.app
                        .table
                        .cell_node(CellKey {
                            row: 3,
                            column: Column::Level as u64,
                        })
                        .ok_or("table level cell missing")?,
                )?;
                f.key(KeyCode::ArrowRight, true)?;
                f.key(KeyCode::ArrowDown, true)?;
                f.key(KeyCode::ArrowDown, true)?;
                let selection = f.app.table.selection().ok_or("no table selection")?;
                assert_ne!(selection.anchor, selection.active);
            }
            _ => {
                f.app.book.resume_follow();
                f.event(WindowEvent::Wake)?;
                f.app.scroll_sample(-187.5)?;
                let anchored = f.app.first_visible_key();
                assert!(!f.app.book.follow());
                let records = [
                    "Published batch 100001",
                    "Committed batch 100002",
                    "Committed batch 100003",
                ]
                .into_iter()
                .map(|s| Record::parse(format!("12:29:00 [INFO] storage: {s}")))
                .collect();
                f.app.book.apply(
                    Path::new("sample.log"),
                    Batch {
                        reset: false,
                        replace_partial: false,
                        records,
                    },
                    false,
                )?;
                f.event(WindowEvent::Wake)?;
                assert_eq!(f.app.book.unseen(), 3);
                assert_eq!(f.app.first_visible_key(), anchored);
            }
        }
        if let Err(e) = snapshots.assert_snapshot(name, &f.image()?, &CompareConfig::default()) {
            failures.push(e.to_string());
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}
