use super::{ExtentError, FixedExtent, VariableExtent, VirtualExtent};

#[test]
fn fixed_range_has_half_open_edges_and_empty_intersections() -> Result<(), ExtentError> {
    let axis = FixedExtent::new(100_000, 12.5)?;
    assert_eq!(axis.visible_range(12.5, 25.0, 0), 1..3);
    assert_eq!(axis.visible_range(12.5, 25.5, 1), 0..5);
    assert_eq!(axis.visible_range(-5.0, 5.0, 80), 0..0);
    assert_eq!(axis.visible_range(0.0, 0.0, 1), 0..0);
    assert_eq!(axis.visible_range(f64::NAN, 20.0, 1), 0..0);
    assert_eq!(axis.visible_range(axis.total(), 20.0, 1), 100_000..100_000);
    assert_eq!(
        axis.visible_range(axis.total() - 4.0, 12.5, 1),
        99_998..100_000
    );
    assert_eq!(axis.reveal(99_999, 0.0, 11.0), axis.offset(99_999));
    let fractional = FixedExtent::new(100, 0.3)?;
    for index in 0..fractional.len() {
        assert_eq!(
            fractional
                .offset(index)
                .and_then(|edge| fractional.index_at(edge)),
            Some(index)
        );
    }
    Ok(())
}

#[test]
fn indexed_measurement_and_append_match_a_prefix_reference() -> Result<(), ExtentError> {
    let mut expected: Vec<_> = (0..103).map(|i| 9.0 + f64::from(i % 10) * 0.25).collect();
    let mut axis = VariableExtent::new(expected.iter().copied())?;
    for (index, new_size) in [(0, 14.0), (102, 2.5), (26, 112.25), (65, 9.75)] {
        assert_eq!(axis.set_size(index, new_size)?, expected[index] != new_size);
        expected[index] = new_size;
    }
    assert!(!axis.set_size(65, 9.75)?);
    axis.append(62, 3.5)?;
    expected.extend(std::iter::repeat_n(3.5, 62));
    axis.append(5, 11.25)?;
    expected.extend(std::iter::repeat_n(11.25, 5));
    let mut cursor = 0.0;
    for (index, size) in expected.iter().copied().enumerate() {
        assert_eq!(axis.offset(index), Some(cursor));
        assert_eq!(axis.index_at(cursor), Some(index));
        assert_eq!(axis.index_at(cursor + size / 2.0), Some(index));
        assert_eq!(axis.visible_range(cursor, size, 0), index..index + 1);
        cursor += size;
    }
    assert_eq!(axis.total(), cursor);
    assert_eq!(axis.index_at(cursor), None);
    Ok(())
}

#[test]
fn rejected_measurement_leaves_the_axis_usable() -> Result<(), ExtentError> {
    let mut axis = VariableExtent::new([4.0, 8.0])?;
    let maximum = f64::from(f32::MAX);
    assert_eq!(axis.set_size(2, 4.0), Err(ExtentError::InvalidIndex));
    assert_eq!(axis.set_size(0, 0.0), Err(ExtentError::InvalidSize));
    assert_eq!(axis.append(2, f64::INFINITY), Err(ExtentError::InvalidSize));
    assert_eq!(axis.append(2, maximum), Err(ExtentError::TooLarge));
    assert_eq!(
        VariableExtent::new([])?.append(usize::MAX, 1.0),
        Err(ExtentError::TooLarge)
    );
    assert_eq!(axis.total(), 12.0);
    axis.set_size(0, 1e30)?;
    axis.set_size(0, 4.0)?;
    assert_eq!(axis.total(), 12.0);
    Ok(())
}
