#[allow(clippy::wildcard_imports)]
use super::*;

pub(in super::super) fn execute_strjoin(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
    omitted_arguments: &[usize],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let values = array_snapshot_any_rank(vm, fiber, place)?;
    // Omission is source provenance, never a particular Integer/String value.
    let argument = |slot| {
        (!omitted_arguments.contains(&slot))
            .then(|| arguments.get(slot))
            .flatten()
    };
    let delimiter = match argument(1) {
        None => ",",
        Some(VmValue::String(value)) => value,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN delimiter must be a string".into(),
            ));
        }
    };
    let start = match argument(2) {
        None => 0,
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "STRJOIN start is negative".into(),
            )
        })?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN start must be an integer".into(),
            ));
        }
    };
    if start > values.len() {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "STRJOIN start exceeds the array".into(),
        ));
    }
    let count = match argument(3) {
        None => values.len() - start,
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "STRJOIN count is negative".into(),
            )
        })?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN count must be an integer".into(),
            ));
        }
    };
    let end = start
        .checked_add(count)
        .filter(|end| *end <= values.len())
        .ok_or_else(|| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "STRJOIN range exceeds the array".into(),
            )
        })?;
    let joined = values[start..end]
        .iter()
        .map(|value| match value {
            VmValue::Integer(value) => Ok(value.to_string()),
            VmValue::String(value) => Ok(value.clone()),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => Err(VmError::InvalidState(
                "STRJOIN array contains a place".into(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(delimiter);
    Ok(VmValue::String(joined))
}

pub(in super::super) fn array_snapshot(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
    let array = one_dimensional_array_place(vm, fiber, place)?;
    let values = vm.read_place_array(fiber, &array)?;
    validate_array_storage_values(vm, fiber, &array, &values)?;
    Ok(values)
}

pub(in super::super) fn validate_array_storage_values(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
    values: &[VmValue],
) -> Result<(), VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let definition = vm
        .generations
        .get(&generation)
        .and_then(|program| program.global(place.variable))
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if values
        .iter()
        .any(|value| value.value_type() != definition.value_type)
    {
        return Err(VmError::InvalidState(
            "array storage type differs from its variable".into(),
        ));
    }
    Ok(())
}

pub(in super::super) fn array_len(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<usize, VmError> {
    let array = one_dimensional_array_place(vm, fiber, place)?;
    vm.place_array_len(fiber, &array)
}

fn one_dimensional_array_place(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<PlaceDescriptor, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("array generation is missing".into()))?;
    let definition = program
        .global(place.variable)
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if place.indices.len() > definition.dimensions.len() {
        return Err(VmError::InvalidArguments(
            "array operation requires a one-dimensional variable".into(),
        ));
    }
    if definition.dimensions.len() != 1 {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "array operation requires a one-dimensional variable".into(),
        ));
    }
    // Reference-taking one-dimensional operations accept both `ARRAY` and
    // `ARRAY:element`. For character data, the character selector has already
    // been split into `place.character`; discarding the remaining element
    // selector therefore preserves the selected character while exposing its
    // complete array. The operation range comes from the following arguments.
    let mut array = place.clone();
    array.indices.clear();
    Ok(array)
}

pub(in super::super) fn commit_array(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    let mut array = place.clone();
    array.indices.clear();
    vm.write_place_array(fiber, &array, values)
}

fn shift_array_values(values: &mut [VmValue], arguments: &[VmValue]) -> Result<bool, VmError> {
    let shift = integer_argument(arguments, 1)?;
    if shift == 0 {
        return Ok(false);
    }
    let fill = arguments
        .get(2)
        .cloned()
        .ok_or_else(|| VmError::InvalidArguments("ARRAYSHIFT fill value is missing".into()))?;
    if matches!(fill, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) {
        return Err(VmError::InvalidArguments(
            "ARRAYSHIFT fill type differs".into(),
        ));
    }
    if values
        .first()
        .is_some_and(|value| value.value_type() != fill.value_type())
    {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "ARRAYSHIFT fill type differs".into(),
        ));
    }
    let start = match optional_integer_argument(arguments, 3, 0)? {
        i64::MIN => 0,
        value => usize::try_from(value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "ARRAYSHIFT start is negative".into(),
            )
        })?,
    };
    if start > values.len() {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "ARRAYSHIFT start exceeds array".into(),
        ));
    }
    let count = match optional_integer_argument(arguments, 4, i64::MIN)? {
        i64::MIN => values.len() - start,
        value => usize::try_from(value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "ARRAYSHIFT count is negative".into(),
            )
        })?,
    };
    let end = start.saturating_add(count).min(values.len());
    let source = values[start..end].to_vec();
    for (relative, value) in values[start..end].iter_mut().enumerate() {
        let source_index = i64::try_from(relative)
            .ok()
            .and_then(|index| index.checked_sub(shift));
        *value = source_index
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|source_index| source.get(source_index).cloned())
            .unwrap_or_else(|| fill.clone());
    }
    Ok(true)
}

