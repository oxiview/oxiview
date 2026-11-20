use super::{Command, Settings};
use crate::{
    Ui,
    controls::button_patch,
    shortcuts::{CLOSE, OPEN, named},
};
use oxiview_core::{
    Action, FocusPolicy, KeyBinding, Keymap, NodeId, UiError, View, component, node,
};
use oxiview_macros::oxi_view;
use oxiview_platform::KeyCode;
use oxiview_style::{Alignment, Edges, LayoutPatch, Length, StylePatch};
use snake::presentation::{INK, MUTED, blank, centered_label, column, label, px, row};
use std::{cell::Cell, rc::Rc};

fn button(
    ui: &Ui,
    id: &'static str,
    description: &'static str,
    command: Command,
    face: View,
    slot: fn(&super::Controls) -> &Cell<Option<NodeId>>,
) -> View {
    let sender = ui.settings.send.clone();
    let controls = Rc::clone(&ui.settings.controls);
    node()
        .theme_type("button")
        .style_patch(button_patch(100.0, 34.0))
        .focusable(FocusPolicy::Tab)
        .on_action(Action::new(id, description), move |_, _, _| {
            let _ = sender.send(command);
        })
        .activate_with(id)
        .on_mount(move |node| slot(&controls).set(Some(node)))
        .child(face)
}

fn label_button(ui: &Ui, title: &str) -> View {
    centered_label(ui, title, 13.0, 650, INK).theme_type("button_label")
}

pub(crate) fn opener(ui: &Ui) -> View {
    button(
        ui,
        OPEN,
        "Open settings",
        Command::Open,
        label_button(ui, "Settings"),
        |s| &s.opener,
    )
}

fn directional(
    view: View,
    settings: &Settings,
    left: &'static str,
    right: &'static str,
    command: fn(bool) -> Command,
) -> Result<View, UiError> {
    let sender = settings.send.clone();
    let other = sender.clone();
    let keymap = Keymap::new([
        KeyBinding::new(left, [named(KeyCode::ArrowLeft)]),
        KeyBinding::new(right, [named(KeyCode::ArrowRight)]),
    ])
    .map_err(|e| UiError::View(e.to_string()))?;
    Ok(view
        .keymap(keymap)
        .on_action(Action::new(left, "Previous option"), move |_, _, _| {
            let _ = sender.send(command(false));
        })
        .on_action(Action::new(right, "Next option"), move |_, _, _| {
            let _ = other.send(command(true));
        }))
}

fn field(ui: &Ui, caption: &str, control: View) -> View {
    node()
        .style(blank(oxiview_style::LayoutStyle {
            row_gap: px(4.0),
            ..column()
        }))
        .child(label(ui, caption, 11.0, 650, MUTED).theme_type("muted"))
        .child(control)
}

pub(crate) fn panel(ui: Ui) -> View {
    component(move || panel_content(ui))
}

fn panel_content(ui: Ui) -> Result<View, UiError> {
    let settings = &ui.settings;
    let palette = settings.palette_label;
    let appearance = button(
        &ui,
        "snake.appearance.cycle",
        "Next appearance",
        Command::Appearance(true),
        label_button(&ui, "System").bind_text(move || palette.get()),
        |s| &s.appearance,
    );
    let appearance = field(
        &ui,
        "Appearance",
        directional(
            appearance,
            settings,
            "snake.appearance.previous",
            "snake.appearance.next",
            Command::Appearance,
        )?,
    );
    let action_label = settings.action_label;
    let selection = button(
        &ui,
        "snake.binding.cycle",
        "Select Action",
        Command::SelectAction(true),
        label_button(&ui, "Turn up").bind_text(move || action_label.get()),
        |s| &s.selection,
    );
    let selection = field(
        &ui,
        "Game action",
        directional(
            selection,
            settings,
            "snake.binding.previous",
            "snake.binding.next",
            Command::SelectAction,
        )?,
    );
    let current = settings.shortcut_label;
    let recording = settings.recording;
    let record = button(
        &ui,
        "snake.binding.record",
        "Record shortcut",
        Command::Record,
        label_button(&ui, "Rebind").bind_text(move || {
            Ok(if recording.get()? {
                "Press a key…".into()
            } else {
                current.get()?
            })
        }),
        |s| &s.record,
    );
    let record = field(&ui, "Shortcut · select to rebind", record);
    let close = button(
        &ui,
        CLOSE,
        "Close settings",
        Command::Close,
        label_button(&ui, "Done"),
        |s| &s.close,
    );
    let status = settings.status;
    let theme_error = settings.theme_error;
    let status_view = label(&ui, "", 12.0, 450, MUTED)
        .theme_type("muted")
        .bind_text(move || match theme_error.get()? {
            Some(message) => Ok(message),
            None => status.get(),
        });
    let group = blank(oxiview_style::LayoutStyle {
        align_items: Alignment::End,
        column_gap: px(12.0),
        ..row()
    });
    let patch = StylePatch {
        layout: LayoutPatch {
            flex_direction: Some(oxiview_style::FlexDirection::Column),
            min_width: Some(px(0.0)),
            width: Some(Length::Percent(1.0)),
            padding: Some(Edges::all(px(12.0))),
            row_gap: Some(px(5.0)),
            ..Default::default()
        },
        ..Default::default()
    };
    let builder = settings.builder_panel;
    let panel = if builder {
        node()
            .theme_type("settings")
            .style_patch(patch)
            .child(
                node()
                    .style(group)
                    .child(appearance)
                    .child(selection)
                    .child(record)
                    .child(close),
            )
            .child(status_view)
    } else {
        oxi_view!("src/settings/panel.oxi")
    };
    let controls = Rc::clone(&settings.controls);
    let sender = settings.send.clone();
    let retry = sender.clone();
    let keymap = Keymap::new([
        KeyBinding::new(CLOSE, [named(KeyCode::Escape)]),
        KeyBinding::new("snake.settings.retry", [named(KeyCode::F5)]),
    ])
    .map_err(|e| UiError::View(e.to_string()))?;
    Ok(panel
        .on_mount(move |id| controls.panel.set(Some(id)))
        .keymap(keymap)
        .on_action(Action::new(CLOSE, "Close settings"), move |_, _, _| {
            let _ = sender.send(Command::Close);
        })
        .on_action(
            Action::new("snake.settings.retry", "Retry saving"),
            move |_, _, _| {
                let _ = retry.send(Command::Retry);
            },
        ))
}
