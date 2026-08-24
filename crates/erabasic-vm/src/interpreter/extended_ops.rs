//! Regex and higher-rank bulk array operations.

use super::{
    BytecodeStorage, BytecodeType, Fiber, NativeServiceRegistry, PlaceDescriptor, Vm, VmError,
    VmValue, array_place, array_snapshot, integer_argument,
};

pub(super) fn execute_regex_match(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let VmValue::String(input) = arguments
        .first()
        .ok_or_else(|| VmError::InvalidArguments("REGEXPMATCH input is missing".into()))?
    else {
        return Err(VmError::InvalidArguments(
            "REGEXPMATCH input must be a string".into(),
        ));
    };
    let VmValue::String(pattern) = arguments
        .get(1)
        .ok_or_else(|| VmError::InvalidArguments("REGEXPMATCH pattern is missing".into()))?
    else {
        return Err(VmError::InvalidArguments(
            "REGEXPMATCH pattern must be a string".into(),
        ));
    };
    let (captures_len, captures) = regex_captures(vm, pattern, input)?;
    let count = i64::try_from(captures.len()).unwrap_or(i64::MAX);
    match arguments.len() {
        2 => {}
        3 => {
            let VmValue::Integer(output) = arguments[2] else {
                return Err(VmError::InvalidArguments(
                    "REGEXPMATCH output flag must be an integer".into(),
                ));
            };
            if output != 0 {
                let result = global_unindexed_place(vm, fiber, "RESULT")?;
                let results = global_unindexed_place(vm, fiber, "RESULTS")?;
                let mut writes = vec![(
                    indexed_place(&result, 1),
                    VmValue::Integer(i64::try_from(captures_len).unwrap_or(i64::MAX)),
                )];
                if count > 0 {
                    writes.extend(
                        captures
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .map(|(index, value)| (indexed_place(&results, index), value)),
                    );
                }
                commit_place_writes(vm, fiber, writes)?;
            }
        }
        4 => {
            let group_count = match &arguments[2] {
                VmValue::IntegerPlace(place) => place.as_ref().clone(),
                _ => {
                    return Err(VmError::InvalidArguments(
                        "REGEXPMATCH group-count output must be an integer place".into(),
                    ));
                }
            };
            let values = match &arguments[3] {
                VmValue::StringPlace(place) => place.as_ref().clone(),
                _ => {
                    return Err(VmError::InvalidArguments(
                        "REGEXPMATCH capture output must be a string-array place".into(),
                    ));
                }
            };
            let mut writes = vec![(
                group_count,
                VmValue::Integer(i64::try_from(captures_len).unwrap_or(i64::MAX)),
            )];
            if count > 0 {
                writes.extend(
                    captures
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(index, value)| (indexed_place(&values, index), value)),
                );
            }
            commit_place_writes(vm, fiber, writes)?;
        }
        _ => {
            return Err(VmError::InvalidArguments(
                "REGEXPMATCH expects two, three, or four arguments".into(),
            ));
        }
    }
    Ok(VmValue::Integer(count))
}

fn regex_captures(
    vm: &mut Vm,
    pattern: &str,
    input: &str,
) -> Result<(usize, Vec<Vec<VmValue>>), VmError> {
    if let Some(captures) = crate::regex_compat::capture_positive_boundaries(pattern, input)
        .map_err(VmError::InvalidArguments)?
    {
        return Ok((
            captures.captures_len,
            captures
                .matches
                .into_iter()
                .map(|values| values.into_iter().map(VmValue::String).collect())
                .collect(),
        ));
    }
    let regex = vm
        .compile_regex(pattern)
        .map_err(VmError::InvalidArguments)?;
    let captures = regex
        .captures_iter(input)
        .map(|captures| {
            (0..regex.captures_len())
                .map(|index| {
                    VmValue::String(
                        captures
                            .get(index)
                            .map_or_else(String::new, |value| value.as_str().to_owned()),
                    )
                })
                .collect()
        })
        .collect();
    Ok((regex.captures_len(), captures))
}

pub(super) fn global_unindexed_place(
    vm: &Vm,
    fiber: &Fiber,
    name: &str,
) -> Result<PlaceDescriptor, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let variable = vm
        .generations
        .get(&generation)
        .and_then(|generation| generation.global_by_name(name))
        .ok_or_else(|| VmError::InvalidState(format!("{name} is not defined")))?;
    Ok(PlaceDescriptor {
        variable: variable.key,
        ..PlaceDescriptor::default()
    })
}

pub(super) fn indexed_place(place: &PlaceDescriptor, index: usize) -> PlaceDescriptor {
    let mut result = place.clone();
    result.indices = vec![u64::try_from(index).unwrap_or(u64::MAX)];
    result
}

