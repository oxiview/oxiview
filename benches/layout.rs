//! Initial layout benchmarks over a fresh caller-owned retained topology.
#![allow(clippy::expect_used)]

use oxiview_base::Size;
use oxiview_layout::{
    Alignment, AvailableSpace, Dimensions, FlexDirection, LayoutBox, LayoutInput, LayoutTree,
    Length, TaffyLayout,
};

fn main() {
    divan::main();
}

struct Node {
    input: LayoutInput,
    children: Vec<usize>,
    layout: LayoutBox,
}

struct Tree(Vec<Node>);

impl Tree {
    fn new(nodes: usize) -> Self {
        let mut tree = Self(
            (0..nodes)
                .map(|id| Node {
                    input: LayoutInput {
                        width: Length::Points(if id == 0 { 800.0 } else { 80.0 }),
                        flex_direction: FlexDirection::Column,
                        align_items: Alignment::Start,
                        ..LayoutInput::default()
                    },
                    children: Vec::new(),
                    layout: LayoutBox::default(),
                })
                .collect(),
        );
        for id in 1..nodes {
            tree.0[(id - 1) / 10].children.push(id);
        }
        tree
    }
}

impl LayoutTree for Tree {
    type NodeId = usize;

    fn children(&self, node: usize) -> &[usize] {
        &self.0[node].children
    }

    fn layout_input(&self, node: usize) -> LayoutInput {
        self.0[node].input.clone()
    }

    fn set_layout(&mut self, node: usize, layout: LayoutBox) {
        self.0[node].layout = layout;
    }

    fn contains_node(&self, node: usize) -> bool {
        node < self.0.len()
    }

    fn measure(
        &mut self,
        _node: usize,
        known: Dimensions<Option<f64>>,
        _available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        Dimensions {
            width: known.width.unwrap_or(24.0),
            height: known.height.unwrap_or(12.0),
        }
    }
}

#[divan::bench(args = [100, 1000, 10000], sample_count = 40, sample_size = 1)]
fn initial_layout(bencher: divan::Bencher, nodes: usize) {
    let viewport = Size::new(800.0, 600.0);
    bencher
        .with_inputs(|| (TaffyLayout::new(), Tree::new(nodes)))
        .bench_local_refs(|(engine, tree)| {
            engine
                .compute(divan::black_box(tree), 0, viewport)
                .expect("initial layout")
        });
}
