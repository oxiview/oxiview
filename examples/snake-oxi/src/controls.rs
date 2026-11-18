//! Button state and event wiring supplied to the view fragment.

use crate::styles::pressed_face;
use oxiview_core::{UiError, View, component};
use oxiview_macros::oxi_view;
use oxiview_reactive::{Signal, signal};
use snake::{
    application::Action,
    presentation::{INK, Ui, button_style, label, primary_caption},
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
pub(crate) fn action_button(props: ButtonProps) -> View {
    component(move || {
        let pressed = signal(false).map_err(|error| UiError::Reactive(error.to_string()))?;
        Ok(button_content(props, pressed))
    })
}
fn button_content(props: ButtonProps, pressed: Signal<bool>) -> View {
    let ButtonProps {
        ui,
        action,
        caption,
        primary,
        compact,
        overlay,
    } = props;
    let phase = ui.phase;
    let difficulty = ui.difficulty;
    let title = if action == Action::Primary {
        label(&ui, "Start game", 15.0, 700, INK)
            .bind_text(move || Ok(primary_caption(phase.get()?).into()))
    } else {
        label(
            &ui,
            caption,
            if matches!(action, Action::Turn(_)) {
                22.0
            } else if compact {
                13.0
            } else {
                14.0
            },
            650,
            INK,
        )
    };
    let slot = if overlay {
        Rc::clone(&ui.slots.overlay_primary)
    } else {
        ui.slots.button(action)
    };
    let pointer = ui.button_pointer(action, pressed);
    let appearance = move || {
        Ok(pressed_face(
            button_style(action, phase.get()?, difficulty.get()?, primary, compact),
            pressed.get()?,
        ))
    };
    oxi_view!("src/button_face.oxi")
}
