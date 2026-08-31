#[allow(clippy::wildcard_imports)]
use super::*;

pub(in super::super) fn execute_swap_transaction(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = |index: usize| match arguments.get(index) {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => Ok(place),
        _ => Err(VmError::InvalidArguments(
            "SWAP requires two mutable places".into(),
        )),
    };
    let left = place(0)?;
    let right = place(1)?;
    let left_value = vm.read_place(fiber, left)?;
    let right_value = vm.read_place(fiber, right)?;
    if left_value.value_type() != right_value.value_type() {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "SWAP places have different value types".into(),
        ));
    }
    // Both targets are fully resolved before the first write. Since EraBasic is
    // single-owner here, the validated writes form one observable transaction.
    vm.write_place(fiber, left, right_value)?;
    vm.write_place(fiber, right, left_value)
}

pub(in super::super) fn execute_integer_mutation(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let Some(VmValue::IntegerPlace(place)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "integer mutation requires a mutable integer place".into(),
        ));
    };
    let Some(VmValue::Integer(mode @ 0..=3)) = arguments.get(1) else {
        return Err(VmError::InvalidArguments(
            "integer mutation mode is invalid".into(),
        ));
    };
    let VmValue::Integer(previous) = vm.read_place(fiber, place)? else {
        return Err(VmError::InvalidArguments(
            "integer mutation target has a different type".into(),
        ));
    };
    let generation = fiber
        .frames
        .last()
        .ok_or_else(|| VmError::InvalidState("integer mutation frame is missing".into()))?
        .generation;
    let increment = mode % 2 == 0;
    let operation = if increment {
        erabasic_compat::IntegerOperation::Add
    } else {
        erabasic_compat::IntegerOperation::Subtract
    };
    let updated = vm
        .integer_arithmetic(generation, operation, previous, Some(1))
        .map_err(VmError::ScriptFailure)?;
    vm.write_place(fiber, place, VmValue::Integer(updated))?;
    Ok(VmValue::Integer(if *mode >= 2 {
        // Postfix reverses one step after the policy-selected update. This also
        // preserves the pinned snake result at a saturated endpoint.
        if increment {
            updated.wrapping_sub(1)
        } else {
            updated.wrapping_add(1)
        }
    } else {
        updated
    }))
}

pub(in super::super) fn native_place_views(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<Vec<NativePlaceView>, VmError> {
    arguments
        .iter()
        .enumerate()
        .filter_map(|(argument_index, argument)| match argument {
            VmValue::IntegerPlace(target) | VmValue::StringPlace(target) => {
                Some((argument_index, target))
            }
            VmValue::Integer(_) | VmValue::String(_) => None,
        })
        .map(|(argument_index, target)| native_place_view(vm, fiber, argument_index, target))
        .collect()
}

pub(in super::super) fn native_implicit_place_views(
    vm: &Vm,
    fiber: &Fiber,
    names: &[&str],
) -> Result<std::collections::BTreeMap<String, NativePlaceView>, VmError> {
    names
        .iter()
        .copied()
        .filter_map(|name| {
            global_unindexed_place(vm, fiber, name)
                .ok()
                .map(|place| (name, place))
        })
        .map(|(name, target)| {
            Ok((
                name.to_owned(),
                native_place_view(vm, fiber, usize::MAX, &target)?,
            ))
        })
        .collect()
}

pub(in super::super) fn native_place_view(
    vm: &Vm,
    fiber: &Fiber,
    argument_index: usize,
    target: &PlaceDescriptor,
) -> Result<NativePlaceView, VmError> {
    let (_, definition) = vm.place_definition(fiber, target)?;
    let values = if target.indices.is_empty() && !definition.dimensions.is_empty() {
        array_snapshot_any_rank(vm, fiber, target)?
    } else {
        vec![vm.read_place(fiber, target)?]
    };
    Ok(NativePlaceView {
        argument_index,
        target: target.clone(),
        values,
    })
}

pub(in super::super) fn validate_native_ready(
    vm: &Vm,
    fiber: &Fiber,
    expected: Option<BytecodeType>,
    ready: &NativeReady,
) -> Result<(), VmError> {
    if expected != ready.value.as_ref().map(VmValue::value_type) {
        return Err(VmError::InvalidArguments(
            "native result type differs from its import".into(),
        ));
    }
    for write in &ready.writes {
        if write.target.backing.is_some() {
            return Err(VmError::InvalidState(
                "Native cannot inject an array backing identity".into(),
            ));
        }
        if write.target.fiber.is_some_and(|owner| owner != fiber.id) {
            return Err(VmError::InvalidState(
                "native write belongs to another fiber".into(),
            ));
        }
        let current = vm.read_place(fiber, &write.target)?;
        if current.value_type() != write.value.value_type()
            || matches!(
                write.value,
                VmValue::IntegerPlace(_) | VmValue::StringPlace(_)
            )
        {
            return Err(VmError::InvalidArguments(
                "native write value type differs from its place".into(),
            ));
        }
    }
    Ok(())
}

pub(in super::super) fn array_place(arguments: &[VmValue]) -> Result<&PlaceDescriptor, VmError> {
    match arguments.first() {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => Ok(place),
        _ => Err(VmError::InvalidArguments(
            "array operation requires a variable reference".into(),
        )),
    }
}

pub(in super::super) fn integer_argument(
    arguments: &[VmValue],
    index: usize,
) -> Result<i64, VmError> {
    match arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(VmError::InvalidArguments(format!(
            "argument {} must be integer",
            index + 1
        ))),
    }
}

