use super::{
    NodeId, NodeMeasure, NodeSemantics, UiCommandSender, UiCommands, UiError, UiPointerEvent,
    UiTree,
    input::{Command, NodeHandler},
};
use oxiview_reactive::{Owner, ReactiveError, Scope, Signal, batch, effect, signal};
use oxiview_scene::Primitive;
use oxiview_style::{Display, Style};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::Hash,
    rc::Rc,
};

type Factory = Box<dyn FnOnce() -> Result<View, UiError>>;
pub(super) type Binding = Box<dyn FnOnce(NodeId, UiCommandSender) -> Result<(), UiError>>;
pub(super) type Setup = Box<dyn FnOnce(&mut UiTree, NodeId, Owner) -> Result<(), UiError>>;

/// An ephemeral construction description consumed once when mounted.
/// The framework retains only the resulting UI nodes, scopes, and bindings.
pub struct View {
    factory: Option<Factory>,
    style: Option<Style>,
    children: Vec<View>,
    handler: Option<Box<NodeHandler>>,
    content: Option<Vec<Primitive>>,
    measure: Option<Box<dyn NodeMeasure>>,
    semantics: Option<NodeSemantics>,
    pub(super) bindings: Vec<Binding>,
    pub(super) setup: Vec<Setup>,
    mounted: Vec<Box<dyn FnOnce(NodeId)>>,
}
/// Starts an ordinary retained node builder.
pub fn node() -> View {
    View {
        factory: None,
        style: None,
        children: Vec::new(),
        handler: None,
        content: None,
        measure: None,
        semantics: None,
        bindings: Vec::new(),
        setup: Vec::new(),
        mounted: Vec::new(),
    }
}
/// Defers a function component until mount; it executes once for that mount.
pub fn component(factory: impl FnOnce() -> Result<View, UiError> + 'static) -> View {
    View {
        factory: Some(Box::new(factory)),
        ..node()
    }
}
impl View {
    /// Sets initial typed style.
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }
    /// Appends an ordered child description.
    pub fn child(mut self, child: View) -> Self {
        self.children.push(child);
        self
    }
    /// Appends ordered children.
    pub fn children(mut self, children: impl IntoIterator<Item = View>) -> Self {
        self.children.extend(children);
        self
    }
    /// Installs a node-owned pointer handler.
    pub fn on_pointer(
        mut self,
        handler: impl FnMut(&UiPointerEvent, &UiTree, &mut UiCommands) + 'static,
    ) -> Self {
        self.handler = Some(Box::new(handler));
        self
    }
    /// Sets node-local custom Scene primitives.
    pub fn content(mut self, content: Vec<Primitive>) -> Self {
        self.content = Some(content);
        self
    }
    /// Installs intrinsic measurement without a layout-engine dependency.
    pub fn measure(mut self, measure: impl NodeMeasure + 'static) -> Self {
        self.measure = Some(Box::new(measure));
        self
    }
    /// Reserves owned semantic metadata.
    pub fn semantics(mut self, semantics: NodeSemantics) -> Self {
        self.semantics = Some(semantics);
        self
    }
    /// Receives the real identity after node construction; the callback runs once.
    pub fn on_mount(mut self, callback: impl FnOnce(NodeId) + 'static) -> Self {
        self.mounted.push(Box::new(callback));
        self
    }
    /// Binds complete typed style to the signals read by this computation.
    pub fn bind_style(
        mut self,
        mut compute: impl FnMut() -> Result<Style, ReactiveError> + 'static,
    ) -> Self {
        self.bindings.push(Box::new(move |id, sender| {
            effect(move || sender.set_style(id, compute()?).map_err(as_reactive)).map_err(as_ui)?;
            Ok(())
        }));
        self
    }
    /// Binds custom painting content without reconstructing the retained node.
    pub fn bind_content(
        mut self,
        mut compute: impl FnMut() -> Result<Vec<Primitive>, ReactiveError> + 'static,
    ) -> Self {
        self.bindings.push(Box::new(move |id, sender| {
            effect(move || sender.set_content(id, compute()?).map_err(as_reactive))
                .map_err(as_ui)?;
            Ok(())
        }));
        self
    }
    fn merge_component(mut inner: View, mut outer: View) -> View {
        if outer.style.is_some() {
            inner.style = outer.style.take();
        }
        if outer.handler.is_some() {
            inner.handler = outer.handler.take();
        }
        if outer.content.is_some() {
            inner.content = outer.content.take();
        }
        if outer.measure.is_some() {
            inner.measure = outer.measure.take();
        }
        if outer.semantics.is_some() {
            inner.semantics = outer.semantics.take();
        }
        inner.children.append(&mut outer.children);
        inner.bindings.append(&mut outer.bindings);
        inner.setup.append(&mut outer.setup);
        inner.mounted.append(&mut outer.mounted);
        inner
    }
}

