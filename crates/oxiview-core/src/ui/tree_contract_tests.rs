//! S1 structure and identity contracts.
#![allow(clippy::unwrap_used)]

use super::*;
use oxiview_platform::WindowEvent;
use proptest::prelude::*;
use std::{
    cell::Cell,
    collections::HashSet,
    hash::{DefaultHasher, Hash, Hasher},
    time::Duration,
};

/// The reference tree owns children directly and has no arena, parent links,
/// generations, or deferred work. Keys index the test's history of real IDs.
#[derive(Clone)]
struct ReferenceNode {
    key: usize,
    children: Vec<ReferenceNode>,
}

impl ReferenceNode {
    fn new(key: usize) -> Self {
        Self {
            key,
            children: Vec::new(),
        }
    }

    fn find(&mut self, key: usize) -> Option<&mut Self> {
        if self.key == key {
            Some(self)
        } else {
            self.children.iter_mut().find_map(|child| child.find(key))
        }
    }

    fn take(&mut self, key: usize) -> Option<Self> {
        if let Some(index) = self.children.iter().position(|child| child.key == key) {
            return Some(self.children.remove(index));
        }
        self.children.iter_mut().find_map(|child| child.take(key))
    }

    fn visit(&self, parent: Option<usize>, visitor: &mut impl FnMut(&Self, Option<usize>)) {
        visitor(self, parent);
        for child in &self.children {
            child.visit(Some(self.key), visitor);
        }
    }
}

fn missing(key: usize) -> UiError {
    if key == 1 {
        UiError::ForeignNode
    } else {
        UiError::StaleNode
    }
}

fn select(selector: u8, live: &[usize], history_len: usize) -> usize {
    let selector = usize::from(selector);
    if selector.is_multiple_of(4) {
        (selector / 4) % history_len
    } else {
        live[selector % live.len()]
    }
}

fn insertion_index(length: usize, selector: u8) -> usize {
    match selector % 4 {
        0 => 0,
        1 => length / 2,
        2 => length,
        _ => length + 1,
    }
}

fn tagged_style(key: usize) -> Style {
    let mut style = Style::default();
    style.layout.width = oxiview_style::Length::Points(key as f64 + 1.0);
    style
}

fn assert_structure(model: &ReferenceNode, tree: &UiTree, ids: &[NodeId]) {
    let mut live = HashSet::new();
    model.visit(None, &mut |node, parent| {
        let id = ids[node.key];
        assert!(live.insert(id));
        assert_eq!(tree.parent(id).unwrap(), parent.map(|key| ids[key]));
        let children: Vec<_> = node.children.iter().map(|child| ids[child.key]).collect();
        assert_eq!(tree.children(id).unwrap(), children);
        assert_eq!(tree.style(id).unwrap(), &tagged_style(node.key));
    });
    assert_eq!(tree.len(), live.len());
    assert_eq!(tree.root(), ids[0]);
    for (key, id) in ids.iter().copied().enumerate() {
        assert_eq!(tree.contains(id), live.contains(&id));
        if !live.contains(&id) {
            assert_eq!(tree.parent(id), Err(missing(key)));
        }
    }
}

