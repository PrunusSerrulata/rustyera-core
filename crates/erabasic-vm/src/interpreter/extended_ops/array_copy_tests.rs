use super::*;

// The pre-optimization scalar algorithm is the oracle for all rectangular extents.
fn scalar_copy(
    source: &[VmValue],
    source_dimensions: &[u64],
    destination: &mut [VmValue],
    destination_dimensions: &[u64],
) -> Result<(), VmError> {
    for (offset, destination) in destination.iter_mut().enumerate() {
        let mut coordinates = vec![0; destination_dimensions.len()];
        array_coordinates(destination_dimensions, offset, &mut coordinates)?;
        if coordinates
            .iter()
            .zip(source_dimensions)
            .any(|(index, length)| index >= length)
        {
            continue;
        }
        let source_offset = array_offset(source_dimensions, &coordinates)?;
        *destination = source
            .get(source_offset)
            .ok_or_else(|| {
                VmError::InvalidState("ARRAYCOPY source storage has an invalid length".into())
            })?
            .clone();
    }
    Ok(())
}

#[test]
fn bulk_arraycopy_matches_scalar_extents_and_preserves_unshared_cells() {
    let shapes = [
        vec![1],
        vec![3],
        vec![7],
        vec![1, 4],
        vec![2, 2],
        vec![3, 3],
        vec![1, 2, 3],
        vec![2, 1, 4],
        vec![3, 3, 2],
    ];
    for strings in [false, true] {
        for source_shape in &shapes {
            for destination_shape in shapes
                .iter()
                .filter(|shape| shape.len() == source_shape.len())
            {
                let length =
                    |shape: &[u64]| usize::try_from(shape.iter().product::<u64>()).unwrap();
                let value = |index: usize| {
                    if strings {
                        VmValue::String(format!("玄関🙂{index}"))
                    } else {
                        VmValue::Integer(i64::try_from(index).unwrap())
                    }
                };
                let source = (0..length(source_shape)).map(value).collect::<Vec<_>>();
                let mut actual = (100..100 + length(destination_shape))
                    .map(value)
                    .collect::<Vec<_>>();
                let mut expected = actual.clone();
                scalar_copy(&source, source_shape, &mut expected, destination_shape).unwrap();
                copy_shared_array_extent(&source, source_shape, &mut actual, destination_shape)
                    .unwrap();
                assert_eq!(
                    actual, expected,
                    "{source_shape:?} -> {destination_shape:?}"
                );
            }
        }
    }
}

#[test]
fn bulk_arraycopy_retains_scalar_errors_for_invalid_storage() {
    for (source_shape, destination_shape, source_length, destination_length) in [
        (vec![3], vec![3], 2, 3),
        (vec![3], vec![3], 3, 4),
        (vec![0], vec![0], 0, 1),
        (vec![0], vec![0], 0, 0),
        (vec![u64::MAX, 2], vec![u64::MAX, 2], 2, 2),
    ] {
        let source = vec![VmValue::Integer(7); source_length];
        let mut actual = vec![VmValue::Integer(9); destination_length];
        let mut expected = actual.clone();
        let previous = scalar_copy(&source, &source_shape, &mut expected, &destination_shape);
        let optimized =
            copy_shared_array_extent(&source, &source_shape, &mut actual, &destination_shape);
        assert_eq!(optimized, previous);
        assert_eq!(actual, expected);
    }
}