impl UiTree {
    /// Consumes a Builder or function component under the parent, applying its
    /// initial bindings before returning. Dynamic handlers use deferred commands.
    pub fn mount(&mut self, parent: NodeId, view: View) -> Result<NodeId, UiError> {
        self.node(parent)?;
        let owner = self.lifetime_owner(parent)?;
        let result = in_owner(owner, || self.mount_view(parent, view))?;
        let applied = self.flush_updates(self.viewport, |_, _, _| {});
        let failure = match applied {
            Err(error) => Some(error),
            Ok(applied) => applied
                .reports
                .iter()
                .flat_map(|report| &report.outcomes)
                .find_map(|outcome| {
                    let belongs = outcome.node.is_some_and(|id| {
                        id == result || self.descendant(id, result).unwrap_or(false)
                    });
                    if belongs {
                        outcome.result.as_ref().err().cloned()
                    } else {
                        None
                    }
                }),
        };
        if let Some(error) = failure {
            if self.contains(result) {
                self.remove_raw(result)?;
            }
            return Err(error);
        }
        Ok(result)
    }
    fn mount_view(&mut self, parent: NodeId, view: View) -> Result<NodeId, UiError> {
        self.mount_inner(parent, view, 0)
    }
    fn mount_inner(
        &mut self,
        parent: NodeId,
        mut view: View,
        component_depth: usize,
    ) -> Result<NodeId, UiError> {
        if component_depth >= super::MAX_DEPTH {
            return Err(UiError::DepthLimit);
        }
        let scope = Scope::new().map_err(as_ui)?;
        let owner = scope.owner();
        if let Some(factory) = view.factory.take() {
            let inner = in_owner(owner, factory)?;
            let inner = View::merge_component(inner, view);
            let root = in_owner(owner, || {
                self.mount_inner(parent, inner, component_depth + 1)
            })?;
            self.node_mut(root)?.scopes.push(scope);
            return Ok(root);
        }
        let root = self.append_scoped(parent, view.style.take().unwrap_or_default(), scope)?;
        let result = in_owner(owner, || {
            if let Some(handler) = view.handler.take() {
                self.node_mut(root)?.handler = Some(Rc::new(RefCell::new(handler)));
            }
            if let Some(content) = view.content.take() {
                self.set_content(root, content)?;
            }
            if let Some(measure) = view.measure.take() {
                self.node_mut(root)?.measure = Some(measure);
            }
            if let Some(semantics) = view.semantics.take() {
                self.set_semantics(root, Some(semantics))?;
            }
            for child in view.children {
                self.mount_inner(root, child, component_depth)?;
            }
            for setup in view.setup {
                setup(self, root, owner)?;
            }
            for binding in view.bindings {
                binding(root, self.mailbox.scoped(owner))?;
            }
            for mounted in view.mounted {
                mounted(root);
            }
            Ok(())
        });
        if let Err(error) = result {
            self.remove_raw(root)?;
            return Err(error);
        }
        Ok(root)
    }
}

/// Creates a real container whose conditional child is mounted only while true.
/// The hidden branch and its scope are destroyed; the container identity persists.
pub fn show(
    condition: Signal<bool>,
    factory: impl Fn() -> Result<View, UiError> + 'static,
) -> View {
    let mut style = Style::default();
    style.layout.display = Display::Block;
    style.hit_test = false;
    let factory = Rc::new(factory);
    let mut view = node().style(style);
    view.setup.push(Box::new(move |tree, container, owner| {
        let sender = tree.mailbox.scoped(owner);
        let mounted = Rc::new(RefCell::new(None::<NodeId>));
        effect(move || {
            let visible = condition.get()?;
            let mounted = Rc::clone(&mounted);
            let factory = Rc::clone(&factory);
            sender
                .mutate(
                    container,
                    Box::new(move |tree, handler, work| {
                        let current = *mounted
                            .try_borrow()
                            .map_err(|_| UiError::CommandBufferBusy)?;
                        if visible {
                            if current.is_none_or(|id| !tree.contains(id)) {
                                let child = owner
                                    .run(|| {
                                        let scope = Scope::new()?;
                                        let child = scope.owner().run(|| {
                                            let view = factory().map_err(as_reactive)?;
                                            tree.mount_view(container, view).map_err(as_reactive)
                                        })?;
                                        tree.node_mut(child)
                                            .map_err(as_reactive)?
                                            .scopes
                                            .push(scope);
                                        Ok(child)
                                    })
                                    .map_err(as_ui)?;
                                *mounted
                                    .try_borrow_mut()
                                    .map_err(|_| UiError::CommandBufferBusy)? = Some(child);
                            }
                            tree.apply(Command::Visible(container, true), handler, work)?;
                        } else {
                            tree.apply(Command::Visible(container, false), handler, work)?;
                            if let Some(child) = current
                                && tree.contains(child)
                            {
                                tree.apply(Command::Remove(child), handler, work)?;
                            }
                            *mounted
                                .try_borrow_mut()
                                .map_err(|_| UiError::CommandBufferBusy)? = None;
                        }
                        Ok(())
                    }),
                )
                .map_err(as_reactive)
        })
        .map_err(as_ui)?;
        Ok(())
    }));
    view
}