pub(in super::super) fn execute_array_mutation(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = array_place(arguments)?.clone();
    let mut values = array_snapshot(vm, fiber, &place)?;
    match operation {
        "arrayremove" => {
            let start = usize::try_from(integer_argument(arguments, 1)?).map_err(|_| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "ARRAYREMOVE start is negative".into(),
                )
            })?;
            let count = integer_argument(arguments, 2)?;
            if count <= 0 || start >= values.len() {
                return Ok(());
            }
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            let end = start.saturating_add(count).min(values.len());
            let removed = end - start;
            for source in end..values.len() {
                values[source - removed] = values[source].clone();
            }
            let default = VmValue::default_for(values[0].value_type());
            let fill_start = values.len() - removed;
            for value in &mut values[fill_start..] {
                *value = default.clone();
            }
        }
        "arrayshift" => {
            if !shift_array_values(&mut values, arguments)? {
                return Ok(());
            }
        }
        "arraysort" => {
            let descending = arguments.get(1).is_some_and(|value| {
                matches!(value, VmValue::String(value) if value.eq_ignore_ascii_case("BACK"))
                    || matches!(value, VmValue::Integer(value) if *value < 0)
            });
            let start = match optional_integer_argument(arguments, 2, 0)? {
                i64::MIN => 0,
                value => usize::try_from(value).map_err(|_| {
                    script_native_error(
                        crate::ScriptFaultKind::Bounds,
                        "ARRAYSORT start is negative".into(),
                    )
                })?,
            };
            let count = match optional_integer_argument(arguments, 3, i64::MIN)? {
                i64::MIN => values.len().saturating_sub(start),
                value => usize::try_from(value).map_err(|_| {
                    script_native_error(
                        crate::ScriptFaultKind::Bounds,
                        "ARRAYSORT count is negative".into(),
                    )
                })?,
            };
            let end = start.saturating_add(count).min(values.len());
            if start > end {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "ARRAYSORT range is invalid".into(),
                ));
            }
            values[start..end].sort_by(|left, right| match (left, right) {
                (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
                (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
                _ => std::cmp::Ordering::Equal,
            });
            if descending {
                values[start..end].reverse();
            }
        }
        _ => return Err(VmError::InvalidArguments("unknown array mutation".into())),
    }
    commit_array(vm, fiber, &place, values)
}

