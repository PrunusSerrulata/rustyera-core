#[allow(clippy::wildcard_imports)]
use super::*;
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

pub(in super::super) fn resolve_dynamic_variable_target(
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
                set_var_index(table, &definition.name, component, || {
                    dynamic_variable_index(vm, fiber, program, component)
                })
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

fn dynamic_variable_index(
    vm: &Vm,
    fiber: &Fiber,
    program: &crate::ProgramGeneration,
    component: &str,
) -> Result<Option<i64>, VmError> {
    let frame = fiber.frames.last().expect("frame exists");
    let name = component.trim();
    let definition = program
        .artifact
        .globals
        .iter()
        .find(|candidate| {
            candidate.owner == Some(frame.function) && candidate.name.eq_ignore_ascii_case(name)
        })
        .or_else(|| {
            program.artifact.globals.iter().find(|candidate| {
                candidate.owner.is_none() && candidate.name.eq_ignore_ascii_case(name)
            })
        });
    let Some(definition) = definition else {
        return Ok(None);
    };
    if definition.value_type != BytecodeType::Integer {
        return Ok(None);
    }
    let character = (definition.storage == BytecodeStorage::Character).then(|| {
        u64::try_from(vm.target_character_for_generation(frame.generation)).unwrap_or(u64::MAX)
    });
    let place = PlaceDescriptor {
        backing: None,
        variable: definition.key,
        indices: vec![0; definition.dimensions.len()],
        character,
        fiber: Some(fiber.id),
        frame: (definition.storage == BytecodeStorage::FunctionLocal).then_some(frame.id),
    };
    match vm.read_place(fiber, &place)? {
        VmValue::Integer(value) => Ok(Some(value)),
        _ => Ok(None),
    }
}

fn set_var_index(
    table: Option<&erabasic_data::NameTable>,
    variable_name: &str,
    component: &str,
    dynamic_index: impl FnOnce() -> Result<Option<i64>, VmError>,
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
    let named_index = table
        .and_then(|table| {
            table.lookup.get(component).or_else(|| {
                table
                    .lookup
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(component))
                    .map(|(_, index)| index)
            })
        })
        .copied();
    if let Some(index) = named_index {
        return u64::try_from(index).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("SETVAR variable {variable_name} has a negative named index {component:?}"),
            )
        });
    }
    if let Some(index) = dynamic_index()? {
        return u64::try_from(index).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                format!("SETVAR variable index expression {component:?} is negative"),
            )
        });
    }
    Err(script_native_error(
        crate::ScriptFaultKind::Resolve,
        format!("SETVAR variable {variable_name} has no named index {component:?}"),
    ))
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

pub(in super::super) fn named_index_dimension(arguments: &[VmValue]) -> Result<i64, VmError> {
    match arguments.get(2) {
        None => Ok(0),
        Some(VmValue::Integer(value)) => Ok(*value),
        Some(_) => Err(VmError::InvalidArguments(
            "named index dimension is not an integer".into(),
        )),
    }
}

pub(in super::super) fn lookup_named_index_target<'a>(
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

pub(in super::super) fn resolve_builtin_index_value(
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
