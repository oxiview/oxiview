use super::{CollectionError, MountStats};
use oxiview_base::Rect;
use oxiview_core::{NodeId, UiTree, View};
use oxiview_style::{Display, Edges, LayoutStyle, Length, Overflow, Position, Style};
use std::{collections::HashMap, hash::Hash};

pub(super) struct Placement<K, T> {
    pub key: K,
    pub parent: NodeId,
    pub index: usize,
    pub frame: Rect,
    pub item: T,
}

#[derive(Clone, Copy)]
struct Mounted {
    parent: NodeId,
    wrapper: NodeId,
    view: NodeId,
    frame: Rect,
}

pub(super) struct Mounts<K>(HashMap<K, Mounted>);

impl<K> Default for Mounts<K> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<K: Copy + Eq + Hash> Mounts<K> {
    pub fn view(&self, key: K) -> Option<NodeId> {
        self.0.get(&key).map(|item| item.view)
    }

    pub fn views(&self) -> impl Iterator<Item = (K, NodeId)> + '_ {
        self.0.iter().map(|(&key, item)| (key, item.view))
    }

    pub fn remove(&mut self, tree: &mut UiTree, key: K) -> Result<bool, CollectionError> {
        let Some(item) = self.0.get(&key) else {
            return Ok(false);
        };
        if tree.contains(item.wrapper) {
            tree.remove(item.wrapper)?;
        }
        self.0.remove(&key);
        Ok(true)
    }

    pub fn clear(&mut self, tree: &mut UiTree) -> Result<(), CollectionError> {
        for key in self.0.keys().copied().collect::<Vec<_>>() {
            self.remove(tree, key)?;
        }
        Ok(())
    }

    pub fn prune(
        &mut self,
        tree: &mut UiTree,
        mut keep: impl FnMut(K) -> Option<NodeId>,
    ) -> Result<usize, CollectionError> {
        let stale: Vec<_> = self
            .0
            .iter()
            .filter(|&(&key, item)| {
                keep(key) != Some(item.parent)
                    || !tree.contains(item.view)
                    || tree.parent(item.wrapper).ok().flatten() != Some(item.parent)
                    || (item.view != item.wrapper
                        && tree.parent(item.view).ok().flatten() != Some(item.wrapper))
            })
            .map(|(&key, _)| key)
            .collect();
        let count = stale.len();
        for key in stale {
            self.remove(tree, key)?;
        }
        Ok(count)
    }

    pub fn place<T>(
        &mut self,
        tree: &mut UiTree,
        placements: impl IntoIterator<Item = Placement<K, T>>,
        mut build: impl FnMut(T) -> Option<View>,
    ) -> Result<MountStats, CollectionError> {
        let mut stats = MountStats::default();
        let mut pending = Vec::new();
        let result = (|| {
            for Placement {
                key,
                parent,
                index,
                frame,
                item,
            } in placements
            {
                if let Some(mounted) = self.0.get_mut(&key) {
                    if mounted.frame != frame {
                        tree.set_style(mounted.wrapper, wrapper_style(frame))?;
                        mounted.frame = frame;
                        stats.repositioned += 1;
                    }
                    if tree.children(parent)?.get(index) != Some(&mounted.wrapper) {
                        tree.reparent(mounted.wrapper, parent, index)?;
                    }
                } else {
                    let wrapper = tree.insert(parent, index, wrapper_style(frame))?;
                    pending.push((
                        key,
                        Mounted {
                            parent,
                            wrapper,
                            view: wrapper,
                            frame,
                        },
                        build(item),
                    ));
                }
            }
            let mut views = Vec::new();
            let mut mounted_items = Vec::new();
            for (_, mounted, view) in &mut pending {
                if let Some(view) = view.take() {
                    views.push((mounted.wrapper, view));
                    mounted_items.push(mounted);
                }
            }
            for (mounted, view) in mounted_items.into_iter().zip(tree.mount_many(views)?) {
                mounted.view = view;
            }
            Ok::<(), CollectionError>(())
        })();
        if let Err(error) = result {
            for (_, item, _) in pending.into_iter().rev() {
                if tree.contains(item.wrapper) {
                    tree.remove(item.wrapper)?;
                }
            }
            return Err(error);
        }
        stats.constructed = pending.len();
        self.0
            .extend(pending.into_iter().map(|(key, mounted, _)| (key, mounted)));
        stats.mounted = self.0.len();
        Ok(stats)
    }
}

fn wrapper_style(frame: Rect) -> Style {
    Style {
        layout: LayoutStyle {
            display: Display::Grid,
            position: Position::Absolute,
            width: Length::Points(frame.size().width()),
            height: Length::Points(frame.size().height()),
            inset: Edges {
                left: Length::Points(frame.origin().x()),
                top: Length::Points(frame.origin().y()),
                right: Length::Auto,
                bottom: Length::Auto,
            },
            ..LayoutStyle::default()
        },
        overflow: Overflow::Clip,
        hit_test: false,
        ..Style::default()
    }
}