proptest! {
    #[test]
    fn generated_arena_edits_match_nested_reference_tree(
        edits in prop::collection::vec(any::<[u8; 4]>(), 1..64),
    ) {
        let mut tree = UiTree::new().unwrap();
        let foreign = UiTree::new().unwrap();
        tree.set_style(tree.root(), tagged_style(0)).unwrap();
        let mut ids = vec![tree.root(), foreign.root()];
        let mut model = ReferenceNode::new(0);

        for [kind, source, destination, index] in edits {
            let mut live = Vec::new();
            model.visit(None, &mut |node, _| live.push(node.key));
            let source = select(source, &live, ids.len());
            let destination = select(destination, &live, ids.len());
            let index = usize::from(index);
            match kind % 6 {
                0..=2 => {
                    let key = ids.len();
                    let length = model.find(destination).map(|node| node.children.len());
                    let index = insertion_index(length.unwrap_or(0), index as u8);
                    let actual = if kind % 6 == 0 {
                        tree.append(ids[destination], tagged_style(key))
                    } else {
                        tree.insert(ids[destination], index, tagged_style(key))
                    };
                    let expected = match model.find(destination) {
                        None => Err(missing(destination)),
                        Some(parent) => {
                            let len = parent.children.len();
                            let index = if kind % 6 == 0 { len } else { index };
                            if index > len {
                                Err(UiError::ChildIndex { index, len })
                            } else {
                                parent.children.insert(index, ReferenceNode::new(key));
                                Ok(())
                            }
                        }
                    };
                    prop_assert_eq!(actual.as_ref().map(|_| ()).map_err(Clone::clone), expected);
                    if let Ok(id) = actual {
                        prop_assert!(!ids.contains(&id));
                        ids.push(id);
                    }
                }
                3 => {
                    let expected = if source == 0 {
                        Err(UiError::RootOperation)
                    } else {
                        model.take(source).map(|_| ()).ok_or_else(|| missing(source))
                    };
                    prop_assert_eq!(tree.remove(ids[source]), expected);
                }
                4 => {
                    let expected = model.find(source).ok_or_else(|| missing(source)).map(|node| {
                        node.children.clear();
                    });
                    prop_assert_eq!(tree.clear(ids[source]), expected);
                }
                _ => {
                    let mut candidate = model.clone();
                    let length = model.find(destination).map_or(0, |node| {
                        node.children.iter().filter(|child| child.key != source).count()
                    });
                    let index = insertion_index(length, index as u8);
                    let expected = (|| {
                        if source == 0 {
                            return Err(UiError::RootOperation);
                        }
                        if candidate.find(source).is_none() {
                            return Err(missing(source));
                        }
                        if candidate.find(destination).is_none() {
                            return Err(missing(destination));
                        }
                        if index > length {
                            return Err(UiError::ChildIndex { index, len: length });
                        }
                        let subtree = candidate.take(source).unwrap();
                        let destination = candidate.find(destination).ok_or(UiError::Cycle)?;
                        destination.children.insert(index, subtree);
                        Ok(())
                    })();
                    prop_assert_eq!(tree.reparent(ids[source], ids[destination], index), expected.clone());
                    if expected.is_ok() {
                        model = candidate;
                    }
                }
            }
            assert_structure(&model, &tree, &ids);
            prop_assert_eq!(foreign.len(), 1);
            prop_assert_eq!(foreign.children(foreign.root()).unwrap(), &[]);
        }
    }
}

#[test]
fn owners_reuse_and_exhaustion_preserve_identity() {
    let mut tree = UiTree::new().unwrap();
    let other = UiTree::new().unwrap();
    let root = tree.root();
    let other_root = other.root();
    assert_eq!(
        (root.slot, root.generation),
        (other_root.slot, other_root.generation)
    );
    assert_ne!(root, other_root);
    assert_eq!(other.parent(root), Err(UiError::ForeignNode));
    let copied = root;
    let hash = |id: NodeId| {
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(hash(copied), hash(root));
    assert_eq!(HashSet::from([root, copied, other_root]).len(), 2);

    let old = tree.append(root, Style::default()).unwrap();
    tree.remove(old).unwrap();
    let reused = tree.append(root, Style::default()).unwrap();
    assert_eq!(old.slot, reused.slot);
    assert_ne!(old.generation, reused.generation);
    assert_eq!(tree.parent(old), Err(UiError::StaleNode));

    tree.remove(reused).unwrap();
    tree.identities.borrow_mut().slots[old.slot].generation = u64::MAX - 1;
    let penultimate = tree.append(root, Style::default()).unwrap();
    assert_eq!(penultimate.generation, u64::MAX - 1);
    tree.remove(penultimate).unwrap();
    let reservation = Reservation::new(&tree.identities).unwrap();
    let final_id = reservation.id;
    assert_eq!(final_id.slot, old.slot);
    assert_eq!(final_id.generation, u64::MAX);
    assert_eq!(tree.parent(final_id), Err(UiError::NotYetCreated));
    drop(reservation);
    let next = tree
        .command_sender()
        .append(root, Style::default())
        .unwrap();
    assert_ne!(next.slot, final_id.slot);
    assert_eq!(tree.parent(final_id), Err(UiError::StaleNode));
    drop(tree);
    let recreated = UiTree::new().unwrap();
    for id in [root, old, reused, penultimate, final_id, next] {
        assert_eq!(recreated.parent(id), Err(UiError::ForeignNode));
    }

    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(allocate_tree_owner(&counter), Ok(u64::MAX - 1));
    for _ in 0..2 {
        assert_eq!(
            allocate_tree_owner(&counter),
            Err(UiError::IdentityExhausted)
        );
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}

#[test]
fn rejected_edits_and_original_position_moves_preserve_clean_and_dirty_work() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let branch = tree.append(root, tagged_style(9)).unwrap();
    let leaf = tree.append(branch, tagged_style(2)).unwrap();
    let viewport = Size::new(120.0, 80.0);
    tree.paint(viewport, 1.0).unwrap();
    let clean = tree.diagnostics();

    assert_eq!(
        tree.insert(branch, 2, Style::default()),
        Err(UiError::ChildIndex { index: 2, len: 1 })
    );
    assert_eq!(tree.reparent(root, branch, 0), Err(UiError::RootOperation));
    assert_eq!(tree.reparent(branch, leaf, 0), Err(UiError::Cycle));
    assert_eq!(tree.children(branch).unwrap(), &[leaf]);
    tree.reparent(branch, root, 0).unwrap();
    tree.paint(viewport, 1.0).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, clean.layout_passes);
    assert_eq!(
        tree.diagnostics().scene_cache_hits,
        clean.scene_cache_hits + 1
    );

    tree.set_style(leaf, tagged_style(3)).unwrap();
    tree.reparent(branch, root, 0).unwrap();
    tree.paint(viewport, 1.0).unwrap();
    assert_eq!(tree.diagnostics().layout_passes, clean.layout_passes + 1);
    assert_eq!(tree.bounds(leaf).unwrap().size().width(), 4.0);
}