pub(in super::super) fn execute_bit_mutation(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = array_place(arguments)?.clone();
    let VmValue::Integer(mut value) = vm.read_place(fiber, &place)? else {
        return Err(VmError::InvalidArguments(
            "bit mutation requires an integer place".into(),
        ));
    };
    // Validate the complete argument list before the first observable write.
    let bits = arguments[1..]
        .iter()
        .enumerate()
        .map(|(index, argument)| match argument {
            VmValue::Integer(bit @ 0..=63) => Ok(u32::try_from(*bit).unwrap_or(63)),
            VmValue::Integer(bit) => Err(script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!(
                    "{} bit argument {} ({bit}) is outside 0..63",
                    operation.to_ascii_uppercase(),
                    index + 2
                ),
            )),
            _ => Err(VmError::InvalidArguments(format!(
                "{} bit argument {} is not an integer",
                operation.to_ascii_uppercase(),
                index + 2
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    for bit in bits {
        let mask = 1_i64.wrapping_shl(bit);
        match operation {
            "setbit" => value |= mask,
            "clearbit" => value &= !mask,
            "invertbit" => value ^= mask,
            _ => unreachable!("bit operation is classified before dispatch"),
        }
    }
    vm.write_place(fiber, &place, VmValue::Integer(value))
}

pub(in super::super) fn execute_split_transaction(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let Some(VmValue::String(target)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "SPLIT target is not a string".into(),
        ));
    };
    let Some(VmValue::String(separator)) = arguments.get(1) else {
        return Err(VmError::InvalidArguments(
            "SPLIT separator is not a string".into(),
        ));
    };
    let output = match arguments.get(2) {
        Some(VmValue::StringPlace(place)) => place.as_ref().clone(),
        _ => {
            return Err(VmError::InvalidArguments(
                "SPLIT output is not a string-array place".into(),
            ));
        }
    };
    let count = match arguments.get(3) {
        Some(VmValue::IntegerPlace(place)) => place.as_ref().clone(),
        None => global_unindexed_place(vm, fiber, "RESULT")?,
        _ => {
            return Err(VmError::InvalidArguments(
                "SPLIT count is not an integer place".into(),
            ));
        }
    };
    let capacity = array_len(vm, fiber, &output)?;
    let _ = vm.read_place(fiber, &count)?;
    let parts = if separator.is_empty() {
        vec![target.as_str()]
    } else {
        target.split(separator).collect::<Vec<_>>()
    };
    let values = parts
        .iter()
        .take(capacity)
        .map(|value| VmValue::String((*value).into()))
        .collect::<Vec<_>>();
    // All destinations and types have been read above, so neither write can
    // discover a new user-visible error after the first mutation.
    for (index, value) in values.into_iter().enumerate() {
        let mut element = output.clone();
        element.indices = vec![u64::try_from(index).unwrap_or(u64::MAX)];
        vm.write_place(fiber, &element, value)?;
    }
    vm.write_place(
        fiber,
        &count,
        VmValue::Integer(i64::try_from(parts.len()).unwrap_or(i64::MAX)),
    )
}

pub(in super::super) fn execute_getnum(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let (variable, key, program) = lookup_named_index_target(vm, fiber, arguments)?;
    let source_dimension = named_index_dimension(arguments)?;
    let data_dimension = if source_dimension > 0 {
        source_dimension - 1
    } else {
        source_dimension
    };
    let Ok(data_dimension) = usize::try_from(data_dimension) else {
        return Ok(VmValue::Integer(-1));
    };
    let value = resolve_builtin_index_value(program, &variable, &key, data_dimension);
    Ok(VmValue::Integer(value.unwrap_or(-1)))
}

pub(in super::super) fn execute_erdname(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "ERDNAME argument 1 is not a variable reference".into(),
        ));
    };
    let Some(VmValue::Integer(index)) = arguments.get(1) else {
        return Err(VmError::InvalidArguments(
            "ERDNAME argument 2 is not an integer".into(),
        ));
    };
    if *index < 0 {
        return Ok(VmValue::String(String::new()));
    }
    let selector = match arguments.get(2) {
        None | Some(VmValue::Integer(i64::MIN)) => None,
        Some(VmValue::Integer(value)) => Some(*value),
        Some(_) => {
            return Err(VmError::InvalidArguments(
                "ERDNAME argument 3 is not an integer".into(),
            ));
        }
    };
    let generation = fiber.frames.last().expect("frame exists").generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("ERDNAME generation is missing".into()))?;
    let variable = program.global(place.variable).ok_or_else(|| {
        VmError::InvalidArguments("ERDNAME variable is not project-visible".into())
    })?;
    let project_data = &program.artifact.project_data;
    let key = selector.map_or_else(
        || variable.name.clone(),
        |selector| format!("{}@{selector}", variable.name),
    );
    let value = project_data
        .static_data
        .deferred_indices
        .resolved
        .get(&key)
        .or_else(|| {
            project_data
                .static_data
                .deferred_indices
                .resolved
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&key))
                .map(|(_, table)| table)
        })
        .and_then(|table| {
            if program
                .artifact
                .manifest
                .compatibility
                .uses_snake_alias_rules()
            {
                table.canonical_names.get(index).cloned()
            } else {
                table
                    .entries
                    .iter()
                    .find_map(|(name, value)| (value == index).then(|| name.clone()))
            }
        })
        .unwrap_or_default();
    Ok(VmValue::String(value))
}

