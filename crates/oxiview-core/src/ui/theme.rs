use super::{
    NodeId, UiError, UiTree,
    input::{Handler, Work},
    reactive_error,
    text::TextAppearance,
};
use oxiview_base::Color;
use oxiview_platform::{SystemAppearance, SystemPreferences};
use oxiview_reactive::{ReactiveError, Signal, batch, signal};
use oxiview_style::{
    AppearanceMode, FocusRing, InteractionState, StateVariant, Style, StylePatch, Theme,
    ThemeEnvironment, ThemeStyle,
};
use std::{
    collections::{HashMap, HashSet, VecDeque, hash_map::Entry},
    rc::Rc,
};

/// Tracks the native preferences of one retained window. Each read observes its own field.
#[derive(Clone, Copy, Debug)]
pub struct PreferenceSignals {
    appearance: Signal<Option<SystemAppearance>>,
    accent_color: Signal<Option<Color>>,
    increased_contrast: Signal<Option<bool>>,
    reduced_motion: Signal<Option<bool>>,
    text_scale: Signal<Option<f64>>,
}

impl PreferenceSignals {
    /// Reads and tracks the system appearance, when available.
    pub fn appearance(self) -> Result<Option<SystemAppearance>, ReactiveError> {
        self.appearance.get()
    }

    /// Reads and tracks the system accent color, when available.
    pub fn accent_color(self) -> Result<Option<Color>, ReactiveError> {
        self.accent_color.get()
    }

    /// Reads and tracks the system contrast preference, when available.
    pub fn increased_contrast(self) -> Result<Option<bool>, ReactiveError> {
        self.increased_contrast.get()
    }

    /// Reads and tracks the system motion preference, when available.
    pub fn reduced_motion(self) -> Result<Option<bool>, ReactiveError> {
        self.reduced_motion.get()
    }

    /// Reads and tracks the system multiplier for logical font sizes, when available.
    pub fn text_scale(self) -> Result<Option<f64>, ReactiveError> {
        self.text_scale.get()
    }

    fn publish(self, preferences: SystemPreferences) -> Result<(), UiError> {
        batch(|| {
            self.appearance.set_if_changed(preferences.appearance)?;
            self.accent_color.set_if_changed(preferences.accent_color)?;
            self.increased_contrast
                .set_if_changed(preferences.increased_contrast)?;
            self.reduced_motion
                .set_if_changed(preferences.reduced_motion)?;
            self.text_scale.set_if_changed(preferences.text_scale)?;
            Ok(())
        })
        .map_err(reactive_error)
    }
}

/// Sparse declarations moved from a Builder into the mounted node.
#[derive(Default)]
pub(super) struct ViewStyles {
    pub(super) widget: Option<Rc<str>>,
    pub(super) base: Option<Box<StylePatch>>,
    pub(super) variants: [Option<Box<StylePatch>>; 4],
}

impl ViewStyles {
    pub(super) fn overlay(&mut self, other: Self) {
        if other.widget.is_some() {
            self.widget = other.widget;
        }
        merge_patch(&mut self.base, other.base);
        for (current, later) in self.variants.iter_mut().zip(other.variants) {
            merge_patch(current, later);
        }
    }

    fn active(&self, state: InteractionState) -> impl Iterator<Item = &StylePatch> {
        self.base
            .as_deref()
            .into_iter()
            .chain(
                StateVariant::PRIORITY
                    .into_iter()
                    .filter_map(move |variant| {
                        if variant.applies(state) {
                            self.variants[variant as usize].as_deref()
                        } else {
                            None
                        }
                    }),
            )
    }
}

pub(super) fn merge_patch(current: &mut Option<Box<StylePatch>>, later: Option<Box<StylePatch>>) {
    if let Some(later) = later {
        if let Some(current) = current {
            current.overlay(*later);
        } else {
            *current = Some(later);
        }
    }
}

