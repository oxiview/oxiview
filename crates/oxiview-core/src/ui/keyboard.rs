use super::{
    ActionSource, KeyChord, Keymap, NodeId, UiError, UiTree, View,
    interaction::{ArmedActivation, ArmedInput},
};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, WindowEvent};
use std::time::Duration;

/// Maximum time between strokes of a shortcut, measured on the backend clock.
pub const KEY_SEQUENCE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(PartialEq, Eq)]
pub(super) enum HeldKey {
    Physical(KeyCode),
    Logical(LogicalKey),
}

impl HeldKey {
    fn from_input(input: &KeyInput<'_>) -> Self {
        if input.code == KeyCode::Unidentified {
            Self::Logical(input.logical.clone())
        } else {
            Self::Physical(input.code)
        }
    }
}

struct KeyInput<'a> {
    code: KeyCode,
    logical: &'a LogicalKey,
    modifiers: KeyModifiers,
    repeat: bool,
    timestamp: Duration,
}

struct PendingSequence {
    context: NodeId,
    prefix: Vec<KeyChord>,
    deadline: Duration,
}

enum SequenceStep {
    Prefix(KeyChord),
    Action { id: String, repeat: bool },
}

#[derive(Default)]
pub(super) struct KeyboardState {
    sequence: Option<PendingSequence>,
    suppressed: Vec<HeldKey>,
}

impl KeyboardState {
    pub(super) fn clear_sequence(&mut self) {
        self.sequence = None;
    }

    pub(super) fn pending_context(&self) -> Option<NodeId> {
        self.sequence.as_ref().map(|pending| pending.context)
    }

    fn suppress(&mut self, key: HeldKey) {
        if !self.suppressed.contains(&key) {
            self.suppressed.push(key);
        }
    }

    fn release_suppression(&mut self, key: &HeldKey) -> bool {
        if let Some(index) = self.suppressed.iter().position(|held| held == key) {
            self.suppressed.swap_remove(index);
            true
        } else {
            false
        }
    }
}

impl View {
    /// Installs this node as a focus context with the supplied Keymap.
    pub fn keymap(mut self, keymap: Keymap) -> Self {
        self.setup
            .push(Box::new(move |tree, id, _| tree.set_keymap(id, keymap)));
        self
    }
}

impl UiTree {
    /// Installs a validated Keymap. A Keymap on the root supplies window defaults.
    pub fn set_keymap(&mut self, id: NodeId, keymap: Keymap) -> Result<(), UiError> {
        let slot = &mut self.node_mut(id)?.interaction_mut().keymap;
        if slot.as_deref() != Some(&keymap) {
            *slot = Some(Box::new(keymap));
            self.interaction.keyboard.clear_sequence();
        }
        Ok(())
    }

    /// Reads the Keymap attached to a node, if present.
    pub fn keymap(&self, id: NodeId) -> Result<Option<&Keymap>, UiError> {
        Ok(self
            .node(id)?
            .interaction
            .as_ref()
            .and_then(|interaction| interaction.keymap.as_deref()))
    }