pub(in super::super) fn execute_index_by_name(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let (variable, key, program) = lookup_named_index_target(vm, fiber, arguments)?;
    let dimension = named_index_dimension(arguments)?;
    let value = usize::try_from(dimension).ok().and_then(|data_dimension| {
        resolve_named_index_value(program, &variable, &key, data_dimension)
    });
    let value = value.ok_or_else(|| {
        script_native_error(
            crate::ScriptFaultKind::Resolve,
            format!("{variable} has no named index {key:?} in dimension {dimension}"),
        )
    })?;
    Ok(VmValue::Integer(value))
}

pub(in super::super) fn execute_set_var(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let Some(VmValue::String(reference)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "SETVAR variable name must be a string".into(),
        ));
    };
    let value = arguments
        .get(1)
        .cloned()
        .ok_or_else(|| VmError::InvalidArguments("SETVAR value is missing".into()))?;
    let (target, value_type, variable_name) =
        resolve_dynamic_variable_target(vm, fiber, reference, true)?;
    if value.value_type() != value_type {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            format!("SETVAR value type differs from {variable_name}"),
        ));
    }
    // Resolve and type-check the complete destination before the mutation.
    let _ = vm.read_place(fiber, &target)?;
    vm.write_place(fiber, &target, value)?;
    Ok(VmValue::Integer(1))
}

