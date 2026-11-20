//! Game buttons share Action delivery with the board's Keymap.

use crate::{Ui, shortcuts::action_id};
use oxiview_core::{Action as UiAction, FocusPolicy, View};
use oxiview_macros::oxi_view;
use oxiview_style::{Alignment, ContentAlignment, LayoutPatch, StylePatch};
use snake::{
    application::Action,
    game::Phase,
    presentation::{INK, centered_label, color, primary_caption, px},
};
use std::rc::Rc;

pub(crate) struct ButtonProps {
    pub(crate) ui: Ui,
    pub(crate) action: Action,
    pub(crate) caption: &'static str,
    pub(crate) primary: bool,
    pub(crate) compact: bool,
    pub(crate) overlay: bool,
}

pub(crate) fn button_patch(width: f64, height: f64) -> StylePatch {
    StylePatch {
        layout: LayoutPatch {
            min_width: Some(px(width)),
            height: Some(px(height)),
            flex_shrink: Some(0.0),
            border_width: Some(1.0),
            justify_content: Some(ContentAlignment::Center),
            align_items: Some(Alignment::Center),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(crate) fn action_button(props: ButtonProps) -> View {
    let ButtonProps {
        ui,
        action,
        caption,
        primary,
        compact,
        overlay,
    } = props;
    let phase = ui.phase;
    let directional = matches!(action, Action::Turn(_));
    let mut face = button_patch(
        if directional {
            40.0
        } else if compact {
            60.0
        } else {
            96.0
        },
        if primary {
            46.0
        } else if compact {
            32.0
        } else {
            38.0
        },
    );
    if compact && !directional {
        face.layout.flex_grow = Some(1.0);
    }
    if overlay {
        face.layout.width = Some(oxiview_style::Length::Percent(1.0));
    }
    if action == Action::NewGame {
        face.background = Some(color([154, 55, 51]));
    }
    let kind = if primary { "primary" } else { "button" };
    let mut title = centered_label(
        &ui,
        caption,
        if directional { 19.0 } else { 14.0 },
        650,
        INK,
    )
    .theme_type(if action == Action::NewGame {
        "restart_label"
    } else if primary {
        "primary_label"
    } else {
        "button_label"
    });
    match action {
        Action::Primary => {
            title = title.bind_text(move || Ok(primary_caption(phase.get()?).into()))
        }
        Action::Difficulty(choice) => {
            let selected = ui.difficulty;
            title = title.bind_text(move || {
                Ok(if selected.get()? == choice {
                    format!("✓ {caption}")
                } else {
                    caption.into()
                })
            });
        }
        _ => {}
    }
    let slot = if overlay {
        Rc::clone(&ui.slots.overlay_primary)
    } else {
        ui.slots.button(action)
    };
    let id = action_id(action);
    let button = oxi_view!("src/button_face.oxi");
    button
        .on_action(UiAction::new(id, caption), move |_, _, commands| {
            ui.queue_game(action, commands);
        })
        .activate_with(id)
        .bind_disabled(move || {
            Ok(match action {
                Action::Difficulty(_) => matches!(phase.get()?, Phase::Running | Phase::Paused),
                Action::Turn(_) => phase.get()? != Phase::Running,
                _ => false,
            })
        })
}
