use crate::{Error, paint};
use oxiview_base::Point;
use oxiview_scene::{BezierPath, Brush, FillRule, PathCommand};
use vello_cpu::{RenderContext, kurbo, peniko};

pub(super) fn paint_path(
    context: &mut RenderContext,
    path: &BezierPath,
    rule: FillRule,
    brush: &Brush,
) -> Result<(), Error> {
    let transform = *context.transform();
    if transform.determinant() == 0.0 {
        return Ok(());
    }
    let point = |local: Point| {
        let local = kurbo::Point::new(local.x(), local.y());
        let device = transform * local;
        if !(device.x as f32).is_finite() || !(device.y as f32).is_finite() {
            return Err(Error::SceneEncoding(
                "non-finite or out-of-range CPU path geometry".into(),
            ));
        }
        Ok(local)
    };
    let mut geometry = kurbo::BezPath::new();
    for &command in path.commands() {
        match command {
            PathCommand::MoveTo(start) => geometry.move_to(point(start)?),
            PathCommand::LineTo(end) => geometry.line_to(point(end)?),
            PathCommand::QuadTo(control, end) => geometry.quad_to(point(control)?, point(end)?),
            PathCommand::CubicTo(first, second, end) => {
                geometry.curve_to(point(first)?, point(second)?, point(end)?);
            }
            PathCommand::Close => geometry.close_path(),
        }
    }
    paint(context, brush);
    context.set_fill_rule(match rule {
        FillRule::NonZero => peniko::Fill::NonZero,
        FillRule::EvenOdd => peniko::Fill::EvenOdd,
    });
    context.fill_path(&geometry);
    context.set_fill_rule(peniko::Fill::NonZero);
    Ok(())
}