pub(in super::super) fn execute_var_set_ex(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
    omitted_arguments: &[usize],
) -> Result<VmValue, VmError> {
    let Some(VmValue::String(reference)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "VARSETEX variable name must be a string".into(),
        ));
    };
    let value = arguments
        .get(1)
        .cloned()
        .ok_or_else(|| VmError::InvalidArguments("VARSETEX value is missing".into()))?;
    if matches!(value, VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) {
        return Err(VmError::InvalidArguments(
            "VARSETEX value must be a scalar".into(),
        ));
    }
    let (target, value_type, variable_name) =
        resolve_dynamic_variable_target(vm, fiber, reference, true)?;
    if value.value_type() != value_type {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            format!("VARSETEX value type differs from {variable_name}"),
        ));
    }
    let dimensions = vm.place_dimensions(fiber, &target)?;
    if dimensions.is_empty() {
        // The pinned implementation accepts a scalar target but performs no write.
        return Ok(VmValue::Integer(1));
    }
    if dimensions.len() > 3 {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "VARSETEX supports arrays of rank one through three".into(),
        ));
    }
    let (set_all_dimensions, last_length, indexed_start, end) =
        var_set_ex_selection(arguments, omitted_arguments, &target, &dimensions)?;
    if indexed_start >= end {
        return Ok(VmValue::Integer(1));
    }

    let mut array = target.clone();
    array.indices.clear();
    let length = vm.place_array_len(fiber, &array)?;
    // The pinned String implementation always preserves the selected prefix;
    // its all-dimensions branch only exists for numeric arrays.
    if set_all_dimensions && value_type == BytecodeType::Integer {
        let prefix_count = var_set_ex_prefix_count(&dimensions)?;
        for prefix in 0..prefix_count {
            let row = prefix.checked_mul(last_length).ok_or_else(|| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "VARSETEX offset exceeds this platform".into(),
                )
            })?;
            vm.fill_place_array_range(
                fiber,
                &array,
                row + indexed_start,
                row + end,
                value.clone(),
            )?;
        }
    } else {
        let row = var_set_ex_selected_row(&target, &dimensions, last_length, length, end)?;
        vm.fill_place_array_range(fiber, &array, row + indexed_start, row + end, value)?;
    }
    Ok(VmValue::Integer(1))
}

fn var_set_ex_selection(
    arguments: &[VmValue],
    omitted_arguments: &[usize],
    target: &PlaceDescriptor,
    dimensions: &[u64],
) -> Result<(bool, usize, usize, usize), VmError> {
    let argument = |index| {
        (!omitted_arguments.contains(&index))
            .then(|| arguments.get(index))
            .flatten()
    };
    let integer_argument = |index, default, label| match argument(index) {
        None => Ok(default),
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("VARSETEX {label} is negative"),
            )
        }),
        Some(_) => Err(VmError::InvalidArguments(format!(
            "VARSETEX {label} must be an integer"
        ))),
    };
    let set_all_dimensions = match argument(2) {
        None => true,
        Some(VmValue::Integer(value)) => *value != 0,
        Some(_) => {
            return Err(VmError::InvalidArguments(
                "VARSETEX dimension mode must be an integer".into(),
            ));
        }
    };
    let last_length = usize::try_from(*dimensions.last().unwrap_or(&0)).map_err(|_| {
        script_native_error(
            crate::ScriptFaultKind::Bounds,
            "VARSETEX final dimension exceeds this platform".into(),
        )
    })?;
    let start = integer_argument(3, 0, "start")?;
    let end = integer_argument(4, last_length, "end")?;
    if end > last_length {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "VARSETEX range exceeds the final dimension".into(),
        ));
    }
    let indexed_start = if dimensions.len() == 1 {
        start
    } else {
        start.max(
            target
                .indices
                .get(dimensions.len() - 1)
                .copied()
                .and_then(|index| usize::try_from(index).ok())
                .unwrap_or(0),
        )
    };
    Ok((set_all_dimensions, last_length, indexed_start, end))
}

