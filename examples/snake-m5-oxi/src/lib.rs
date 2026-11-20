//! Native Snake milestone demo with keyboard navigation and inline player settings.

mod app;
mod cli;
mod controls;
mod messages;
pub mod replay;
mod settings;
mod shortcuts;
#[cfg(test)]
mod window_tests;

mod board {
    oxiview_macros::oxi_module!("src/board.oxi");
}
mod screen {
    oxiview_macros::oxi_module!("src/screen.oxi");
}
mod styles {
    oxiview_macros::oxi_module!("src/styles.oxi");
}
mod sidebar {
    oxiview_macros::oxi_module!("src/sidebar.oxi");
}

pub use cli::run;

use oxiview_core::UiCommands;
use snake::{application::Action, game::Phase, presentation::Ui as GameUi};
use std::ops::Deref;

#[derive(Clone)]
struct Ui {
    game: GameUi,
    settings: settings::Settings,
}

impl Deref for Ui {
    type Target = GameUi;
    fn deref(&self) -> &Self::Target {
        &self.game
    }
}

impl Ui {
    fn queue_game(&self, action: Action, commands: &mut UiCommands) {
        let entering_board = match action {
            Action::Primary => self.phase.get().is_ok_and(|p| p != Phase::Running),
            Action::NewGame | Action::Turn(_) => true,
            _ => false,
        };
        self.game.request_action(action);
        if entering_board && let Some(board) = self.slots.board.get() {
            commands.focus(board);
        }
    }
}
