//! Node-local board drawing; no window, text, or model mutation lives here.

use crate::game::{Cell, Direction, Snapshot};
use oxiview_base::{Color, CornerRadii, Point, Rect, Size};
use oxiview_scene::{Brush, Primitive};

fn fill(rect: Rect, radius: f64, color: Color) -> Primitive {
    Primitive::Fill(rect, CornerRadii::uniform(radius), Brush::Solid(color))
}
fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// Draws the board at its actual retained node size, centered on whole logical cells.
pub(crate) fn draw(snapshot: &Snapshot, size: Size) -> Vec<Primitive> {
    let columns = f64::from(snapshot.board.columns());
    let rows = f64::from(snapshot.board.rows());
    let cell = ((size.width() - 32.0) / columns)
        .min((size.height() - 32.0) / rows)
        .floor();
    if !cell.is_finite() || cell < 2.0 {
        return Vec::new();
    }
    let width = columns * cell;
    let height = rows * cell;
    let left = ((size.width() - width) / 2.0).floor();
    let top = ((size.height() - height) / 2.0).floor();
    let center = |at: Cell| {
        Point::new(
            left + (f64::from(at.x) + 0.5) * cell,
            top + (f64::from(at.y) + 0.5) * cell,
        )
    };
    let mut primitives = vec![fill(
        Rect::new(left, top, left + width, top + height),
        3.0,
        rgb(18, 35, 30),
    )];
    let line = Color::from_rgba8(113, 155, 133, 24);
    for column in 1..snapshot.board.columns() {
        let x = left + f64::from(column) * cell;
        primitives.push(fill(Rect::new(x, top, x + 0.5, top + height), 0.0, line));
    }
    for row in 1..snapshot.board.rows() {
        let y = top + f64::from(row) * cell;
        primitives.push(fill(Rect::new(left, y, left + width, y + 0.5), 0.0, line));
    }
    if let Some(food) = snapshot.food {
        let at = center(food);
        let radius = cell * 0.29;
        primitives.push(fill(
            Rect::new(
                at.x() - radius,
                at.y() - radius,
                at.x() + radius,
                at.y() + radius,
            ),
            radius,
            rgb(249, 125, 104),
        ));
        let glint = cell * 0.075;
        primitives.push(fill(
            Rect::new(
                at.x() - glint * 1.8,
                at.y() - glint * 1.9,
                at.x() - glint * 0.2,
                at.y() - glint * 0.3,
            ),
            glint,
            rgb(255, 209, 173),
        ));
    }
    let body_color = |index: usize| {
        let proportion = (index as f64 / snapshot.body.len().max(1) as f64).clamp(0.0, 1.0);
        rgb(
            (162.0 - 96.0 * proportion) as u8,
            (225.0 - 75.0 * proportion) as u8,
            (113.0 + 11.0 * proportion) as u8,
        )
    };
    let half = cell * 0.38;
    for (index, pair) in snapshot.body.windows(2).enumerate() {
        if let [a, b] = pair {
            let a = center(*a);
            let b = center(*b);
            let rect = if (a.x() - b.x()).abs() > 0.1 {
                Rect::new(
                    a.x().min(b.x()),
                    a.y() - half,
                    a.x().max(b.x()),
                    a.y() + half,
                )
            } else {
                Rect::new(
                    a.x() - half,
                    a.y().min(b.y()),
                    a.x() + half,
                    a.y().max(b.y()),
                )
            };
            primitives.push(fill(rect, 0.0, body_color(index + 1)));
        }
    }
    for (index, segment) in snapshot.body.iter().copied().enumerate().rev() {
        let at = center(segment);
        let tone = if index == 0 {
            rgb(189, 239, 119)
        } else {
            body_color(index)
        };
        primitives.push(fill(
            Rect::new(at.x() - half, at.y() - half, at.x() + half, at.y() + half),
            cell * 0.2,
            tone,
        ));
    }
    if let Some(head) = snapshot.body.first() {
        let at = center(*head);
        for side in [-1.0, 1.0] {
            let (x, y) = match snapshot.direction {
                Direction::Right => (at.x() + cell * 0.16, at.y() + side * cell * 0.18),
                Direction::Left => (at.x() - cell * 0.16, at.y() + side * cell * 0.18),
                Direction::Up => (at.x() + side * cell * 0.18, at.y() - cell * 0.16),
                Direction::Down => (at.x() + side * cell * 0.18, at.y() + cell * 0.16),
            };
            let radius = (cell * 0.065).max(1.0);
            primitives.push(fill(
                Rect::new(x - radius, y - radius, x + radius, y + radius),
                radius,
                rgb(23, 49, 29),
            ));
        }
    }
    primitives
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Board, Difficulty, Game, SeededRng};

    #[test]
    fn tiny_canvases_are_empty_and_valid_canvases_draw_inside_their_bounds()
    -> Result<(), Box<dyn std::error::Error>> {
        let game = Game::new(Board::default(), Difficulty::Classic, SeededRng::new(42))?;
        assert!(draw(&game.snapshot(), Size::new(0.0, 0.0)).is_empty());
        let size = Size::new(680.0, 540.0);
        let primitives = draw(&game.snapshot(), size);
        assert!(!primitives.is_empty());
        for primitive in primitives {
            if let Primitive::Fill(rect, _, _) = primitive {
                assert!(rect.origin().x() >= 0.0 && rect.origin().y() >= 0.0);
                assert!(rect.origin().x() + rect.size().width() <= size.width());
                assert!(rect.origin().y() + rect.size().height() <= size.height());
            }
        }
        Ok(())
    }
}