fn var_set_ex_prefix_count(dimensions: &[u64]) -> Result<usize, VmError> {
    dimensions[..dimensions.len() - 1]
        .iter()
        .try_fold(1usize, |count, dimension| {
            usize::try_from(*dimension)
                .ok()
                .and_then(|dimension| count.checked_mul(dimension))
        })
        .ok_or_else(|| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "VARSETEX dimensions exceed this platform".into(),
            )
        })
}

fn var_set_ex_selected_row(
    target: &PlaceDescriptor,
    dimensions: &[u64],
    last_length: usize,
    array_length: usize,
    end: usize,
) -> Result<usize, VmError> {
    let mut prefix = 0usize;
    for (dimension, length) in dimensions[..dimensions.len() - 1].iter().enumerate() {
        let length = usize::try_from(*length).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "VARSETEX dimension exceeds this platform".into(),
            )
        })?;
        let index = target
            .indices
            .get(dimension)
            .copied()
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(0);
        if index >= length {
            return Err(script_native_error(
                crate::ScriptFaultKind::Bounds,
                "VARSETEX prefix index is out of range".into(),
            ));
        }
        prefix = prefix
            .checked_mul(length)
            .and_then(|prefix| prefix.checked_add(index))
            .ok_or_else(|| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "VARSETEX prefix exceeds this platform".into(),
                )
            })?;
    }
    let row = prefix.checked_mul(last_length).ok_or_else(|| {
        script_native_error(
            crate::ScriptFaultKind::Bounds,
            "VARSETEX offset exceeds this platform".into(),
        )
    })?;
    if row.checked_add(end).is_none_or(|end| end > array_length) {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            "VARSETEX range exceeds the variable".into(),
        ));
    }
    Ok(row)
}

pub(in super::super) fn execute_get_var(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
    string_result: bool,
) -> Result<VmValue, VmError> {
    let Some(VmValue::String(reference)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "GETVAR variable name must be a string".into(),
        ));
    };
    let (target, value_type, variable_name) =
        resolve_dynamic_variable_target(vm, fiber, reference, false)?;
    let expected = if string_result {
        BytecodeType::String
    } else {
        BytecodeType::Integer
    };
    if value_type != expected {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            format!(
                "{} target {variable_name} has the wrong value type",
                if string_result { "GETVARS" } else { "GETVAR" }
            ),
        ));
    }
    vm.read_place(fiber, &target)
}