struct Entry<T> {
    root: NodeId,
    item: Signal<T>,
}
impl<T> Copy for Entry<T> {}
impl<T> Clone for Entry<T> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Creates a real container with keyed children. Retained keys keep their node
/// identities and item signals; changed item data updates bindings without
/// rerunning their function components. Duplicate keys reject the update.
pub fn keyed_for<T, K>(
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    render: impl Fn(Signal<T>) -> Result<View, UiError> + 'static,
) -> View
where
    T: Clone + 'static,
    K: Clone + Eq + Hash + 'static,
{
    let mut style = Style::default();
    style.layout.display = Display::Block;
    style.hit_test = false;
    let mut view = node().style(style);
    let key = Rc::new(key);
    let render = Rc::new(render);
    view.setup.push(Box::new(move |tree, container, owner| {
        let entries = Rc::new(RefCell::new(HashMap::<K, Entry<T>>::new()));
        let sender = tree.mailbox.scoped(owner);
        effect(move || {
            let values = items.get()?;
            let entries = Rc::clone(&entries);
            let key = Rc::clone(&key);
            let render = Rc::clone(&render);
            sender
                .mutate(
                    container,
                    Box::new(move |tree, handler, work| {
                        let mut keys = HashSet::new();
                        let mut desired = Vec::new();
                        for value in values {
                            let item_key = key(&value);
                            if !keys.insert(item_key.clone()) {
                                return Err(UiError::DuplicateKey);
                            }
                            desired.push((item_key, value));
                        }
                        let old = entries
                            .try_borrow()
                            .map_err(|_| UiError::CommandBufferBusy)?
                            .clone();
                        let mut next = HashMap::new();
                        let mut created = Vec::new();
                        for (key, value) in &desired {
                            if let Some(entry) = old
                                .get(key)
                                .copied()
                                .filter(|entry| tree.contains(entry.root))
                            {
                                next.insert(key.clone(), entry);
                            } else {
                                let new_entry = owner
                                    .run(|| {
                                        let scope = Scope::new()?;
                                        let item_owner = scope.owner();
                                        let item = item_owner.run(|| signal(value.clone()))?;
                                        let child = item_owner.run(|| {
                                            let view = render(item).map_err(as_reactive)?;
                                            tree.mount_view(container, view).map_err(as_reactive)
                                        })?;
                                        tree.node_mut(child)
                                            .map_err(as_reactive)?
                                            .scopes
                                            .push(scope);
                                        Ok(Entry { root: child, item })
                                    })
                                    .map_err(as_ui);
                                match new_entry {
                                    Ok(entry) => {
                                        created.push(entry.root);
                                        next.insert(key.clone(), entry);
                                    }
                                    Err(error) => {
                                        for child in created {
                                            tree.remove_raw(child)?;
                                        }
                                        return Err(error);
                                    }
                                }
                            }
                        }
                        let removed: HashSet<_> = old
                            .iter()
                            .filter(|(key, _)| !keys.contains(*key))
                            .map(|(_, entry)| entry.root)
                            .collect();
                        for child in tree.children(container)?.to_vec() {
                            if removed.contains(&child) {
                                tree.apply(Command::Remove(child), handler, work)?;
                            }
                        }
                        for (index, (key, _)) in desired.iter().enumerate() {
                            let entry = next.get(key).ok_or(UiError::StaleNode)?;
                            tree.apply(
                                Command::Reparent(entry.root, container, index),
                                handler,
                                work,
                            )?;
                        }
                        batch(|| {
                            for (key, value) in &desired {
                                if let Some(entry) = next.get(key) {
                                    entry.item.set(value.clone())?;
                                }
                            }
                            Ok(())
                        })
                        .map_err(as_ui)?;
                        *entries
                            .try_borrow_mut()
                            .map_err(|_| UiError::CommandBufferBusy)? = next;
                        Ok(())
                    }),
                )
                .map_err(as_reactive)
        })
        .map_err(as_ui)?;
        Ok(())
    }));
    view
}

/// Alias for keyed list construction, convenient inside `view!` expressions.
pub fn for_each<T, K>(
    items: Signal<Vec<T>>,
    key: impl Fn(&T) -> K + 'static,
    render: impl Fn(Signal<T>) -> Result<View, UiError> + 'static,
) -> View
where
    T: Clone + 'static,
    K: Clone + Eq + Hash + 'static,
{
    keyed_for(items, key, render)
}

fn as_ui(error: ReactiveError) -> UiError {
    UiError::Reactive(error.to_string())
}
fn in_owner<T>(owner: Owner, action: impl FnOnce() -> Result<T, UiError>) -> Result<T, UiError> {
    owner.run(|| Ok(action())).map_err(as_ui)?
}
fn as_reactive(error: UiError) -> ReactiveError {
    ReactiveError::User(error.to_string())
}
