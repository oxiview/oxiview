use oxiview_base::{Affine, Color, CornerRadii, Point, Rect};
use oxiview_scene::{Brush, Clip, ColorStop, NodeId, NodeKind, Primitive, Scene};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub fn fill(rect: Rect, color: Color) -> NodeKind {
    NodeKind::Draw(Primitive::Fill(
        rect,
        CornerRadii::uniform(0.0),
        Brush::Solid(color),
    ))
}
pub fn red() -> Color {
    Color::from_rgba8(255, 0, 0, 255)
}
pub fn blue() -> Color {
    Color::from_rgba8(0, 0, 255, 255)
}

pub fn scene(name: &str) -> TestResult<(Scene, NodeId)> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, NodeKind::Transform(Affine::IDENTITY))?;
    match name {
        "rounded-rect" => {
            scene.insert(
                Some(root),
                0,
                NodeKind::Draw(Primitive::Fill(
                    Rect::new(8.0, 8.0, 56.0, 56.0),
                    CornerRadii::uniform(10.0),
                    Brush::Solid(Color::from_rgba8(24, 160, 112, 255)),
                )),
            )?;
        }
        "linear-gradient" => {
            scene.insert(
                Some(root),
                0,
                NodeKind::Draw(Primitive::Fill(
                    Rect::new(8.0, 8.0, 56.0, 56.0),
                    CornerRadii::default(),
                    Brush::LinearGradient(
                        Point::new(8.0, 8.0),
                        Point::new(56.0, 8.0),
                        vec![
                            ColorStop {
                                position: 0.0,
                                color: red(),
                            },
                            ColorStop {
                                position: 1.0,
                                color: blue(),
                            },
                        ],
                    ),
                )),
            )?;
        }
        "clip" => {
            let clip = scene.insert(
                Some(root),
                0,
                NodeKind::Clip(Clip::Rect(Rect::new(20.0, 20.0, 44.0, 44.0))),
            )?;
            scene.insert(Some(clip), 0, fill(Rect::new(4.0, 4.0, 60.0, 60.0), red()))?;
        }
        "opacity" | "overlap" => {
            let parent = if name == "opacity" {
                scene.insert(Some(root), 0, NodeKind::Opacity(0.5))?
            } else {
                root
            };
            scene.insert(
                Some(parent),
                0,
                fill(Rect::new(8.0, 8.0, 40.0, 40.0), red()),
            )?;
            scene.insert(
                Some(parent),
                1,
                fill(Rect::new(24.0, 24.0, 56.0, 56.0), blue()),
            )?;
        }
        "reused-subtree" => {
            let subtree = scene.insert(None, 1, fill(Rect::new(0.0, 0.0, 16.0, 24.0), red()))?;
            for (index, x) in [8.0, 40.0].into_iter().enumerate() {
                scene.insert(
                    Some(root),
                    index,
                    NodeKind::Reference(subtree, Affine::new([1.0, 0.0, 0.0, 1.0, x, 20.0])),
                )?;
            }
        }
        _ => return Err(format!("unknown test scene: {name}").into()),
    }
    Ok((scene, root))
}