#[test]
fn moving_a_subtree_past_depth_limit_is_atomic() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let branch = tree.append(root, Style::default()).unwrap();
    let leaf = tree.append(branch, Style::default()).unwrap();
    let mut destination = root;
    for _ in 1..MAX_DEPTH - 1 {
        destination = tree.append(destination, Style::default()).unwrap();
    }
    let before = tree.children(root).unwrap().to_vec();
    let count = tree.len();
    assert_eq!(
        tree.reparent(branch, destination, 0),
        Err(UiError::DepthLimit)
    );
    assert_eq!(tree.len(), count);
    assert_eq!(tree.children(root).unwrap(), before);
    assert_eq!(tree.parent(branch).unwrap(), Some(root));
    assert_eq!(tree.parent(leaf).unwrap(), Some(branch));
    assert_eq!(tree.children(destination).unwrap(), &[]);
    tree.reparent(leaf, destination, 0).unwrap();
    assert_eq!(tree.parent(leaf).unwrap(), Some(destination));
}

struct Release {
    label: usize,
    trace: Rc<RefCell<Vec<usize>>>,
}

impl Drop for Release {
    fn drop(&mut self) {
        self.trace.borrow_mut().push(self.label);
    }
}

#[test]
fn queued_resources_transfer_or_drop_once_with_their_reservations() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let foreign = UiTree::new().unwrap();
    let sender = tree.command_sender();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let calls = Rc::new(Cell::new(0));
    let post_handler = |node, label| {
        let release = Release {
            label,
            trace: Rc::clone(&trace),
        };
        let calls = Rc::clone(&calls);
        sender
            .post(input::Command::Handler(
                node,
                Box::new(move |_, _, _| {
                    let _release = &release;
                    calls.set(calls.get() + 1);
                }),
            ))
            .unwrap();
    };

    let failed = sender.append(foreign.root(), Style::default()).unwrap();
    post_handler(failed, 1);
    let live = sender.append(root, Style::default()).unwrap();
    post_handler(live, 2);
    // A caller need not keep even a single copy of a queued creation's ID.
    sender.append(root, Style::default()).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree.children(root).unwrap(), &[]);
    assert_eq!(tree.style(live), Err(UiError::NotYetCreated));
    assert_eq!(foreign.style(live), Err(UiError::ForeignNode));
    assert!(trace.borrow().is_empty());

    let viewport = Size::new(200.0, 100.0);
    let result = tree.flush_updates(viewport, |_, _, _| {}).unwrap();
    let outcomes = &result.reports[0].outcomes;
    assert_eq!(outcomes[0].result, Err(UiError::ForeignNode));
    assert_eq!(outcomes[1].result, Err(UiError::StaleNode));
    assert_eq!(tree.style(failed), Err(UiError::StaleNode));
    assert_eq!(outcomes[2].node, Some(live));
    assert!(outcomes[2..].iter().all(|outcome| outcome.result.is_ok()));
    assert_eq!(tree.len(), 3);
    assert_eq!(tree.children(root).unwrap()[0], live);
    assert_eq!(tree.parent(live).unwrap(), Some(root));
    assert_eq!(*trace.borrow(), vec![1]);

    let canceled = sender.append(root, Style::default()).unwrap();
    post_handler(canceled, 3);
    assert_eq!(canceled.slot, failed.slot);
    assert_ne!(canceled, failed);
    assert_eq!(tree.style(failed), Err(UiError::StaleNode));
    let result = tree
        .dispatch(&WindowEvent::Destroyed, viewport, |_, _, _| {})
        .unwrap();
    assert_eq!(result.events, 0);
    assert_eq!(calls.get(), 0);
    assert_eq!(tree.diagnostics().dispatched_handlers, 0);
    assert!(
        result.reports[0]
            .outcomes
            .iter()
            .all(|outcome| outcome.result == Err(UiError::QueueClosed))
    );
    assert_eq!(*trace.borrow(), vec![1, 3, 2]);
    for id in [live, canceled, root] {
        assert_eq!(tree.style(id), Err(UiError::Closed));
    }
    drop(tree);
    assert_eq!(*trace.borrow(), vec![1, 3, 2]);
    assert_eq!(sender.append(root, Style::default()), Err(UiError::Closed));
}

