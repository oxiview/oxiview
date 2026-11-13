use super::*;

fn draw(value: u8) -> NodeKind {
    NodeKind::Draw(Primitive::Fill(
        Rect::new(0.0, 0.0, 10.0, 20.0),
        CornerRadii::uniform(2.0),
        Brush::Solid(Color::from_rgba8(value, 0, 0, 255)),
    ))
}
fn group() -> NodeKind {
    NodeKind::Transform(Affine::IDENTITY)
}

#[test]
fn cycles_indices_and_removal_are_atomic() -> Result<(), Error> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, group())?;
    let child = scene.insert(Some(root), 0, draw(1))?;
    let reference = scene.insert(None, 1, NodeKind::Reference(child, Affine::IDENTITY))?;
    let before = scene.clone();
    assert_eq!(scene.reparent(root, Some(child), 0), Err(Error::Cycle));
    assert_eq!(scene.reparent(reference, Some(child), 0), Err(Error::Cycle));
    assert_eq!(
        scene.replace(child, NodeKind::Reference(reference, Affine::IDENTITY)),
        Err(Error::Cycle)
    );
    assert_eq!(
        scene.insert(Some(child), 0, NodeKind::Reference(root, Affine::IDENTITY)),
        Err(Error::Cycle)
    );
    assert_eq!(scene.remove(root), Err(Error::SubtreeReferenced(child)));
    assert!(matches!(
        scene.reparent(child, None, 4),
        Err(Error::ChildIndex { .. })
    ));
    assert!(matches!(
        scene.insert(None, 4, group()),
        Err(Error::ChildIndex { .. })
    ));
    assert_eq!(scene, before);
    scene.remove(reference)?;
    scene.remove(root)?;
    assert_eq!(scene.content(child), Err(Error::MissingNode(child)));
    let new = scene.insert(None, 0, group())?;
    assert!(new > reference);
    assert_eq!(
        scene.insert(None, 1, NodeKind::Reference(child, Affine::IDENTITY)),
        Err(Error::MissingNode(child))
    );
    Ok(())
}

#[test]
fn validates_paint_and_opacity() -> Result<(), Error> {
    let mut scene = Scene::new();
    for opacity in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert_eq!(
            scene.insert(None, 0, NodeKind::Opacity(opacity)),
            Err(Error::InvalidOpacity)
        );
    }
    for positions in [
        vec![],
        vec![0.0],
        vec![0.0, f64::NAN],
        vec![-0.1, 1.0],
        vec![1.0, 0.0],
        vec![0.0, 1.1],
    ] {
        let brush = Brush::LinearGradient(
            Point::default(),
            Point::new(10.0, 0.0),
            positions
                .into_iter()
                .map(|position| ColorStop {
                    position,
                    color: Color::from_rgba8(0, 0, 0, 255),
                })
                .collect(),
        );
        assert_eq!(
            scene.insert(
                None,
                0,
                NodeKind::Draw(Primitive::Fill(
                    Rect::default(),
                    CornerRadii::default(),
                    brush
                ))
            ),
            Err(Error::InvalidGradient)
        );
    }
    let root = scene.insert(None, 0, NodeKind::Opacity(0.0))?;
    scene.replace(root, NodeKind::Opacity(1.0))?;
    let brush = Brush::LinearGradient(
        Point::default(),
        Point::new(10.0, 0.0),
        [0.0, 0.5, 0.5, 1.0]
            .map(|position| ColorStop {
                position,
                color: Color::from_rgba8(255, 0, 0, 255),
            })
            .to_vec(),
    );
    scene.insert(
        Some(root),
        0,
        NodeKind::Draw(Primitive::Stroke(
            Rect::default(),
            CornerRadii::default(),
            2.0,
            brush,
        )),
    )?;
    assert_eq!(scene.encode(root)?.nodes.len(), 3);
    Ok(())
}