fn resolve_dynamic_variable_target(
    vm: &Vm,
    fiber: &Fiber,
    reference: &str,
    require_mutable: bool,
) -> Result<(PlaceDescriptor, BytecodeType, String), VmError> {
    let frame = fiber.frames.last().expect("frame exists");
    let generation = frame.generation;
    let function = frame.function;
    let frame_id = frame.id;
    let fiber_id = fiber.id;
    let mut components = reference.split(':');
    let variable_name = components.next().unwrap_or_default().trim();
    if variable_name.is_empty() {
        return Err(script_native_error(
            crate::ScriptFaultKind::Parse,
            "SETVAR variable name is empty".into(),
        ));
    }
    let (definition, mut indices) = {
        let program = vm
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("SETVAR generation is missing".into()))?;
        let globals = &program.artifact.globals;
        let definition = globals
            .iter()
            .find(|definition| {
                definition.owner == Some(function)
                    && definition.name.eq_ignore_ascii_case(variable_name)
            })
            .or_else(|| {
                globals.iter().find(|definition| {
                    definition.owner.is_none()
                        && definition.name.eq_ignore_ascii_case(variable_name)
                })
            })
            .cloned()
            .ok_or_else(|| {
                script_native_error(
                    crate::ScriptFaultKind::Resolve,
                    format!("SETVAR target {variable_name:?} is not a variable"),
                )
            })?;
        if require_mutable && !definition.mutable {
            return Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                format!("SETVAR target {variable_name:?} is read-only"),
            ));
        }
        let components = components.collect::<Vec<_>>();
        let explicit_character = definition.storage == BytecodeStorage::Character
            && components.len() > definition.dimensions.len();
        let indices = components
            .into_iter()
            .enumerate()
            .map(|(position, component)| {
                let table = if explicit_character && position == 0 {
                    None
                } else {
                    let data_dimension = position.saturating_sub(usize::from(explicit_character));
                    erabasic_data::NameTableKind::for_data_variable(
                        &definition.name,
                        data_dimension,
                    )
                    .and_then(|kind| {
                        program
                            .artifact
                            .project_data
                            .static_data
                            .name_tables
                            .get(&kind)
                    })
                };
                set_var_index(table, &definition.name, component)
            })
            .collect::<Result<Vec<_>, _>>()?;
        (definition, indices)
    };
    let character = if definition.storage == BytecodeStorage::Character {
        if indices.len() > definition.dimensions.len() {
            Some(indices.remove(0))
        } else {
            Some(u64::try_from(vm.target_character_for_generation(generation)).unwrap_or(u64::MAX))
        }
    } else {
        None
    };
    let target = PlaceDescriptor {
        backing: None,
        variable: definition.key,
        indices,
        character,
        fiber: Some(fiber_id),
        frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(frame_id),
    };
    Ok((target, definition.value_type, definition.name))
}

fn set_var_index(
    table: Option<&erabasic_data::NameTable>,
    variable_name: &str,
    component: &str,
) -> Result<u64, VmError> {
    let component = component.trim();
    if component.is_empty() {
        return Err(script_native_error(
            crate::ScriptFaultKind::Parse,
            "SETVAR contains an empty variable index".into(),
        ));
    }
    if let Ok(index) = component.parse::<i64>() {
        return u64::try_from(index).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("SETVAR variable index {component:?} is negative"),
            )
        });
    }
    let index = table
        .and_then(|table| {
            table.lookup.get(component).or_else(|| {
                table
                    .lookup
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(component))
                    .map(|(_, index)| index)
            })
        })
        .copied()
        .ok_or_else(|| {
            script_native_error(
                crate::ScriptFaultKind::Resolve,
                format!("SETVAR variable {variable_name} has no named index {component:?}"),
            )
        })?;
    u64::try_from(index).map_err(|_| {
        script_native_error(
            crate::ScriptFaultKind::Bounds,
            format!("SETVAR variable {variable_name} has a negative named index {component:?}"),
        )
    })
}