pub(super) fn commit_place_writes(
    vm: &mut Vm,
    fiber: &mut Fiber,
    writes: Vec<(PlaceDescriptor, VmValue)>,
) -> Result<(), VmError> {
    // Read every destination before the first write so bounds and storage failures cannot
    // leave partially updated regex outputs.
    for (place, value) in &writes {
        let previous = vm.read_place(fiber, place)?;
        if previous.value_type() != value.value_type() {
            return Err(VmError::InvalidArguments(
                "REGEXPMATCH output type differs from its destination".into(),
            ));
        }
    }
    for (place, value) in writes {
        vm.write_place(fiber, &place, value)?;
    }
    Ok(())
}

pub(super) fn execute_array_copy(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let (source, source_type, source_dimensions) =
        array_copy_place(vm, fiber, arguments.first(), "source", false)?;
    let (destination, destination_type, destination_dimensions) =
        array_copy_place(vm, fiber, arguments.get(1), "destination", true)?;
    if source_type != destination_type {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY array types differ".into(),
        ));
    }
    if source_dimensions.len() != destination_dimensions.len() {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY dimensions differ".into(),
        ));
    }
    let source_values = array_snapshot_any_rank(vm, fiber, &source)?;
    let mut destination_values = array_snapshot_any_rank(vm, fiber, &destination)?;
    copy_shared_array_extent(
        &source_values,
        &source_dimensions,
        &mut destination_values,
        &destination_dimensions,
    )?;
    commit_array_any_rank(vm, fiber, &destination, destination_values)
}

fn copy_shared_array_extent(
    source: &[VmValue],
    source_dimensions: &[u64],
    destination: &mut [VmValue],
    destination_dimensions: &[u64],
) -> Result<(), VmError> {
    // Emuera accepts arrays of the same rank even when individual lengths differ. Each
    // dimension is truncated independently, while destination cells outside the shared
    // rectangular extent retain their previous values.
    for (destination_offset, destination_value) in destination.iter_mut().enumerate() {
        let coordinates = array_coordinates(destination_dimensions, destination_offset)?;
        if coordinates
            .iter()
            .zip(source_dimensions)
            .any(|(index, length)| index >= length)
        {
            continue;
        }
        let source_offset = array_offset(source_dimensions, &coordinates)?;
        let source_value = source.get(source_offset).ok_or_else(|| {
            VmError::InvalidState("ARRAYCOPY source storage has an invalid length".into())
        })?;
        *destination_value = source_value.clone();
    }
    Ok(())
}

fn array_coordinates(dimensions: &[u64], mut offset: usize) -> Result<Vec<u64>, VmError> {
    let mut coordinates = vec![0; dimensions.len()];
    for dimension in (0..dimensions.len()).rev() {
        let length = usize::try_from(dimensions[dimension]).map_err(|_| {
            VmError::InvalidState("ARRAYCOPY dimension exceeds this platform".into())
        })?;
        if length == 0 {
            return Err(VmError::InvalidState(
                "ARRAYCOPY array has a zero-length dimension".into(),
            ));
        }
        coordinates[dimension] = u64::try_from(offset % length).unwrap_or(u64::MAX);
        offset /= length;
    }
    Ok(coordinates)
}

fn array_offset(dimensions: &[u64], coordinates: &[u64]) -> Result<usize, VmError> {
    let offset = dimensions
        .iter()
        .zip(coordinates)
        .try_fold(0u64, |offset, (length, index)| {
            offset.checked_mul(*length)?.checked_add(*index)
        })
        .ok_or_else(|| VmError::InvalidState("ARRAYCOPY array offset overflow".into()))?;
    usize::try_from(offset)
        .map_err(|_| VmError::InvalidState("ARRAYCOPY array offset exceeds this platform".into()))
}