#[allow(clippy::too_many_lines)]
pub(in super::super) fn execute_variable_fill(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = match arguments.first() {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => place.clone(),
        _ => {
            return Err(VmError::InvalidArguments(format!(
                "{operation} destination must be a mutable variable place"
            )));
        }
    };
    let generation = fiber.frames.last().expect("frame exists").generation;
    let definition = vm
        .generations
        .get(&generation)
        .and_then(|generation| generation.global(place.variable))
        .cloned()
        .ok_or_else(|| VmError::InvalidState("VARSET variable is missing".into()))?;
    if !definition.mutable {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "VARSET destination is read-only".into(),
        ));
    }
    let default = VmValue::default_for(definition.value_type);
    if operation == "varset" {
        if definition.storage == BytecodeStorage::Character && place.character.is_none() {
            return Err(VmError::InvalidArguments(
                "VARSET character destination has no character".into(),
            ));
        }
        let value = fill_value_or_default(arguments, 1, default);
        if matches!(value, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) {
            return Err(VmError::InvalidArguments(
                "VARSET value type differs".into(),
            ));
        }
        if value.value_type() != definition.value_type {
            return Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                "VARSET value type differs".into(),
            ));
        }
        if definition.dimensions.is_empty() {
            let _ = vm.read_place(fiber, &place)?;
            return vm.write_place(fiber, &place, value);
        }
        let mut array = place;
        array.indices.clear();
        let length = vm.place_array_len(fiber, &array)?;
        let (start, end) = if definition.dimensions.len() == 1 {
            let mut start = optional_nonnegative(arguments, 2, 0, "VARSET start")?;
            let mut end = optional_nonnegative(arguments, 3, length, "VARSET end")?;
            if start > end {
                std::mem::swap(&mut start, &mut end);
            }
            if end > length {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "VARSET range is invalid".into(),
                ));
            }
            (start, end)
        } else {
            // The reference ignores range arguments for higher-rank arrays and
            // applies VARSET to their complete flattened storage.
            (0, length)
        };
        return vm.fill_place_array_range(fiber, &array, start, end, value);
    }

    if definition.storage != BytecodeStorage::Character || definition.dimensions.len() > 1 {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "CVARSET requires a scalar or one-dimensional character variable".into(),
        ));
    }
    let element = optional_nonnegative(arguments, 1, 0, "CVARSET element")?;
    let value = fill_value_or_default(arguments, 2, default);
    if matches!(value, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) {
        return Err(VmError::InvalidArguments(
            "CVARSET value type differs".into(),
        ));
    }
    if value.value_type() != definition.value_type {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "CVARSET value type differs".into(),
        ));
    }
    let character_count = vm.memory.characters.len();
    let mut start = optional_nonnegative(arguments, 3, 0, "CVARSET start")?;
    let mut end = optional_nonnegative(arguments, 4, character_count, "CVARSET end")?;
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    if end > character_count {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "CVARSET range is invalid".into(),
        ));
    }
    let indices = if definition.dimensions.is_empty() {
        Vec::new()
    } else {
        if element >= usize::try_from(definition.dimensions[0]).unwrap_or(0) {
            return Err(script_native_error(
                crate::ScriptFaultKind::Bounds,
                "CVARSET element is out of range".into(),
            ));
        }
        vec![u64::try_from(element).unwrap_or(u64::MAX)]
    };
    let destinations = (start..end)
        .map(|character| PlaceDescriptor {
            indices: indices.clone(),
            character: Some(u64::try_from(character).unwrap_or(u64::MAX)),
            ..place.as_ref().clone()
        })
        .collect::<Vec<_>>();
    for destination in &destinations {
        let previous = vm.read_place(fiber, destination)?;
        if previous.value_type() != value.value_type() {
            return Err(VmError::InvalidArguments(
                "CVARSET value type differs".into(),
            ));
        }
    }
    for destination in destinations {
        vm.write_place(fiber, &destination, value.clone())?;
    }
    Ok(())
}

fn fill_value_or_default(arguments: &[VmValue], index: usize, default: VmValue) -> VmValue {
    match arguments.get(index) {
        None | Some(VmValue::Integer(i64::MIN)) => default,
        // Translated projects commonly use numeric zero to clear a string array.
        // Treat it as the string default instead of materializing the text "0".
        Some(VmValue::Integer(0)) if default.value_type() == BytecodeType::String => default,
        Some(value) => value.clone(),
    }
}

pub(in super::super) fn optional_nonnegative(
    arguments: &[VmValue],
    index: usize,
    default: usize,
    label: &str,
) -> Result<usize, VmError> {
    match arguments.get(index) {
        None | Some(VmValue::Integer(i64::MIN)) => Ok(default),
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("{label} is negative"),
            )
        }),
        Some(_) => integer_argument(arguments, index).map(|_| default),
    }
}

pub(in super::super) fn optional_integer_argument(
    arguments: &[VmValue],
    index: usize,
    default: i64,
) -> Result<i64, VmError> {
    if arguments.get(index).is_none() {
        Ok(default)
    } else {
        integer_argument(arguments, index)
    }
}
