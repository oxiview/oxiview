//! M3 CPU workloads over live scopes and mounted views.
#![allow(clippy::expect_used)] // A rejected workload invalidates the measurement.

use oxiview_base::Size;
use oxiview_core::{UiTree, component, keyed_for, node, show};
use oxiview_reactive::{ReactiveError, Scope, batch, effect, memo, signal};
use oxiview_style::Style;
use std::{cell::Cell, rc::Rc};

fn main() {
    divan::main();
}

fn apply_bindings(tree: &mut UiTree) {
    let update = tree
        .flush_updates(Size::new(640.0, 480.0), |_, _, _| {})
        .expect("apply component updates");
    assert!(!update.reports.is_empty());
    assert!(
        update
            .reports
            .iter()
            .flat_map(|report| &report.outcomes)
            .all(|outcome| outcome.result.is_ok())
    );
}

#[divan::bench(sample_count = 100, sample_size = 100)]
fn batched_memo_diamond(bencher: divan::Bencher) {
    let scope = Scope::new().expect("scope");
    let observed = Rc::new(Cell::new(0));
    let output = Rc::clone(&observed);
    let source = scope
        .run(|| {
            let source = signal(0_u32)?;
            let left = memo(move || Ok(source.get()? + 1))?;
            let right = memo(move || Ok(source.get()? * 2))?;
            let sum = memo(move || Ok(left.get()? + right.get()?))?;
            effect(move || {
                output.set(sum.get()?);
                Ok(())
            })?;
            Ok(source)
        })
        .expect("memo graph");
    let mut final_value = 0;
    bencher.bench_local(|| {
        final_value = if final_value == 2 { 4 } else { 2 };
        batch(|| {
            source.set(1)?;
            source.set(divan::black_box(final_value))
        })
        .expect("batched writes");
        divan::black_box(observed.get())
    });
    assert_eq!(observed.get(), final_value * 3 + 1);
}

#[divan::bench(sample_count = 100, sample_size = 100)]
fn bound_style_update(bencher: divan::Bencher) {
    let scope = Scope::new().expect("scope");
    let dimmed = scope.run(|| signal(false)).expect("source");
    let mut tree = UiTree::new().expect("tree");
    let mounts = Rc::new(Cell::new(0));
    let mounted = Rc::clone(&mounts);
    let view = component(move || {
        mounted.set(mounted.get() + 1);
        Ok(node().bind_style(move || {
            Ok(Style {
                opacity: if dimmed.get()? { 0.5 } else { 1.0 },
                ..Style::default()
            })
        }))
    });
    let id = tree.mount(tree.root(), view).expect("mount component");
    dimmed.set(true).expect("initial opacity change");
    apply_bindings(&mut tree);
    assert_eq!(tree.style(id).expect("component style").opacity, 0.5);
    let mut dim = true;
    bencher.bench_local(|| {
        dim = !dim;
        dimmed.set(divan::black_box(dim)).expect("toggle opacity");
        apply_bindings(&mut tree);
    });
    assert_eq!(mounts.get(), 1);
    assert_eq!(
        tree.style(id).expect("component style").opacity,
        if dim { 0.5 } else { 1.0 }
    );
}

#[divan::bench(sample_count = 100, sample_size = 50)]
fn show_mount_and_dispose(bencher: divan::Bencher) {
    let scope = Scope::new().expect("scope");
    let visible = scope.run(|| signal(false)).expect("condition");
    let mut tree = UiTree::new().expect("tree");
    let last_state = Rc::new(Cell::new(None));
    let saved = Rc::clone(&last_state);
    let view = show(visible, move || {
        saved.set(Some(signal(0_u8).expect("branch-local signal")));
        Ok(node())
    });
    let container = tree.mount(tree.root(), view).expect("mount conditional");
    bencher.bench_local(|| {
        visible.set(true).expect("show");
        apply_bindings(&mut tree);
        visible.set(false).expect("hide");
        apply_bindings(&mut tree);
    });
    assert!(
        tree.children(container)
            .expect("conditional children")
            .is_empty()
    );
    let state = last_state.get().expect("branch was mounted");
    assert_eq!(state.get(), Err(ReactiveError::Disposed));
}

#[divan::bench(sample_count = 100, sample_size = 10)]
fn keyed_reorder_32(bencher: divan::Bencher) {
    let forward: Vec<u8> = (0..32).collect();
    let backward: Vec<u8> = forward.iter().copied().rev().collect();
    let scope = Scope::new().expect("scope");
    let rows = scope.run(|| signal(forward.clone())).expect("rows");
    let mut tree = UiTree::new().expect("tree");
    let mounts = Rc::new(Cell::new(0));
    let mounted = Rc::clone(&mounts);
    let view = keyed_for(
        rows,
        |key| *key,
        move |row| {
            mounted.set(mounted.get() + 1);
            Ok(node().bind_style(move || {
                Ok(Style {
                    opacity: if row.get()?.is_multiple_of(2) {
                        0.5
                    } else {
                        1.0
                    },
                    ..Style::default()
                })
            }))
        },
    );
    let container = tree.mount(tree.root(), view).expect("mount rows");
    let original = tree.children(container).expect("row identities").to_vec();
    rows.set(backward.clone()).expect("initial reorder");
    apply_bindings(&mut tree);
    assert!(
        tree.children(container)
            .expect("reversed identities")
            .iter()
            .eq(original.iter().rev())
    );
    let mut reversed = true;
    bencher
        .with_inputs(|| {
            reversed = !reversed;
            if reversed {
                backward.clone()
            } else {
                forward.clone()
            }
        })
        .bench_local_values(|order| {
            rows.set(divan::black_box(order)).expect("reorder rows");
            apply_bindings(&mut tree);
        });
    assert_eq!(mounts.get(), 32);
    let expected: Vec<_> = rows
        .get()
        .expect("final order")
        .into_iter()
        .map(|key| original[usize::from(key)])
        .collect();
    assert_eq!(
        tree.children(container).expect("retained identities"),
        expected
    );
}