pub(super) fn execute_array_multi_sort(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if arguments.is_empty() {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORT requires at least one array".into(),
        ));
    }
    let mut arrays = Vec::with_capacity(arguments.len());
    for (index, argument) in arguments.iter().enumerate() {
        let place = match argument {
            VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => place.clone(),
            _ => {
                return Err(VmError::InvalidArguments(format!(
                    "ARRAYMSORT argument {} must be an array place",
                    index + 1
                )));
            }
        };
        let generation = fiber.frames.last().expect("frame exists").generation;
        let definition = vm
            .generations
            .get(&generation)
            .and_then(|generation| {
                generation
                    .artifact
                    .globals
                    .iter()
                    .find(|definition| definition.key == place.variable)
            })
            .ok_or_else(|| VmError::InvalidState("ARRAYMSORT variable is missing".into()))?;
        if definition.storage == BytecodeStorage::Character
            || !definition.mutable
            || place.character.is_some()
            || !place.indices.is_empty()
            || !(1..=3).contains(&definition.dimensions.len())
            || (index == 0 && definition.dimensions.len() != 1)
        {
            return Err(VmError::InvalidArguments(format!(
                "ARRAYMSORT argument {} is not a mutable non-character array of the required rank",
                index + 1
            )));
        }
        let dimensions = definition.dimensions.clone();
        let values = array_snapshot_any_rank(vm, fiber, &place)?;
        arrays.push((place, dimensions, values));
    }

    let key_values = &arrays[0].2;
    let key_count = key_values
        .iter()
        .position(|value| {
            matches!(value, VmValue::Integer(0))
                || matches!(value, VmValue::String(value) if value.is_empty())
        })
        .unwrap_or(key_values.len());
    let mut order: Vec<usize> = (0..key_count).collect();
    order.sort_by(
        |left, right| match (&key_values[*left], &key_values[*right]) {
            (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
            (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
            _ => std::cmp::Ordering::Equal,
        },
    );

    // Validate every first dimension and build every candidate before the first write.
    let mut candidates = Vec::with_capacity(arrays.len());
    for (place, dimensions, values) in arrays {
        let first = usize::try_from(dimensions[0])
            .map_err(|_| VmError::InvalidState("ARRAYMSORT dimension is too large".into()))?;
        if first < key_count {
            return Ok(VmValue::Integer(0));
        }
        let row_width = values.len().checked_div(first).ok_or_else(|| {
            VmError::InvalidState("ARRAYMSORT array has an invalid first dimension".into())
        })?;
        let mut candidate = values.clone();
        for (destination, source) in order.iter().copied().enumerate() {
            let destination_start = destination * row_width;
            let source_start = source * row_width;
            candidate[destination_start..destination_start + row_width]
                .clone_from_slice(&values[source_start..source_start + row_width]);
        }
        candidates.push((place, candidate));
    }
    for (place, candidate) in candidates {
        commit_array_any_rank(vm, fiber, &place, candidate)?;
    }
    Ok(VmValue::Integer(1))
}

pub(super) fn execute_array_multi_sort_ex(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if arguments.len() < 2 {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORTEX requires a key and variable-name array".into(),
        ));
    }
    let (key, _, key_dimensions) = array_copy_place(vm, fiber, arguments.first(), "key", false)?;
    if key_dimensions.len() != 1 {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORTEX key must be one-dimensional".into(),
        ));
    }
    let key_values = array_snapshot_any_rank(vm, fiber, &key)?;
    let names_place = array_place(&arguments[1..])?;
    let names = array_snapshot(vm, fiber, names_place)?;
    let ascending = !matches!(arguments.get(2), Some(VmValue::Integer(0)));
    let fixed = match integer_argument(arguments, 3) {
        Err(_) | Ok(i64::MIN) => None,
        Ok(0) => return Ok(VmValue::Integer(0)),
        Ok(value) if value > 0 => Some(usize::try_from(value).unwrap_or(usize::MAX)),
        Ok(_) => None,
    };
    if fixed.is_none()
        && key_values
            .iter()
            .any(|value| matches!(value, VmValue::String(value) if value.is_empty()))
    {
        return Ok(VmValue::Integer(0));
    }
    let key_count = fixed.map_or_else(
        || {
            key_values
                .iter()
                .position(|value| matches!(value, VmValue::Integer(0)))
                .unwrap_or(key_values.len())
        },
        |length| length.min(key_values.len()),
    );
    let mut order = (0..key_count).collect::<Vec<_>>();
    order.sort_by(
        |left, right| match (&key_values[*left], &key_values[*right]) {
            (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
            (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
            _ => std::cmp::Ordering::Equal,
        },
    );
    if !ascending {
        order.reverse();
    }
    let mut candidates = Vec::new();
    for name in names {
        let VmValue::String(name) = name else {
            return Err(VmError::InvalidArguments(
                "ARRAYMSORTEX variable-name array must contain strings".into(),
            ));
        };
        if name.is_empty() {
            break;
        }
        let (place, _, dimensions) =
            array_copy_place(vm, fiber, Some(&VmValue::String(name)), "target", true)?;
        let values = array_snapshot_any_rank(vm, fiber, &place)?;
        let first = usize::try_from(dimensions[0])
            .map_err(|_| VmError::InvalidState("ARRAYMSORTEX dimension is too large".into()))?;
        if first < key_count {
            return Ok(VmValue::Integer(0));
        }
        let row_width = values.len() / first;
        let mut candidate = values.clone();
        for (destination, source) in order.iter().copied().enumerate() {
            candidate[destination * row_width..(destination + 1) * row_width]
                .clone_from_slice(&values[source * row_width..(source + 1) * row_width]);
        }
        candidates.push((place, candidate));
    }
    for (place, candidate) in candidates {
        commit_array_any_rank(vm, fiber, &place, candidate)?;
    }
    Ok(VmValue::Integer(1))
}

pub(super) fn array_copy_place(
    vm: &Vm,
    fiber: &Fiber,
    value: Option<&VmValue>,
    role: &str,
    destination: bool,
) -> Result<(PlaceDescriptor, BytecodeType, Vec<u64>), VmError> {
    let frame = fiber.frames.last().expect("frame exists");
    let generation = frame.generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("ARRAYCOPY generation is missing".into()))?;
    let (place, value_type) = match value {
        Some(VmValue::IntegerPlace(place)) => (place.as_ref().clone(), BytecodeType::Integer),
        Some(VmValue::StringPlace(place)) => (place.as_ref().clone(), BytecodeType::String),
        Some(VmValue::String(name)) => {
            // Era variable-name strings are resolved in the active function scope. A
            // project can contain many same-named dynamic locals, so the generation-wide
            // name index can otherwise select a caller's or unrelated function's array.
            let definition = program
                .function_locals(frame.function)
                .chain(program.function_statics(frame.function))
                .find(|definition| definition.name.eq_ignore_ascii_case(name))
                .or_else(|| {
                    program.artifact.globals.iter().find(|definition| {
                        definition.owner.is_none() && definition.name.eq_ignore_ascii_case(name)
                    })
                })
                .ok_or_else(|| {
                    VmError::InvalidArguments(format!(
                        "ARRAYCOPY {role} variable {name:?} does not exist"
                    ))
                })?;
            (
                PlaceDescriptor {
                    variable: definition.key,
                    fiber: Some(fiber.id),
                    frame: (definition.storage == BytecodeStorage::FunctionLocal)
                        .then_some(frame.id),
                    ..PlaceDescriptor::default()
                },
                definition.value_type,
            )
        }
        _ => {
            return Err(VmError::InvalidArguments(format!(
                "ARRAYCOPY {role} must be an array place or variable-name string"
            )));
        }
    };
    let definition = program
        .global(place.variable)
        .ok_or_else(|| VmError::InvalidState("ARRAYCOPY variable is missing".into()))?;
    if definition.storage == BytecodeStorage::Character {
        return Err(VmError::InvalidArguments(format!(
            "ARRAYCOPY {role} cannot be a character variable"
        )));
    }
    if destination && !definition.mutable {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY destination is read-only".into(),
        ));
    }
    if !(1..=3).contains(&definition.dimensions.len()) || !place.indices.is_empty() {
        return Err(VmError::InvalidArguments(format!(
            "ARRAYCOPY {role} must be an unindexed one to three dimensional array"
        )));
    }
    Ok((place, value_type, definition.dimensions.clone()))
}

