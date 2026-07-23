//! Complex native operations executed transactionally against VM state.
//!
//! These helpers are kept out of the bytecode dispatch loop so array and
//! character mutation rules can be reviewed as one cohesive subsystem.

use super::{
    BytecodeStorage, BytecodeType, Fiber, NativePlaceView, NativeReady, PlaceDescriptor, Vm,
    VmError, VmValue, array_snapshot_any_rank, character_series, global_unindexed_place,
    indexed_place,
};

pub(super) fn execute_swap_transaction(
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
        return Err(VmError::InvalidArguments(
            "SWAP places have different value types".into(),
        ));
    }
    // Both targets are fully resolved before the first write. Since EraBasic is
    // single-owner here, the validated writes form one observable transaction.
    vm.write_place(fiber, left, right_value)?;
    vm.write_place(fiber, right, left_value)
}

pub(super) fn native_place_views(
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

pub(super) fn native_implicit_place_views(
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

pub(super) fn native_place_view(
    vm: &Vm,
    fiber: &Fiber,
    argument_index: usize,
    target: &PlaceDescriptor,
) -> Result<NativePlaceView, VmError> {
    let values = if target.indices.is_empty() {
        array_snapshot_any_rank(vm, fiber, target)
            .or_else(|_| vm.read_place(fiber, target).map(|value| vec![value]))?
    } else {
        vec![vm.read_place(fiber, target)?]
    };
    Ok(NativePlaceView {
        argument_index,
        target: target.clone(),
        values,
    })
}

pub(super) fn validate_native_ready(
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

pub(super) fn array_place(arguments: &[VmValue]) -> Result<&PlaceDescriptor, VmError> {
    match arguments.first() {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => Ok(place),
        _ => Err(VmError::InvalidArguments(
            "array operation requires a variable reference".into(),
        )),
    }
}

pub(super) fn integer_argument(arguments: &[VmValue], index: usize) -> Result<i64, VmError> {
    match arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(VmError::InvalidArguments(format!(
            "argument {} must be integer",
            index + 1
        ))),
    }
}

pub(super) fn execute_bit_mutation(
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
            VmValue::Integer(bit) => Err(VmError::InvalidArguments(format!(
                "{} bit argument {} ({bit}) is outside 0..63",
                operation.to_ascii_uppercase(),
                index + 2
            ))),
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

pub(super) fn execute_split_transaction(
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
    let current = array_snapshot(vm, fiber, &output)?;
    let _ = vm.read_place(fiber, &count)?;
    let parts = if separator.is_empty() {
        vec![target.as_str()]
    } else {
        target.split(separator).collect::<Vec<_>>()
    };
    let values = parts
        .iter()
        .take(current.len())
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

pub(super) fn execute_getnum(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let (_, _, _, value) = lookup_named_index(vm, fiber, arguments)?;
    Ok(VmValue::Integer(value.unwrap_or(-1)))
}

pub(super) fn execute_index_by_name(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let (variable, key, dimension, value) = lookup_named_index(vm, fiber, arguments)?;
    let value = value.ok_or_else(|| {
        VmError::InvalidArguments(format!(
            "{variable} has no named index {key:?} in dimension {dimension}"
        ))
    })?;
    Ok(VmValue::Integer(value))
}

pub(super) fn execute_set_var(
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
        return Err(VmError::InvalidArguments(format!(
            "SETVAR value type differs from {variable_name}"
        )));
    }
    // Resolve and type-check the complete destination before the mutation.
    let _ = vm.read_place(fiber, &target)?;
    vm.write_place(fiber, &target, value)?;
    Ok(VmValue::Integer(1))
}

pub(super) fn execute_get_var(
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
        return Err(VmError::InvalidArguments(format!(
            "{} target {variable_name} has the wrong value type",
            if string_result { "GETVARS" } else { "GETVAR" }
        )));
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
        return Err(VmError::InvalidArguments(
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
                VmError::InvalidArguments(format!(
                    "SETVAR target {variable_name:?} is not a variable"
                ))
            })?;
        if require_mutable && !definition.mutable {
            return Err(VmError::InvalidArguments(format!(
                "SETVAR target {variable_name:?} is read-only"
            )));
        }
        let table = name_table_kind(&definition.name).and_then(|kind| {
            program
                .artifact
                .project_data
                .static_data
                .name_tables
                .get(&kind)
        });
        let indices = components
            .map(|component| set_var_index(table, &definition.name, component))
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
        return Err(VmError::InvalidArguments(
            "SETVAR contains an empty variable index".into(),
        ));
    }
    if let Ok(index) = component.parse::<i64>() {
        return u64::try_from(index).map_err(|_| {
            VmError::InvalidArguments(format!("SETVAR variable index {component:?} is negative"))
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
            VmError::InvalidArguments(format!(
                "SETVAR variable {variable_name} has no named index {component:?}"
            ))
        })?;
    u64::try_from(index).map_err(|_| {
        VmError::InvalidArguments(format!(
            "SETVAR variable {variable_name} has a negative named index {component:?}"
        ))
    })
}

pub(super) fn execute_encode_to_uni_result(
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
        return Err(VmError::InvalidArguments(format!(
            "ENCODETOUNI input has {} UTF-16 units but RESULT can hold only {}",
            utf16.len(),
            capacity.saturating_sub(1)
        )));
    }
    let mut encoded = Vec::with_capacity(utf16.len());
    for (index, unit) in utf16.iter().copied().enumerate() {
        let code_point = if (0xd800..=0xdbff).contains(&unit) {
            let Some(low) = utf16.get(index + 1).copied() else {
                return Err(VmError::InvalidArguments(
                    "ENCODETOUNI input ends with an unpaired UTF-16 surrogate".into(),
                ));
            };
            if !(0xdc00..=0xdfff).contains(&low) {
                return Err(VmError::InvalidArguments(
                    "ENCODETOUNI input contains an unpaired UTF-16 surrogate".into(),
                ));
            }
            0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&unit) {
            // The pinned reference advances one UTF-16 position after converting
            // a surrogate pair, then rejects the low surrogate on the next pass.
            return Err(VmError::InvalidArguments(
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

fn lookup_named_index(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<(String, String, i64, Option<i64>), VmError> {
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
    let dimension = match arguments.get(2) {
        None => 0,
        Some(VmValue::Integer(value)) => *value,
        Some(_) => {
            return Err(VmError::InvalidArguments(
                "named index dimension is not an integer".into(),
            ));
        }
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
    let kind = name_table_kind(name);
    let value = kind
        .and_then(|kind| {
            program
                .artifact
                .project_data
                .static_data
                .name_tables
                .get(&kind)
        })
        .and_then(|table| table.lookup.get(key))
        .map(|index| i64::from(*index));
    Ok((name.into(), key.clone(), dimension, value))
}

fn name_table_kind(name: &str) -> Option<erabasic_data::NameTableKind> {
    fn exact(name: &str) -> Option<erabasic_data::NameTableKind> {
        match name {
            "ABL" => Some(erabasic_data::NameTableKind::Abl),
            "EXP" => Some(erabasic_data::NameTableKind::Exp),
            "TALENT" => Some(erabasic_data::NameTableKind::Talent),
            "PALAM" => Some(erabasic_data::NameTableKind::Palam),
            "TRAIN" => Some(erabasic_data::NameTableKind::Train),
            "MARK" => Some(erabasic_data::NameTableKind::Mark),
            "ITEM" | "ITEMSALES" | "ITEMPRICE" | "ITEMNAME" => {
                Some(erabasic_data::NameTableKind::Item)
            }
            "BASE" | "MAXBASE" | "LOSEBASE" | "DOWNBASE" => {
                Some(erabasic_data::NameTableKind::Base)
            }
            "SOURCE" => Some(erabasic_data::NameTableKind::Source),
            "EX" => Some(erabasic_data::NameTableKind::Ex),
            "STR" | "STRNAME" => Some(erabasic_data::NameTableKind::Strname),
            "EQUIP" => Some(erabasic_data::NameTableKind::Equip),
            "TEQUIP" => Some(erabasic_data::NameTableKind::Tequip),
            "FLAG" => Some(erabasic_data::NameTableKind::Flag),
            "TFLAG" => Some(erabasic_data::NameTableKind::Tflag),
            "CFLAG" => Some(erabasic_data::NameTableKind::Cflag),
            "TCVAR" => Some(erabasic_data::NameTableKind::Tcvar),
            "CSTR" => Some(erabasic_data::NameTableKind::Cstr),
            "STAIN" => Some(erabasic_data::NameTableKind::Stain),
            "TSTR" => Some(erabasic_data::NameTableKind::Tstr),
            "SAVESTR" => Some(erabasic_data::NameTableKind::Savestr),
            "GLOBAL" => Some(erabasic_data::NameTableKind::Global),
            "GLOBALS" => Some(erabasic_data::NameTableKind::Globals),
            "DAY" => Some(erabasic_data::NameTableKind::Day),
            "TIME" => Some(erabasic_data::NameTableKind::Time),
            "MONEY" => Some(erabasic_data::NameTableKind::Money),
            _ => None,
        }
    }

    exact(name).or_else(|| {
        let normalized = name.to_ascii_uppercase();
        (normalized != name).then(|| exact(&normalized)).flatten()
    })
}

pub(super) fn execute_strjoin(
    vm: &Vm,
    fiber: &Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let values = array_snapshot_any_rank(vm, fiber, place)?;
    let delimiter = match arguments.get(1) {
        None => ",",
        Some(VmValue::String(value)) => value,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN delimiter must be a string".into(),
            ));
        }
    };
    let start = match arguments.get(2) {
        None => 0,
        Some(VmValue::Integer(value)) => usize::try_from(*value)
            .map_err(|_| VmError::InvalidArguments("STRJOIN start is negative".into()))?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN start must be an integer".into(),
            ));
        }
    };
    if start > values.len() {
        return Err(VmError::InvalidArguments(
            "STRJOIN start exceeds the array".into(),
        ));
    }
    let count = match arguments.get(3) {
        None => values.len() - start,
        Some(VmValue::Integer(value)) => usize::try_from(*value)
            .map_err(|_| VmError::InvalidArguments("STRJOIN count is negative".into()))?,
        _ => {
            return Err(VmError::InvalidArguments(
                "STRJOIN count must be an integer".into(),
            ));
        }
    };
    let end = start
        .checked_add(count)
        .filter(|end| *end <= values.len())
        .ok_or_else(|| VmError::InvalidArguments("STRJOIN range exceeds the array".into()))?;
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

pub(super) fn array_snapshot(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let program = vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("array generation is missing".into()))?;
    let definition = program
        .global(place.variable)
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if definition.dimensions.len() != 1 || !place.indices.is_empty() {
        return Err(VmError::InvalidArguments(
            "array operation requires an unindexed one-dimensional variable".into(),
        ));
    }
    vm.read_place_array(fiber, place)
}

pub(super) fn commit_array(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    vm.write_place_array(fiber, place, values)
}

pub(super) fn execute_array_mutation(
    vm: &mut Vm,
    fiber: &mut Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let place = array_place(arguments)?.clone();
    let mut values = array_snapshot(vm, fiber, &place)?;
    match operation {
        "arrayremove" => {
            let start = usize::try_from(integer_argument(arguments, 1)?)
                .map_err(|_| VmError::InvalidArguments("ARRAYREMOVE start is negative".into()))?;
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
            let shift = integer_argument(arguments, 1)?;
            if shift == 0 {
                return Ok(());
            }
            let fill = arguments.get(2).cloned().ok_or_else(|| {
                VmError::InvalidArguments("ARRAYSHIFT fill value is missing".into())
            })?;
            if values
                .first()
                .is_some_and(|value| value.value_type() != fill.value_type())
            {
                return Err(VmError::InvalidArguments(
                    "ARRAYSHIFT fill type differs".into(),
                ));
            }
            let start = match integer_argument(arguments, 3).unwrap_or(0) {
                i64::MIN => 0,
                value => usize::try_from(value).map_err(|_| {
                    VmError::InvalidArguments("ARRAYSHIFT start is negative".into())
                })?,
            };
            if start > values.len() {
                return Err(VmError::InvalidArguments(
                    "ARRAYSHIFT start exceeds array".into(),
                ));
            }
            let count = match integer_argument(arguments, 4).unwrap_or(i64::MIN) {
                i64::MIN => values.len() - start,
                value => usize::try_from(value).map_err(|_| {
                    VmError::InvalidArguments("ARRAYSHIFT count is negative".into())
                })?,
            };
            let end = start.saturating_add(count).min(values.len());
            let source = values[start..end].to_vec();
            for (relative, value) in values[start..end].iter_mut().enumerate() {
                let source_index = i64::try_from(relative).unwrap_or(i64::MAX) - shift;
                *value = usize::try_from(source_index)
                    .ok()
                    .and_then(|source_index| source.get(source_index).cloned())
                    .unwrap_or_else(|| fill.clone());
            }
        }
        "arraysort" => {
            let descending = arguments.get(1).is_some_and(|value| {
                matches!(value, VmValue::String(value) if value.eq_ignore_ascii_case("BACK"))
                    || matches!(value, VmValue::Integer(value) if *value < 0)
            });
            let start = match integer_argument(arguments, 2).unwrap_or(0) {
                i64::MIN => 0,
                value => usize::try_from(value)
                    .map_err(|_| VmError::InvalidArguments("ARRAYSORT start is negative".into()))?,
            };
            let count = match integer_argument(arguments, 3).unwrap_or(i64::MIN) {
                i64::MIN => values.len().saturating_sub(start),
                value => usize::try_from(value)
                    .map_err(|_| VmError::InvalidArguments("ARRAYSORT count is negative".into()))?,
            };
            let end = start.saturating_add(count).min(values.len());
            if start > end {
                return Err(VmError::InvalidArguments(
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
pub(super) fn execute_variable_fill(
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
        return Err(VmError::InvalidArguments(
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
        let value = arguments.get(1).cloned().unwrap_or(default);
        if value.value_type() != definition.value_type {
            return Err(VmError::InvalidArguments(
                "VARSET value type differs".into(),
            ));
        }
        if definition.dimensions.len() != 1 || !place.indices.is_empty() {
            let _ = vm.read_place(fiber, &place)?;
            return vm.write_place(fiber, &place, value);
        }
        let length = vm.place_array_len(fiber, &place)?;
        let mut start = optional_nonnegative(arguments, 2, 0, "VARSET start")?;
        let mut end = optional_nonnegative(arguments, 3, length, "VARSET end")?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        if end > length {
            return Err(VmError::InvalidArguments("VARSET range is invalid".into()));
        }
        return vm.fill_place_array_range(fiber, &place, start, end, value);
    }

    if definition.storage != BytecodeStorage::Character || definition.dimensions.len() > 1 {
        return Err(VmError::InvalidArguments(
            "CVARSET requires a scalar or one-dimensional character variable".into(),
        ));
    }
    let element = optional_nonnegative(arguments, 1, 0, "CVARSET element")?;
    let value = arguments.get(2).cloned().unwrap_or(default);
    if value.value_type() != definition.value_type {
        return Err(VmError::InvalidArguments(
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
        return Err(VmError::InvalidArguments("CVARSET range is invalid".into()));
    }
    let indices = if definition.dimensions.is_empty() {
        Vec::new()
    } else {
        if element >= usize::try_from(definition.dimensions[0]).unwrap_or(0) {
            return Err(VmError::InvalidArguments(
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

pub(super) fn optional_nonnegative(
    arguments: &[VmValue],
    index: usize,
    default: usize,
    label: &str,
) -> Result<usize, VmError> {
    match integer_argument(arguments, index) {
        Err(_) | Ok(i64::MIN) => Ok(default),
        Ok(value) => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments(format!("{label} is negative"))),
    }
}

pub(super) fn execute_find_element(
    vm: &mut Vm,
    fiber: &Fiber,
    last: bool,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let place = array_place(arguments)?;
    let values = array_snapshot(vm, fiber, place)?;
    let needle = arguments
        .get(1)
        .ok_or_else(|| VmError::InvalidArguments("FINDELEMENT target is missing".into()))?;
    let start = match integer_argument(arguments, 2).unwrap_or(0) {
        i64::MIN => 0,
        value => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments("FINDELEMENT start is negative".into()))?,
    };
    let end = match integer_argument(arguments, 3).unwrap_or(i64::MIN) {
        i64::MIN => values.len(),
        value => usize::try_from(value)
            .map_err(|_| VmError::InvalidArguments("FINDELEMENT end is negative".into()))?,
    };
    if start > end || end > values.len() {
        return Err(VmError::InvalidArguments(
            "FINDELEMENT range is invalid".into(),
        ));
    }
    let exact = !matches!(integer_argument(arguments, 4), Ok(0) | Err(_));
    // FINDELEMENT treats the string needle as one regular expression for the
    // whole query. Compile it lazily so an empty range keeps its historical
    // no-op behavior, but do not rebuild the same automaton for every element.
    let mut compiled_regex = None;
    let mut matched = |value: &VmValue| -> Result<bool, VmError> {
        match (value, needle) {
            (VmValue::Integer(value), VmValue::Integer(needle)) => Ok(value == needle),
            (VmValue::String(value), VmValue::String(needle)) => {
                match crate::regex_compat::find_repeated_character(needle, value) {
                    crate::regex_compat::RepeatedCharacterMatch::Unsupported => {}
                    crate::regex_compat::RepeatedCharacterMatch::NoMatch => return Ok(false),
                    crate::regex_compat::RepeatedCharacterMatch::Match(matched) => {
                        return Ok(!exact || matched.len() == value.len());
                    }
                }
                if compiled_regex.is_none() {
                    compiled_regex = Some(
                        vm.compile_regex(needle)
                            .map_err(VmError::InvalidArguments)?,
                    );
                }
                let regex = compiled_regex
                    .as_ref()
                    .expect("the regex was initialized immediately above");
                Ok(regex
                    .find(value)
                    .is_some_and(|matched| !exact || matched.as_str().len() == value.len()))
            }
            _ => Err(VmError::InvalidArguments("FINDELEMENT types differ".into())),
        }
    };
    let range: Box<dyn Iterator<Item = usize>> = if last {
        Box::new((start..end).rev())
    } else {
        Box::new(start..end)
    };
    for index in range {
        if matched(&values[index])? {
            return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
        }
    }
    Ok(VmValue::Integer(-1))
}

#[allow(clippy::too_many_lines)]
pub(super) fn execute_array_query(
    vm: &Vm,
    fiber: &Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if matches!(operation, "groupmatch" | "nosames" | "allsames") {
        let Some(first) = arguments.first() else {
            return Err(VmError::InvalidArguments(format!(
                "{operation} requires at least two arguments"
            )));
        };
        if arguments.len() < 2
            || arguments
                .iter()
                .any(|value| value.value_type() != first.value_type())
        {
            return Err(VmError::InvalidArguments(format!(
                "{operation} arguments must have one value type"
            )));
        }
        let value = match operation {
            "groupmatch" => i64::try_from(
                arguments[1..]
                    .iter()
                    .filter(|candidate| *candidate == first)
                    .count(),
            )
            .unwrap_or(i64::MAX),
            "nosames" => i64::from(
                arguments
                    .iter()
                    .enumerate()
                    .all(|(index, value)| !arguments[..index].contains(value)),
            ),
            "allsames" => i64::from(arguments[1..].iter().all(|value| value == first)),
            _ => unreachable!(),
        };
        return Ok(VmValue::Integer(value));
    }

    let place = array_place(arguments)?;
    // CMATCH ranges over one selected field across characters, just like the
    // CARRAY family. Its first place is therefore intentionally indexed.
    let character_range = operation == "cmatch" || operation.contains("carray");
    let values = if character_range {
        character_series(vm, fiber, place)?
    } else {
        array_snapshot(vm, fiber, place)?
    };
    let (start_argument, end_argument) = if matches!(operation, "match" | "cmatch") {
        (2, 3)
    } else if matches!(operation, "inrangearray" | "inrangecarray") {
        (3, 4)
    } else {
        (1, 2)
    };
    let start = optional_index(arguments, start_argument, 0, operation)?;
    let end = optional_index(arguments, end_argument, values.len(), operation)?;
    if start > end || end > values.len() {
        return Err(VmError::InvalidArguments(format!(
            "{operation} range is invalid"
        )));
    }
    let range = &values[start..end];
    let result = match operation {
        "sumarray" | "sumcarray" => range.iter().try_fold(0i64, |sum, value| match value {
            VmValue::Integer(value) => Ok(sum.wrapping_add(*value)),
            _ => Err(VmError::InvalidArguments(format!(
                "{operation} requires an integer array"
            ))),
        })?,
        "maxarray" | "maxcarray" | "minarray" | "mincarray" => {
            let values = range
                .iter()
                .map(|value| match value {
                    VmValue::Integer(value) => Ok(*value),
                    _ => Err(VmError::InvalidArguments(format!(
                        "{operation} requires an integer array"
                    ))),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = if operation.starts_with("max") {
                values.into_iter().max()
            } else {
                values.into_iter().min()
            };
            value.ok_or_else(|| VmError::InvalidArguments(format!("{operation} range is empty")))?
        }
        "match" | "cmatch" => {
            let needle = arguments.get(1).ok_or_else(|| {
                VmError::InvalidArguments(format!("{operation} target is missing"))
            })?;
            if range
                .iter()
                .any(|candidate| candidate.value_type() != needle.value_type())
            {
                return Err(VmError::InvalidArguments(format!(
                    "{operation} target type differs"
                )));
            }
            i64::try_from(
                range
                    .iter()
                    .filter(|candidate| *candidate == needle)
                    .count(),
            )
            .unwrap_or(i64::MAX)
        }
        "inrangearray" | "inrangecarray" => {
            let minimum = integer_argument(arguments, 1)?;
            let maximum = integer_argument(arguments, 2)?;
            i64::try_from(
                range
                    .iter()
                    .filter(|value| {
                        matches!(value, VmValue::Integer(value) if *value >= minimum && *value <= maximum)
                    })
                    .count(),
            )
            .unwrap_or(i64::MAX)
        }
        _ => return Err(VmError::InvalidArguments("unknown array query".into())),
    };
    Ok(VmValue::Integer(result))
}

pub(super) fn optional_index(
    arguments: &[VmValue],
    index: usize,
    default: usize,
    operation: &str,
) -> Result<usize, VmError> {
    match arguments.get(index) {
        None | Some(VmValue::Integer(i64::MIN)) => Ok(default),
        Some(VmValue::Integer(value)) => usize::try_from(*value).map_err(|_| {
            VmError::InvalidArguments(format!("{operation} range cannot be negative"))
        }),
        _ => Err(VmError::InvalidArguments(format!(
            "{operation} range must be integer"
        ))),
    }
}