#[test]
fn teardown_respects_the_current_order_and_keeps_root_state_on_clear() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let left = tree.append(root, tagged_style(1)).unwrap();
    let right = tree.append(root, tagged_style(2)).unwrap();
    let leaf = tree.append(left, tagged_style(3)).unwrap();
    tree.set_style(root, tagged_style(0)).unwrap();
    tree.set_semantics(
        root,
        Some(NodeSemantics {
            label: "Root".into(),
            ..NodeSemantics::default()
        }),
    )
    .unwrap();

    let trace = Rc::new(RefCell::new(Vec::new()));
    let root_calls = Rc::new(Cell::new(0));
    for (label, id) in [root, left, right, leaf].into_iter().enumerate() {
        let release = Release {
            label,
            trace: Rc::clone(&trace),
        };
        let root_calls = Rc::clone(&root_calls);
        tree.set_pointer_handler(id, move |_, _, _| {
            let _release = &release;
            if release.label == 0 {
                root_calls.set(root_calls.get() + 1);
            }
        })
        .unwrap();
    }

    tree.reparent(leaf, right, 0).unwrap();
    tree.reparent(right, root, 0).unwrap();
    assert_eq!(tree.style(leaf).unwrap(), &tagged_style(3));
    tree.clear(root).unwrap();
    assert_eq!(*trace.borrow(), vec![3, 2, 1]);
    assert_eq!(tree.root(), root);
    assert_eq!(tree.style(root).unwrap(), &tagged_style(0));
    assert_eq!(tree.semantics(root).unwrap().unwrap().label, "Root");
    let new_leaf = tree.append(root, Style::default()).unwrap();
    assert_eq!(tree.parent(new_leaf).unwrap(), Some(root));
    tree.dispatch(
        &WindowEvent::PointerMoved {
            position: Point::new(0.5, 0.5),
            timestamp: Duration::ZERO,
        },
        Size::new(200.0, 100.0),
        |_, _, _| {},
    )
    .unwrap();
    assert_eq!(root_calls.get(), 2);
    drop(tree);
    assert_eq!(*trace.borrow(), vec![3, 2, 1, 0]);
}

#[test]
fn disposed_pointer_handler_owner_skips_only_the_callback() {
    let mut tree = UiTree::new().unwrap();
    let root = tree.root();
    let calls = Rc::new(Cell::new(0));
    let recorded = Rc::clone(&calls);
    tree.set_pointer_handler(root, move |_, _, _| recorded.set(recorded.get() + 1))
        .unwrap();
    tree.node(root).unwrap().scopes[0].dispose().unwrap();

    let mut observed = 0;
    let result = tree
        .dispatch(
            &WindowEvent::PointerMoved {
                position: Point::new(5.0, 5.0),
                timestamp: Duration::ZERO,
            },
            Size::new(100.0, 80.0),
            |_, _, _| observed += 1,
        )
        .unwrap();
    assert_eq!(result.events, 2);
    assert_eq!(observed, result.events);
    assert_eq!(calls.get(), 0);
    assert_eq!(tree.diagnostics().dispatched_handlers, 0);
}

#[test]
fn teardown_releases_every_deep_resource_before_its_parent() {
    let mut tree = UiTree::new().unwrap();
    let mut node = tree.root();
    let trace = Rc::new(RefCell::new(Vec::new()));
    for level in 1..=MAX_DEPTH {
        let release = Release {
            label: level,
            trace: Rc::clone(&trace),
        };
        tree.set_pointer_handler(node, move |_, _, _| {
            let _release = &release;
        })
        .unwrap();
        if level < MAX_DEPTH {
            node = tree.append(node, Style::default()).unwrap();
        }
    }
    drop(tree);
    assert!(trace.borrow().iter().copied().eq((1..=MAX_DEPTH).rev()));
}