    pub(super) fn route_key(
        &mut self,
        event: &WindowEvent,
        consumed: bool,
    ) -> Result<bool, UiError> {
        let WindowEvent::Key {
            code,
            logical,
            modifiers,
            pressed,
            repeat,
            timestamp,
        } = event
        else {
            return Ok(false);
        };
        let input = KeyInput {
            code: *code,
            logical,
            modifiers: *modifiers,
            repeat: *repeat,
            timestamp: *timestamp,
        };
        let held = HeldKey::from_input(&input);
        if consumed {
            self.interaction.keyboard.clear_sequence();
            self.take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))?;
            if *pressed {
                self.interaction.keyboard.suppress(held);
            } else {
                self.interaction.keyboard.release_suppression(&held);
            }
            return Ok(true);
        }
        if !self.accepts_input() {
            return Ok(!pressed && self.interaction.keyboard.release_suppression(&held));
        }
        if !pressed {
            if self.interaction.keyboard.release_suppression(&held) {
                return Ok(true);
            }
            if let Some(arm) =
                self.take_arm(|source| matches!(source, ArmedInput::Keyboard(key) if *key == held))?
            {
                self.perform_action(
                    arm.target,
                    &arm.action,
                    ActionSource::Keyboard,
                    Some(input.timestamp),
                )?;
                return Ok(true);
            }
            return Ok(false);
        }
        if *repeat && self.interaction.keyboard.suppressed.contains(&held) {
            return Ok(true);
        }
        self.interaction.keyboard.release_suppression(&held);
        let KeyModifiers {
            shift,
            alt,
            control,
            meta,
        } = input.modifiers;
        if matches!(input.logical, LogicalKey::Named(KeyCode::Tab)) && !alt && !control && !meta {
            self.interaction.keyboard.clear_sequence();
            self.tab_focus(shift)?;
            return Ok(true);
        }
        self.transition_focus(self.focused_node(), Some(true))?;
        if self.route_shortcut(&input, held)? {
            return Ok(true);
        }
        self.activate_from_key(&input)
    }

    fn route_shortcut(&mut self, input: &KeyInput<'_>, held: HeldKey) -> Result<bool, UiError> {
        let pending = self
            .interaction
            .keyboard
            .sequence
            .take()
            .filter(|pending| input.timestamp < pending.deadline);
        if let Some(pending) = pending {
            if input.repeat {
                self.interaction.keyboard.sequence = Some(pending);
                return Ok(true);
            }
            if let Some(step) = self.sequence_step(pending.context, &pending.prefix, input)? {
                self.finish_step(pending.context, pending.prefix, step, input, held)?;
                return Ok(true);
            }
        }
        let start = self.focused_node().unwrap_or(self.root);
        if !self.interaction_available(start)? {
            return Ok(false);
        }
        let mut context = Some(start);
        while let Some(id) = context {
            if let Some(step) = self.sequence_step(id, &[], input)? {
                self.finish_step(id, Vec::new(), step, input, held)?;
                return Ok(true);
            }
            context = self.node(id)?.parent;
        }
        Ok(false)
    }

    fn sequence_step(
        &self,
        context: NodeId,
        prefix: &[KeyChord],
        input: &KeyInput<'_>,
    ) -> Result<Option<SequenceStep>, UiError> {
        Ok(self
            .keymap(context)?
            .and_then(|map| {
                map.matching_binding(prefix, input.code, input.logical, input.modifiers)
            })
            .map(|(binding, chord)| {
                if binding.sequence.len() == prefix.len() + 1 {
                    SequenceStep::Action {
                        id: binding.action.clone(),
                        repeat: binding.repeat,
                    }
                } else {
                    SequenceStep::Prefix(chord.clone())
                }
            }))
    }

    fn finish_step(
        &mut self,
        context: NodeId,
        mut prefix: Vec<KeyChord>,
        step: SequenceStep,
        input: &KeyInput<'_>,
        held: HeldKey,
    ) -> Result<(), UiError> {
        match step {
            SequenceStep::Prefix(chord) => {
                if !input.repeat {
                    self.take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))?;
                    self.interaction.keyboard.suppress(held);
                    prefix.push(chord);
                    self.interaction.keyboard.sequence = Some(PendingSequence {
                        context,
                        prefix,
                        deadline: input.timestamp.saturating_add(KEY_SEQUENCE_TIMEOUT),
                    });
                }
            }
            SequenceStep::Action { id, repeat } if !input.repeat || repeat => {
                self.take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))?;
                if !prefix.is_empty() {
                    self.interaction.keyboard.suppress(held);
                }
                self.perform_focused_action(&id, input.timestamp)?;
            }
            SequenceStep::Action { .. } => {}
        }
        Ok(())
    }

    fn activate_from_key(&mut self, input: &KeyInput<'_>) -> Result<bool, UiError> {
        if matches!(input.logical, LogicalKey::Named(KeyCode::Escape)) && !input.repeat {
            return self
                .take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))
                .map(|press| press.is_some());
        }
        if input.modifiers != KeyModifiers::default() {
            return Ok(false);
        }
        let Some(target) = self.focused_node() else {
            return Ok(false);
        };
        let Some(action) = self.activation_action(target)?.map(str::to_owned) else {
            return Ok(false);
        };
        match input.logical {
            LogicalKey::Named(KeyCode::Space) => {
                if !input.repeat {
                    self.set_armed(Some(ArmedActivation {
                        target,
                        action,
                        input: ArmedInput::Keyboard(HeldKey::from_input(input)),
                    }))?;
                }
                Ok(true)
            }
            LogicalKey::Named(KeyCode::Enter) => {
                if !input.repeat {
                    self.take_arm(|source| matches!(source, ArmedInput::Keyboard(_)))?;
                    self.perform_action(
                        target,
                        &action,
                        ActionSource::Keyboard,
                        Some(input.timestamp),
                    )?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