#[test]
fn instances_resolve_transforms_clips_and_group_boundaries() -> Result<(), Error> {
    let mut scene = Scene::new();
    let scale = Affine::new([2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    let shift = Affine::new([1.0, 0.0, 0.0, 1.0, 7.0, 11.0]);
    let definition = scene.insert(None, 0, NodeKind::Transform(scale))?;
    let clip = Clip::RoundedRect(Rect::new(0.0, 0.0, 5.0, 5.0), CornerRadii::uniform(1.0));
    let clipped = scene.insert(Some(definition), 0, NodeKind::Clip(clip.clone()))?;
    let opacity = scene.insert(Some(clipped), 0, NodeKind::Opacity(0.5))?;
    scene.insert(Some(opacity), 0, draw(1))?;
    scene.insert(Some(opacity), 1, draw(2))?;
    let outer_clip = Clip::Rect(Rect::new(-10.0, -10.0, 100.0, 100.0));
    let root = scene.insert(None, 1, NodeKind::Clip(outer_clip.clone()))?;
    scene.insert(Some(root), 0, NodeKind::Reference(definition, shift))?;
    scene.insert(
        Some(root),
        1,
        NodeKind::Reference(definition, Affine::IDENTITY),
    )?;
    let encoded = scene.encode(root)?;
    let mut expected = Vec::new();
    for transform in [scale.then(shift), scale] {
        expected.push(EncodedNode::BeginOpacity(0.5));
        for value in [1, 2] {
            if let NodeKind::Draw(primitive) = draw(value) {
                expected.push(EncodedNode::Draw {
                    primitive,
                    transform,
                    clips: vec![
                        ResolvedClip {
                            shape: outer_clip.clone(),
                            transform: Affine::IDENTITY,
                        },
                        ResolvedClip {
                            shape: clip.clone(),
                            transform,
                        },
                    ],
                });
            }
        }
        expected.push(EncodedNode::EndOpacity);
    }
    assert_eq!(encoded.nodes, expected);
    assert_eq!(
        scale.then(shift).transform_point(Point::new(1.0, 1.0)),
        Point::new(9.0, 14.0)
    );
    Ok(())
}

#[test]
fn reorder_and_replace_preserve_identity_and_children() -> Result<(), Error> {
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, group())?;
    let a = scene.insert(Some(root), 0, draw(1))?;
    let b = scene.insert(Some(root), 1, draw(2))?;
    let child = scene.insert(Some(a), 0, draw(3))?;
    scene.replace(a, NodeKind::Opacity(0.5))?;
    assert_eq!(scene.children(Some(a))?, &[child]);
    scene.reparent(a, Some(root), 1)?;
    assert_eq!(scene.children(Some(root))?, &[b, a]);
    scene.remove(a)?;
    assert_eq!(scene.children(Some(root))?, &[b]);
    assert_eq!(scene.content(child), Err(Error::MissingNode(child)));
    Ok(())
}

#[test]
fn reference_updates_and_scope_restoration() -> Result<(), Error> {
    let mut scene = Scene::new();
    let definition = scene.insert(None, 0, draw(1))?;
    let root = scene.insert(None, 1, group())?;
    let scoped = scene.insert(Some(root), 0, NodeKind::Opacity(0.25))?;
    let transformed = scene.insert(
        Some(scoped),
        0,
        NodeKind::Transform(Affine::new([1.0, 0.0, 0.5, 1.0, 10.0, 0.0])),
    )?;
    let clipped = scene.insert(
        Some(transformed),
        0,
        NodeKind::Clip(Clip::Rect(Rect::default())),
    )?;
    scene.insert(
        Some(clipped),
        0,
        NodeKind::Reference(definition, Affine::IDENTITY),
    )?;
    scene.insert(
        Some(root),
        1,
        NodeKind::Reference(definition, Affine::IDENTITY),
    )?;
    scene.replace(definition, draw(7))?;
    let encoded = scene.encode(root)?;
    assert_eq!(encoded.nodes.len(), 4);
    assert_eq!(encoded.nodes[0], EncodedNode::BeginOpacity(0.25));
    assert_eq!(encoded.nodes[2], EncodedNode::EndOpacity);
    if let NodeKind::Draw(primitive) = draw(7) {
        assert_eq!(
            encoded.nodes[3],
            EncodedNode::Draw {
                primitive,
                transform: Affine::IDENTITY,
                clips: vec![]
            }
        );
    }
    // Internal references disappear with their ownership subtree and do not
    // prevent removing it. The external definition survives.
    scene.remove(root)?;
    assert_eq!(scene.roots(), &[definition]);
    Ok(())
}

#[test]
fn equality_preserves_reference_topology() -> Result<(), Error> {
    let mut first = Scene::new();
    let a = first.insert(None, 0, draw(1))?;
    let b = first.insert(None, 1, draw(1))?;
    let reference = first.insert(None, 2, NodeKind::Reference(a, Affine::IDENTITY))?;
    let mut second = first.clone();
    second.replace(reference, NodeKind::Reference(b, Affine::IDENTITY))?;
    assert_ne!(first, second);
    assert_eq!(first.encode(reference)?, second.encode(reference)?);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // A timing assertion must fail loudly on quadratic construction regressions.
fn builds_five_thousand_nodes_under_half_a_second() -> Result<(), Error> {
    let started = std::time::Instant::now();
    let mut scene = Scene::new();
    let root = scene.insert(None, 0, group())?;
    for index in 0..5_000 {
        scene.insert(Some(root), index, draw(1))?;
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "building 5,000 nodes took {elapsed:?}; expected less than 500 ms"
    );
    assert_eq!(scene.children(Some(root))?.len(), 5_000);
    Ok(())
}

#[test]
#[allow(clippy::panic)] // Image invariants at the shared rendering port.
fn rgba_image_validates_length_and_clears_transparent_rgb() -> Result<(), Error> {
    let size = PixelSize {
        width: 2,
        height: 1,
    };
    let image = RgbaImage::new(size, vec![1, 2, 3, 0, 4, 5, 6, 7])?;
    assert_eq!(image.pixels(), &[0, 0, 0, 0, 4, 5, 6, 7]);
    assert_eq!((image.width(), image.height()), (2, 1));
    assert!(RgbaImage::new(size, vec![0; 7]).is_err());
    assert!(
        RgbaImage::new(
            PixelSize {
                width: u32::MAX,
                height: u32::MAX
            },
            vec![]
        )
        .is_err()
    );
    Ok(())
}