pub(in super::super) fn execute_encode_to_uni_result(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let Some(VmValue::String(value)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "ENCODETOUNI statement requires a string".into(),
        ));
    };
    let result = global_unindexed_place(vm, fiber, "RESULT")?;
    let capacity = vm.place_array_len(fiber, &result)?;
    let utf16 = value.encode_utf16().collect::<Vec<_>>();
    if utf16.len() >= capacity {
        return Err(script_native_error(
            crate::ScriptFaultKind::Bounds,
            format!(
                "ENCODETOUNI input has {} UTF-16 units but RESULT can hold only {}",
                utf16.len(),
                capacity.saturating_sub(1)
            ),
        ));
    }
    let mut encoded = Vec::with_capacity(utf16.len());
    for (index, unit) in utf16.iter().copied().enumerate() {
        let code_point = if (0xd800..=0xdbff).contains(&unit) {
            let Some(low) = utf16.get(index + 1).copied() else {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Operation,
                    "ENCODETOUNI input ends with an unpaired UTF-16 surrogate".into(),
                ));
            };
            if !(0xdc00..=0xdfff).contains(&low) {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Operation,
                    "ENCODETOUNI input contains an unpaired UTF-16 surrogate".into(),
                ));
            }
            0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&unit) {
            // The pinned reference advances one UTF-16 position after converting
            // a surrogate pair, then rejects the low surrogate on the next pass.
            return Err(script_native_error(
                crate::ScriptFaultKind::Operation,
                "ENCODETOUNI input contains an isolated low UTF-16 surrogate".into(),
            ));
        } else {
            u32::from(unit)
        };
        encoded.push(VmValue::Integer(i64::from(code_point)));
    }
    let mut writes = Vec::with_capacity(encoded.len() + 1);
    writes.push((
        indexed_place(&result, 0),
        VmValue::Integer(i64::try_from(encoded.len()).unwrap_or(i64::MAX)),
    ));
    writes.extend(
        encoded
            .into_iter()
            .enumerate()
            .map(|(index, value)| (indexed_place(&result, index + 1), value)),
    );
    for (target, value) in &writes {
        let previous = vm.read_place(fiber, target)?;
        if previous.value_type() != value.value_type() {
            return Err(VmError::InvalidState(
                "ENCODETOUNI RESULT element has an unexpected type".into(),
            ));
        }
    }
    for (target, value) in writes {
        vm.write_place(fiber, &target, value)?;
    }
    Ok(())
}

fn named_index_dimension(arguments: &[VmValue]) -> Result<i64, VmError> {
    match arguments.get(2) {
        None => Ok(0),
        Some(VmValue::Integer(value)) => Ok(*value),
        Some(_) => Err(VmError::InvalidArguments(
            "named index dimension is not an integer".into(),
        )),
    }
}

fn lookup_named_index_target<'a>(
    vm: &'a Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<(String, String, &'a crate::state::ProgramGeneration), VmError> {
    let Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) = arguments.first() else {
        return Err(VmError::InvalidArguments(
            "named index argument 1 is not a variable reference".into(),
        ));
    };
    let Some(VmValue::String(key)) = arguments.get(1) else {
        return Err(VmError::InvalidArguments(
            "named index key is not a string".into(),
        ));
    };
    let generation = fiber.frames.last().expect("frame exists").generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("named index generation is missing".into()))?;
    let name = program
        .global(place.variable)
        .map(|definition| definition.name.as_str())
        .ok_or_else(|| {
            VmError::InvalidArguments("named index variable is not project-visible".into())
        })?;
    Ok((name.into(), key.clone(), program))
}

pub(in super::super) fn resolve_named_index_value(
    program: &crate::state::ProgramGeneration,
    variable: &str,
    key: &str,
    dimension: usize,
) -> Option<i64> {
    if program
        .artifact
        .manifest
        .compatibility
        .uses_snake_alias_rules()
    {
        let tables = &program
            .artifact
            .project_data
            .static_data
            .deferred_indices
            .resolved;
        let dimension_key = format!("{variable}@{}", dimension.checked_add(1)?);
        let table = tables.iter().find_map(|(name, table)| {
            (name.eq_ignore_ascii_case(&dimension_key)
                || (dimension == 0 && name.eq_ignore_ascii_case(variable)))
            .then_some(table)
        });
        if let Some(table) = table {
            return table.entries.get(key).copied();
        }
    }
    resolve_builtin_index_value(program, variable, key, dimension)
}

fn resolve_builtin_index_value(
    program: &crate::state::ProgramGeneration,
    variable: &str,
    key: &str,
    dimension: usize,
) -> Option<i64> {
    let kind = erabasic_data::NameTableKind::for_data_variable(variable, dimension)?;
    program
        .artifact
        .project_data
        .static_data
        .name_tables
        .get(&kind)?
        .lookup
        .get(key)
        .copied()
        .map(i64::from)
}
