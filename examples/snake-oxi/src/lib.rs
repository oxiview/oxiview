//! Rust application modules and the precompiled Snake screen.

mod controls;
mod messages;

mod board {
    oxiview_macros::oxi_module!("src/board.oxi");
}
mod screen {
    oxiview_macros::oxi_module!("src/screen.oxi");
}
mod sidebar {
    oxiview_macros::oxi_module!("src/sidebar.oxi");
}
mod styles {
    oxiview_macros::oxi_module!("src/styles.oxi");
}

pub use screen::screen;
