use crate::{Error, Point};
use std::sync::Arc;

/// One element of a Bézier path in local coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    /// Starts a new contour at this point.
    MoveTo(Point),
    /// Appends a line to its endpoint.
    LineTo(Point),
    /// Appends a quadratic segment with one control point and an endpoint.
    QuadTo(Point, Point),
    /// Appends a cubic segment with two control points and an endpoint.
    CubicTo(Point, Point, Point),
    /// Closes the current contour with a line back to its starting point.
    Close,
}

/// Immutable path geometry. Clones share the validated command storage.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierPath(Arc<[PathCommand]>);

impl BezierPath {
    /// Validates finite coordinates and requires each contour to begin with a move.
    /// Empty paths are allowed. Fill operations implicitly close open contours.
    pub fn new(commands: Vec<PathCommand>) -> Result<Self, Error> {
        let mut open = false;
        let finite = |point: Point| point.x().is_finite() && point.y().is_finite();
        for &command in &commands {
            let valid = match command {
                PathCommand::MoveTo(point) => {
                    open = true;
                    finite(point)
                }
                PathCommand::LineTo(end) => open && finite(end),
                PathCommand::QuadTo(control, end) => open && finite(control) && finite(end),
                PathCommand::CubicTo(first, second, end) => {
                    open && finite(first) && finite(second) && finite(end)
                }
                PathCommand::Close => std::mem::take(&mut open),
            };
            if !valid {
                return Err(Error::InvalidPath);
            }
        }
        Ok(Self(commands.into()))
    }

    /// Returns the path elements in contour order.
    pub fn commands(&self) -> &[PathCommand] {
        &self.0
    }
}
