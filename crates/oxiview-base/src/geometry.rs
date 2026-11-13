use crate::Error;

/// A two-dimensional point. Coordinates are preserved, including non-finite values.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point(pub(crate) kurbo::Point);
impl Point {
    /// Creates a point in the caller's coordinate space.
    pub const fn new(x: f64, y: f64) -> Self {
        Self(kurbo::Point::new(x, y))
    }
    /// Returns the horizontal coordinate.
    pub const fn x(self) -> f64 {
        self.0.x
    }
    /// Returns the vertical coordinate.
    pub const fn y(self) -> f64 {
        self.0.y
    }
}

/// Width and height; signed and non-finite values are preserved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size(pub(crate) kurbo::Size);
impl Size {
    /// Creates a size without normalization.
    pub const fn new(width: f64, height: f64) -> Self {
        Self(kurbo::Size::new(width, height))
    }
    /// Returns the width.
    pub const fn width(self) -> f64 {
        self.0.width
    }
    /// Returns the height.
    pub const fn height(self) -> f64 {
        self.0.height
    }
}

/// Axis-aligned rectangle with directed edges; coordinates are not reordered.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect(pub(crate) kurbo::Rect);
impl Rect {
    /// Creates a rectangle from left, top, right, and bottom coordinates.
    pub const fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self(kurbo::Rect::new(x0, y0, x1, y1))
    }
    /// Creates a rectangle from its origin and signed size.
    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self(kurbo::Rect::from_origin_size(origin.0, size.0))
    }
    /// Returns the first corner.
    pub fn origin(self) -> Point {
        Point::new(self.0.x0, self.0.y0)
    }
    /// Returns the signed size.
    pub fn size(self) -> Size {
        Size(self.0.size())
    }
    /// Returns left, top, right, and bottom coordinates.
    pub fn edges(self) -> [f64; 4] {
        [self.0.x0, self.0.y0, self.0.x1, self.0.y1]
    }
}

/// Circular corner radii, clockwise from top left in a downward-positive y axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadii(pub(crate) kurbo::RoundedRectRadii);
impl CornerRadii {
    /// Creates radii without clamping; consumers choose their shape's bounds.
    pub const fn new(top_left: f64, top_right: f64, bottom_right: f64, bottom_left: f64) -> Self {
        Self(kurbo::RoundedRectRadii::new(
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        ))
    }
    /// Creates four equal radii.
    pub const fn uniform(radius: f64) -> Self {
        Self::new(radius, radius, radius, radius)
    }
    /// Returns radii clockwise from top left.
    pub const fn values(self) -> [f64; 4] {
        [
            self.0.top_left,
            self.0.top_right,
            self.0.bottom_right,
            self.0.bottom_left,
        ]
    }
}

/// A two-dimensional affine transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub(crate) kurbo::Affine);
impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Affine {
    /// The identity transform.
    pub const IDENTITY: Self = Self(kurbo::Affine::IDENTITY);
    /// Creates `[a, b, c, d, e, f]`: `(x, y)` maps to `(ax + cy + e, bx + dy + f)`.
    pub const fn new(coefficients: [f64; 6]) -> Self {
        Self(kurbo::Affine::new(coefficients))
    }
    /// Returns the six coefficients in constructor order.
    pub fn coefficients(self) -> [f64; 6] {
        self.0.as_coeffs()
    }
    /// Applies this transform to a point.
    pub fn transform_point(self, point: Point) -> Point {
        Point(self.0 * point.0)
    }
    /// Composes transforms, applying `self` first and `next` second.
    pub fn then(self, next: Self) -> Self {
        Self(next.0 * self.0)
    }
    /// Returns the inverse, rejecting singular and non-finite results.
    pub fn inverse(self) -> Result<Self, Error> {
        let determinant = self.0.determinant();
        if determinant == 0.0 || !determinant.is_finite() {
            return Err(Error::NonInvertibleTransform);
        }
        let inverse = self.0.inverse();
        if !inverse.as_coeffs().iter().all(|value| value.is_finite()) {
            return Err(Error::NonInvertibleTransform);
        }
        Ok(Self(inverse))
    }
}
