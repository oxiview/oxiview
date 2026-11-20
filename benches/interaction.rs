//! M5 window-free workloads. Construction and theme parsing precede each timed run.
#![allow(clippy::expect_used)] // A rejected workload invalidates the measurement.

use oxiview_base::Size;
use oxiview_core::{Action, FocusPolicy, KeyBinding, KeyChord, Keymap, UiTree, node};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, WindowEvent};
use oxiview_style::{AppearanceMode, Theme};
use std::{cell::Cell, rc::Rc, time::Duration};

fn main() {
    divan::main();
}

const VIEWPORT: Size = Size::new(640.0, 480.0);

fn key(code: KeyCode, pressed: bool) -> WindowEvent {
    WindowEvent::Key {
        code,
        logical: code.us_logical_key(),
        modifiers: KeyModifiers::default(),
        pressed,
        repeat: false,
        timestamp: Duration::ZERO,
    }
}
fn press(tree: &mut UiTree, events: &[WindowEvent; 2]) {
    for event in events {
        tree.dispatch(event, VIEWPORT, |_, _, _| {})
            .expect("dispatch benchmark input");
    }
}
#[divan::bench(sample_count = 80, sample_size = 16)]
fn tab_256(bencher: divan::Bencher) {
    let mut tree = UiTree::new().expect("tree");
    let root = tree.root();
    tree.mount(
        root,
        node().children((0..256).map(|_| node().focusable(FocusPolicy::Tab))),
    )
    .expect("mount Tab stops");
    tree.layout(VIEWPORT).expect("initial layout");
    let events = [key(KeyCode::Tab, true), key(KeyCode::Tab, false)];
    bencher.bench_local(|| {
        press(&mut tree, &events);
        divan::black_box(tree.focused_node());
    });
    assert!(tree.focused_node().is_some());
}
#[divan::bench(sample_count = 80, sample_size = 32)]
fn action_eight_contexts(bencher: divan::Bencher) {
    let mut tree = UiTree::new().expect("tree");
    let root = tree.root();
    let mut parent = root;
    let count = Rc::new(Cell::new(0u64));
    let hit = Rc::clone(&count);
    let map = |key_code| {
        Keymap::new([KeyBinding::new(
            "bench.select",
            [KeyChord::logical(
                LogicalKey::Named(key_code),
                KeyModifiers::default(),
            )],
        )])
        .expect("valid mapping")
    };
    let mut target = tree
        .mount(
            parent,
            node()
                .keymap(map(KeyCode::F6))
                .on_action(Action::new("bench.select", "Select"), move |_, _, _| {
                    hit.set(hit.get() + 1)
                }),
        )
        .expect("outer context");
    for _ in 1..8 {
        parent = target;
        target = tree
            .mount(
                parent,
                node().focusable(FocusPolicy::Tab).keymap(map(KeyCode::F7)),
            )
            .expect("nested context");
    }
    tree.focus(target).expect("focus leaf");
    tree.layout(VIEWPORT).expect("initial layout");
    let events = [key(KeyCode::F6, true), key(KeyCode::F6, false)];
    bencher.bench_local(|| {
        press(&mut tree, &events);
        divan::black_box(count.get());
    });
    assert!(count.get() > 0);
}
#[divan::bench(sample_count = 80, sample_size = 16)]
fn color_token_16_of_256(bencher: divan::Bencher) {
    let theme = Theme::from_toml(
        r##"
version = 1
[tokens.colors]
ink = "#112233"
fixed = "#778899"
[dark.colors]
ink = "#ccbbaa"
[rules.consumer]
background = "@ink"
[rules.stable]
background = "@fixed"
"##,
    )
    .expect("benchmark theme");
    let mut tree = UiTree::new().expect("tree");
    tree.set_theme(theme).expect("install theme");
    let root = tree.root();
    tree.mount(
        root,
        node().children(
            (0..256).map(|i| node().theme_type(if i < 16 { "consumer" } else { "stable" })),
        ),
    )
    .expect("mount themed rows");
    tree.layout(VIEWPORT).expect("initial layout");
    let mut dark = false;
    bencher.bench_local(|| {
        dark = !dark;
        let changed = tree
            .set_appearance_mode(if dark {
                AppearanceMode::Dark
            } else {
                AppearanceMode::Light
            })
            .expect("update palette");
        divan::black_box(changed);
    });
}