pub(super) struct NodeStyles {
    local: ViewStyles,
    complete: Option<Box<Style>>,
    state: InteractionState,
    tracks_state: bool,
    pending: bool,
    consumer: Option<Consumer>,
    pub(super) focus_ring: Option<FocusRing>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Consumer {
    widget: Rc<str>,
    state: InteractionState,
    usage: ThemeUsage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ThemeUsage {
    background: bool,
    border_color: bool,
    border_radii: bool,
    padding: bool,
    opacity: bool,
    focus_ring: bool,
    text_size: bool,
    text_color: bool,
}

impl Consumer {
    fn resolve(&self, theme: &Theme, environment: ThemeEnvironment) -> ThemeStyle {
        let source = theme.resolve(&self.widget, self.state, environment);
        ThemeStyle {
            background: source.background.filter(|_| self.usage.background),
            border_color: source.border_color.filter(|_| self.usage.border_color),
            border_radii: source.border_radii.filter(|_| self.usage.border_radii),
            padding: source.padding.filter(|_| self.usage.padding),
            opacity: source.opacity.filter(|_| self.usage.opacity),
            focus_ring: source.focus_ring.filter(|_| self.usage.focus_ring),
            text_size: source.text_size.filter(|_| self.usage.text_size),
            text_color: source.text_color.filter(|_| self.usage.text_color),
        }
    }
}

impl NodeStyles {
    fn consumer(&self, state: InteractionState, text: bool) -> Option<Consumer> {
        let widget = Rc::clone(self.local.widget.as_ref()?);
        let sparse = self.complete.is_none();
        let mut usage = ThemeUsage {
            background: sparse,
            border_color: sparse,
            border_radii: sparse,
            padding: sparse,
            opacity: sparse,
            focus_ring: state.focused,
            text_size: text,
            text_color: text,
        };
        for local in self.local.active(state) {
            usage.background &= local.background.is_none();
            usage.border_color &= local.border_color.is_none();
            usage.border_radii &= local.border_radii.is_none();
            usage.padding &= local.layout.padding.is_none();
            usage.opacity &= local.opacity.is_none();
            usage.focus_ring &= local.focus_ring.is_none();
            usage.text_size &= local.text_size.is_none();
            usage.text_color &= local.text_color.is_none();
        }
        Some(Consumer {
            widget,
            state,
            usage,
        })
    }

    fn resolve(&self, state: InteractionState, mut theme: ThemeStyle) -> (Style, ThemeStyle) {
        let mut style = self.complete.as_deref().cloned().unwrap_or_default();
        if self.complete.is_none() {
            theme.apply_to(&mut style);
        }
        for local in self.local.active(state) {
            local.apply_to(&mut style);
            theme.focus_ring = local.focus_ring.or(theme.focus_ring);
            theme.text_size = local.text_size.or(theme.text_size);
            theme.text_color = local.text_color.or(theme.text_color);
        }
        (style, theme)
    }
}

struct Consumers {
    values: ThemeStyle,
    nodes: HashSet<NodeId>,
}

pub(super) struct ThemeState {
    theme: Theme,
    mode: AppearanceMode,
    preferences: SystemPreferences,
    signals: PreferenceSignals,
    consumers: HashMap<Consumer, Consumers>,
    text_nodes: HashSet<NodeId>,
    pending: VecDeque<NodeId>,
}

impl ThemeState {
    pub(super) fn new() -> Result<Self, ReactiveError> {
        Ok(Self {
            theme: Theme::default(),
            mode: AppearanceMode::default(),
            preferences: SystemPreferences::default(),
            signals: PreferenceSignals {
                appearance: signal(None)?,
                accent_color: signal(None)?,
                increased_contrast: signal(None)?,
                reduced_motion: signal(None)?,
                text_scale: signal(None)?,
            },
            consumers: HashMap::new(),
            text_nodes: HashSet::new(),
            pending: VecDeque::new(),
        })
    }

    pub(super) fn has_pending_styles(&self) -> bool {
        !self.pending.is_empty()
    }

    fn values_for(&self, consumer: &Consumer) -> ThemeStyle {
        self.consumers.get(consumer).map_or_else(
            || consumer.resolve(&self.theme, environment(self.mode, self.preferences)),
            |group| group.values,
        )
    }

    fn remove_consumer(&mut self, id: NodeId, consumer: &Consumer) {
        if let Some(group) = self.consumers.get_mut(consumer) {
            group.nodes.remove(&id);
            if group.nodes.is_empty() {
                self.consumers.remove(consumer);
            }
        }
    }
}

struct StyleUpdate {
    id: NodeId,
    style: Option<Style>,
    focus_ring: Option<FocusRing>,
    text: Option<TextAppearance>,
    remeasure: bool,
    repaint_text: bool,
}

impl UiTree {
    /// Returns the independent, window-owned system preference signals.
    pub fn preference_signals(&self) -> PreferenceSignals {
        self.theme.signals
    }

    /// Returns the window's chosen appearance mode.
    pub fn appearance_mode(&self) -> AppearanceMode {
        self.theme.mode
    }

    /// Selects the palette for the window and returns whether visual work changed.
    pub fn set_appearance_mode(&mut self, mode: AppearanceMode) -> Result<bool, UiError> {
        self.update_theme(None, mode, self.theme.preferences)
    }

    /// Installs a validated theme revision and returns whether visual work changed.
    pub fn set_theme(&mut self, theme: Theme) -> Result<bool, UiError> {
        self.update_theme(Some(theme), self.theme.mode, self.theme.preferences)
    }

    /// Publishes an owned preference snapshot and resolves affected visual properties.
    pub fn set_system_preferences(
        &mut self,
        mut preferences: SystemPreferences,
    ) -> Result<bool, UiError> {
        preferences.text_scale = preferences
            .text_scale
            .filter(|scale| scale.is_finite() && *scale > 0.0);
        preferences.accent_color = preferences.accent_color.filter(|color| {
            let [r, g, b, a] = color.to_srgb();
            [r, g, b, a].into_iter().all(f32::is_finite) && (0.0..=1.0).contains(&a)
        });
        self.update_theme(None, self.theme.mode, preferences)
    }

    fn update_theme(
        &mut self,
        candidate: Option<Theme>,
        mode: AppearanceMode,
        preferences: SystemPreferences,
    ) -> Result<bool, UiError> {
        if self.closed {
            return Err(UiError::Closed);
        }
        let current = &self.theme;
        let next_environment = environment(mode, preferences);
        let scale = preferences.text_scale.unwrap_or(1.0);
        let theme = candidate.as_ref().unwrap_or(&current.theme);
        let mut affected = HashMap::new();
        let mut group_updates = Vec::new();

        if theme != &current.theme
            || next_environment != environment(current.mode, current.preferences)
        {
            for (consumer, group) in &current.consumers {
                let values = consumer.resolve(theme, next_environment);
                if values != group.values {
                    affected.extend(group.nodes.iter().map(|id| (*id, values)));
                    group_updates.push((consumer.clone(), values));
                }
            }
        }
        if scale != current.preferences.text_scale.unwrap_or(1.0) {
            for &id in &current.text_nodes {
                if let Entry::Vacant(entry) = affected.entry(id) {
                    let values = self
                        .node(id)?
                        .theme
                        .as_ref()
                        .and_then(|styles| styles.consumer.as_ref())
                        .and_then(|consumer| current.consumers.get(consumer))
                        .map_or_else(ThemeStyle::default, |group| group.values);
                    entry.insert(values);
                }
            }
        }

        let updates = affected
            .into_iter()
            .map(|(id, values)| {
                let state = self
                    .node(id)?
                    .theme
                    .as_ref()
                    .map_or(InteractionState::default(), |styles| styles.state);
                self.prepare_style_update(id, state, values, scale)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A theme revision cannot change `display`. All fallible style and
        // text preparation takes place before any of these commits.
        let mut changed = false;
        for update in updates {
            changed |= self.commit_style_update(update)?;
        }
        for (consumer, values) in group_updates {
            if let Some(group) = self.theme.consumers.get_mut(&consumer) {
                group.values = values;
            }
        }
        if let Some(theme) = candidate {
            self.theme.theme = theme;
        }
        self.theme.mode = mode;
        if preferences != self.theme.preferences {
            self.theme.preferences = preferences;
            self.theme.signals.publish(preferences)?;
        }
        Ok(changed)
    }

    pub(super) fn install_node_styles(
        &mut self,
        id: NodeId,
        local: ViewStyles,
        complete: Option<Style>,
    ) -> Result<(), UiError> {
        for patch in local.base.iter().chain(local.variants.iter().flatten()) {
            patch
                .validate()
                .map_err(|error| UiError::Style(error.to_string()))?;
        }
        let tracks_state = local.widget.is_some()
            || local.variants.iter().any(Option::is_some)
            || local
                .base
                .as_ref()
                .is_some_and(|base| base.focus_ring.is_some());
        self.node_mut(id)?.theme = Some(Box::new(NodeStyles {
            local,
            complete: complete.map(Box::new),
            state: InteractionState::default(),
            tracks_state,
            pending: false,
            consumer: None,
            focus_ring: None,
        }));
        self.refresh_theme_style(id, &mut |_, _, _| {}, &mut Work::default())?;
        Ok(())
    }

    pub(super) fn install_text_style(&mut self, id: NodeId) -> Result<(), UiError> {
        self.theme.text_nodes.insert(id);
        self.refresh_theme_style(id, &mut |_, _, _| {}, &mut Work::default())?;
        Ok(())
    }

    pub(super) fn set_local_style(
        &mut self,
        id: NodeId,
        style: Style,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<bool, UiError> {
        self.validate_style(&style)?;
        let Some(styles) = &mut self.node_mut(id)?.theme else {
            self.prepare_display_suppression(id, style.layout.display, handler, work)?;
            return self.apply_style(id, style);
        };
        let previous = styles.complete.replace(Box::new(style));
        let result = self.refresh_theme_style(id, handler, work);
        if result.is_err()
            && let Some(styles) = &mut self.node_mut(id)?.theme
        {
            styles.complete = previous;
        }
        result
    }

    fn refresh_theme_style(
        &mut self,
        id: NodeId,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<bool, UiError> {
        let node = self.node(id)?;
        let state = if node.theme.is_some() {
            self.interaction_state(id)?
        } else {
            InteractionState::default()
        };
        let old_consumer = node
            .theme
            .as_ref()
            .and_then(|styles| styles.consumer.clone());
        let consumer = node
            .theme
            .as_ref()
            .and_then(|styles| styles.consumer(state, node.text.is_some()));
        let values = consumer
            .as_ref()
            .map_or_else(ThemeStyle::default, |key| self.theme.values_for(key));
        let update = self.prepare_style_update(
            id,
            state,
            values,
            self.theme.preferences.text_scale.unwrap_or(1.0),
        )?;
        if let Some(style) = &update.style {
            self.prepare_display_suppression(id, style.layout.display, handler, work)?;
        }
        let changed = self.commit_style_update(update)?;
        if consumer != old_consumer {
            if let Some(old) = old_consumer {
                self.theme.remove_consumer(id, &old);
            }
            if let Some(key) = &consumer {
                self.theme
                    .consumers
                    .entry(key.clone())
                    .or_insert_with(|| Consumers {
                        values,
                        nodes: HashSet::new(),
                    })
                    .nodes
                    .insert(id);
            }
        }
        if let Some(styles) = &mut self.node_mut(id)?.theme {
            styles.state = state;
            styles.consumer = consumer;
        }
        Ok(changed)
    }

    pub(super) fn mark_interaction_style(&mut self, id: NodeId) -> Result<(), UiError> {
        if let Some(styles) = &mut self.node_mut(id)?.theme
            && styles.tracks_state
            && !styles.pending
        {
            styles.pending = true;
            self.theme.pending.push_back(id);
        }
        Ok(())
    }

    // Each round visits only nodes affected by state changes. Cleanup can mark
    // additional nodes for the following round.
    pub(super) fn sync_interaction_styles(
        &mut self,
        handler: &mut Handler<'_>,
        work: &mut Work,
    ) -> Result<bool, UiError> {
        for _ in 0..self.theme.pending.len() {
            let Some(id) = self.theme.pending.pop_front() else {
                break;
            };
            if let Ok(node) = self.node_mut(id)
                && let Some(styles) = &mut node.theme
            {
                styles.pending = false;
                self.refresh_theme_style(id, handler, work)?;
            }
        }
        Ok(!self.theme.pending.is_empty())
    }

    pub(super) fn untrack_node_styles(&mut self, id: NodeId) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        let has_text = node.text.is_some();
        let consumer = node
            .theme
            .as_mut()
            .and_then(|styles| styles.consumer.take());
        if let Some(consumer) = consumer {
            self.theme.remove_consumer(id, &consumer);
        }
        if has_text {
            self.theme.text_nodes.remove(&id);
        }
        Ok(())
    }

    pub(super) fn clear_theme_consumers(&mut self) {
        self.theme.consumers.clear();
        self.theme.text_nodes.clear();
        self.theme.pending.clear();
    }

    fn prepare_style_update(
        &self,
        id: NodeId,
        state: InteractionState,
        values: ThemeStyle,
        scale: f64,
    ) -> Result<StyleUpdate, UiError> {
        let node = self.node(id)?;
        let (style, values) = if let Some(styles) = &node.theme {
            let (style, values) = styles.resolve(state, values);
            self.validate_style(&style)?;
            (Some(style), values)
        } else {
            (None, values)
        };
        let mut remeasure = false;
        let mut repaint_text = false;
        let text = if let Some(text) = &node.text {
            let appearance = text.resolve_appearance(values.text_color, values.text_size, scale);
            let old = text.appearance();
            remeasure = text.appearance_changes_metrics(appearance)?;
            repaint_text = remeasure || appearance.color != old.color;
            (appearance != old).then_some(appearance)
        } else {
            None
        };
        if remeasure && node.content_revision == u64::MAX {
            return Err(UiError::RevisionExhausted);
        }
        let focus_ring = values
            .focus_ring
            .filter(|ring| state.focused && ring.width > 0.0);
        Ok(StyleUpdate {
            id,
            style,
            focus_ring,
            text,
            remeasure,
            repaint_text,
        })
    }

    fn commit_style_update(&mut self, update: StyleUpdate) -> Result<bool, UiError> {
        let mut changed = false;
        if let Some(style) = update.style {
            changed |= self.apply_style(update.id, style)?;
        }

        let node = self.node_mut(update.id)?;
        let mut previous_ring = None;
        if let Some(styles) = &mut node.theme {
            previous_ring = std::mem::replace(&mut styles.focus_ring, update.focus_ring);
        }
        if let Some(appearance) = update.text
            && let Some(text) = &node.text
        {
            text.set_appearance(appearance);
            if update.repaint_text {
                node.painting = None;
            }
        }
        if update.remeasure {
            self.bump_measure_revision(update.id)?;
        } else if update.repaint_text {
            self.dirty_paint();
        }
        if previous_ring.map(|ring| ring.width) != update.focus_ring.map(|ring| ring.width) {
            self.dirty_paint();
        } else if previous_ring != update.focus_ring
            && let Some(draws) = self.dirty_draws(update.id)
        {
            draws.focus_ring = true;
        }
        Ok(changed || update.repaint_text || previous_ring != update.focus_ring)
    }
}

fn environment(mode: AppearanceMode, preferences: SystemPreferences) -> ThemeEnvironment {
    let dark = match mode {
        AppearanceMode::System => matches!(preferences.appearance, Some(SystemAppearance::Dark)),
        AppearanceMode::Light => false,
        AppearanceMode::Dark => true,
    };
    ThemeEnvironment {
        dark,
        increased_contrast: preferences.increased_contrast.unwrap_or(false),
        accent_color: preferences.accent_color,
    }
}

#[cfg(test)]
mod tests;
