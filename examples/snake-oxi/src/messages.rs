//! Player-facing messages for each game phase.

use snake::game::{Collision, Phase, Snapshot};

pub(crate) fn overlay_title(phase: Phase) -> &'static str {
    match phase {
        Phase::Ready => "Ready when you are.",
        Phase::Paused => "Take a breath.",
        Phase::Over(_) => "One more round?",
        Phase::Won => "You cleared the board!",
        Phase::Running => "",
    }
}

pub(crate) fn overlay_message(game: &Snapshot) -> String {
    match game.phase {
        Phase::Ready => "Collect the coral food.\nLeave yourself room to turn.".into(),
        Phase::Paused => "Your next move can wait.\nPress Space or Resume to continue.".into(),
        Phase::Over(Collision::Wall) => format!(
            "The edge wins this round.\nYou collected {} points.",
            game.score
        ),
        Phase::Over(Collision::Body) => format!(
            "You crossed your own path.\nYou collected {} points.",
            game.score
        ),
        Phase::Won => format!(
            "Every cell is yours.\n{} points, beautifully played.",
            game.score
        ),
        Phase::Running => String::new(),
    }
}