pub(super) fn array_snapshot_any_rank(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
    if !place.indices.is_empty() {
        return Err(VmError::InvalidArguments(
            "array place must be unindexed".into(),
        ));
    }
    let generation = fiber.frames.last().expect("frame exists").generation;
    let definition = vm
        .generations
        .get(&generation)
        .and_then(|generation| generation.global(place.variable))
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if !(1..=3).contains(&definition.dimensions.len()) {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY requires a one to three dimensional array".into(),
        ));
    }
    vm.read_place_array(fiber, place)
}

pub(super) fn commit_array_any_rank(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    vm.write_place_array(fiber, place, values)
}

pub(super) fn execute_random_place_transaction(
    memory: &mut crate::Memory,
    generation: crate::GenerationId,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    natives: &mut NativeServiceRegistry,
    operation: &str,
) -> Result<(), String> {
    let definition = artifact
        .globals
        .iter()
        .find(|definition| {
            definition.owner.is_none() && definition.name.eq_ignore_ascii_case("RANDDATA")
        })
        .ok_or_else(|| "RANDDATA is not defined".to_owned())?;
    if definition.storage != BytecodeStorage::Project
        || definition.value_type != BytecodeType::Integer
        || definition.dimensions != [625]
    {
        return Err("RANDDATA must be a mutable one-dimensional integer[625] variable".into());
    }
    if operation == "initrand" {
        let cell = memory
            .cell(generation, definition, 0)
            .ok_or_else(|| "RANDDATA storage is unavailable".to_owned())?;
        let values = cell
            .integers()
            .ok_or_else(|| "RANDDATA contains a non-integer value".to_owned())?;
        // Native state is only replaced after the entire array and index validate.
        natives.set_random_values(values)
    } else {
        let values = natives.random_values()?;
        let cell = memory
            .cell_mut(generation, definition.key, definition.storage, 0)
            .ok_or_else(|| "RANDDATA storage is unavailable".to_owned())?;
        let targets = cell
            .integers_mut()
            .ok_or_else(|| "RANDDATA contains a non-integer value".to_owned())?;
        if targets.len() != values.len() {
            return Err("RANDDATA storage changed during DUMPRAND".into());
        }
        // Every target slot was validated above, so this commit cannot partially fail.
        targets.copy_from_slice(&values);
        Ok(())
    }
}
