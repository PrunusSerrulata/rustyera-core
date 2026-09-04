//! Character-array queries, mutations, sorting, and selection operations.

use super::native_ops::{optional_integer_argument, script_native_error};
use super::{
    BytecodeStorage, Fiber, PlaceDescriptor, Vm, VmError, VmValue, array_place, integer_argument,
    optional_index,
};
use crate::{character_definition, shared_definition};

pub(super) fn character_series(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
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
        .ok_or_else(|| VmError::InvalidState("character array variable is missing".into()))?;
    if definition.storage != BytecodeStorage::Character {
        return Err(script_native_error(
            crate::ScriptFaultKind::Argument,
            "character-array query requires a character variable".into(),
        ));
    }
    (0..vm.memory.characters.len())
        .map(|character| {
            let mut element = place.clone();
            element.character = Some(u64::try_from(character).unwrap_or(u64::MAX));
            vm.read_place(fiber, &element)
        })
        .collect()
}

/// Query only the caller's artifact data; no current-session template or registry state is consulted.
pub(super) fn query_character_name(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    operation: &str,
    arguments: &[VmValue],
) -> Result<Option<VmValue>, VmError> {
    let names = &artifact.project_data.static_data.character_name_lookup;
    let lookup = match operation.to_ascii_lowercase().as_str() {
        "getcsvnobyname" => &names.names,
        "getcsvnobycallname" => &names.call_names,
        "getcsvnobynickname" => &names.nick_names,
        "getcsvnobymastername" => &names.master_names,
        _ => return Ok(None),
    };
    if !artifact.manifest.compatibility.supports_snake_data_apis() {
        return Err(VmError::InvalidState(
            "GETCSVNO is unavailable in this compatibility identity".into(),
        ));
    }
    let [VmValue::String(name)] = arguments else {
        return Err(VmError::InvalidArguments(
            "GETCSVNO name lookup expects exactly one string".into(),
        ));
    };
    Ok(Some(VmValue::Integer(
        lookup.get(name).copied().unwrap_or(-1),
    )))
}

#[allow(clippy::too_many_lines)]
pub(super) fn execute_character_query(
    vm: &Vm,
    fiber: &Fiber,
    operation: &str,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let artifact = &vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("character query generation is missing".into()))?
        .artifact;
    if let Some(value) = query_character_name(artifact, operation, arguments)? {
        return Ok(value);
    }
    if operation == "charanum" {
        return Ok(VmValue::Integer(
            i64::try_from(vm.memory.characters.len()).unwrap_or(i64::MAX),
        ));
    }
    if matches!(operation, "getchara" | "getspchara") {
        let number = integer_argument(arguments, 0)?;
        let requested_sp =
            operation == "getspchara" || optional_integer_argument(arguments, 1, 0)? != 0;
        let no = character_definition(artifact, "NO")
            .ok_or_else(|| VmError::InvalidState("NO is not defined".into()))?;
        let cflag = character_definition(artifact, "CFLAG");
        for (index, character) in vm.memory.characters.iter().enumerate() {
            let value = character.get(&no.key).and_then(crate::VariableCell::first);
            if value != Some(VmValue::Integer(number)) {
                continue;
            }
            if operation == "getchara" && arguments.get(1).is_none() {
                return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
            }
            let is_sp = cflag
                .and_then(|definition| character.get(&definition.key))
                .and_then(crate::VariableCell::first)
                .is_some_and(|value| matches!(value, VmValue::Integer(value) if value != 0));
            if is_sp == requested_sp {
                return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
            }
        }
        return Ok(VmValue::Integer(-1));
    }
    if matches!(operation, "findchara" | "findlastchara") {
        let place = array_place(arguments)?;
        let values = character_series(vm, fiber, place)?;
        let needle = arguments
            .get(1)
            .ok_or_else(|| VmError::InvalidArguments("FINDCHARA target is missing".into()))?;
        let start = optional_index(arguments, 2, 0, operation)?;
        let end = optional_index(arguments, 3, values.len(), operation)?;
        if start >= values.len() || start > end || end > values.len() {
            return Err(script_native_error(
                crate::ScriptFaultKind::Bounds,
                "FINDCHARA character range is invalid".into(),
            ));
        }
        let indices: Box<dyn Iterator<Item = usize>> = if operation == "findlastchara" {
            Box::new((start..end).rev())
        } else {
            Box::new(start..end)
        };
        for index in indices {
            if &values[index] == needle {
                return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
            }
        }
        return Ok(VmValue::Integer(-1));
    }

    let number = integer_argument(arguments, 0)?;
    let field_index = if matches!(
        operation,
        "csvcstr"
            | "csvbase"
            | "csvabl"
            | "csvmark"
            | "csvexp"
            | "csvrelation"
            | "csvtalent"
            | "csvcflag"
            | "csvequip"
            | "csvjuel"
    ) {
        usize::try_from(integer_argument(arguments, 1)?).map_err(|_| {
            script_native_error(
                crate::ScriptFaultKind::Bounds,
                "CSV field index is negative".into(),
            )
        })?
    } else {
        0
    };
    let sp_argument = if matches!(
        operation,
        "csvcstr"
            | "csvbase"
            | "csvabl"
            | "csvmark"
            | "csvexp"
            | "csvrelation"
            | "csvtalent"
            | "csvcflag"
            | "csvequip"
            | "csvjuel"
    ) {
        2
    } else {
        1
    };
    let requested_sp = optional_integer_argument(arguments, sp_argument, 0)? != 0;
    let template = artifact
        .project_data
        .static_data
        .characters
        .iter()
        .find(|template| template.no == number && template.is_sp_character == requested_sp);
    if operation == "existcsv" {
        return Ok(VmValue::Integer(i64::from(template.is_some())));
    }
    let template = template.ok_or_else(|| {
        script_native_error(
            crate::ScriptFaultKind::Resolve,
            format!("character CSV number {number} does not exist"),
        )
    })?;
    let value = match operation {
        "csvname" => VmValue::String(template.name.clone()),
        "csvcallname" => VmValue::String(template.call_name.clone()),
        "csvnickname" => VmValue::String(template.nick_name.clone()),
        "csvmastername" => VmValue::String(template.master_name.clone()),
        "csvcstr" => VmValue::String(template.cstr.get(&field_index).cloned().unwrap_or_default()),
        "csvbase" => VmValue::Integer(*template.max_base.get(&field_index).unwrap_or(&0)),
        "csvabl" => VmValue::Integer(*template.abl.get(&field_index).unwrap_or(&0)),
        "csvmark" => VmValue::Integer(*template.mark.get(&field_index).unwrap_or(&0)),
        "csvexp" => VmValue::Integer(*template.exp.get(&field_index).unwrap_or(&0)),
        "csvrelation" => VmValue::Integer(*template.relation.get(&field_index).unwrap_or(&0)),
        "csvtalent" => VmValue::Integer(*template.talent.get(&field_index).unwrap_or(&0)),
        "csvcflag" => VmValue::Integer(*template.cflag.get(&field_index).unwrap_or(&0)),
        "csvequip" => VmValue::Integer(*template.equip.get(&field_index).unwrap_or(&0)),
        "csvjuel" => VmValue::Integer(*template.juel.get(&field_index).unwrap_or(&0)),
        _ => {
            return Err(VmError::InvalidArguments(
                "unknown character CSV query".into(),
            ));
        }
    };
    Ok(value)
}

#[allow(clippy::too_many_lines)]
pub(super) fn execute_character_mutation(
    vm: &mut Vm,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let generation = vm.current_generation;
    let artifact = &vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("character mutation generation is missing".into()))?
        .artifact;
    if matches!(operation, "pickupchara" | "sortchara") {
        for name in ["CHARANUM", "TARGET", "ASSI", "MASTER"] {
            validate_named_integer_slot(artifact, &vm.memory, name)?;
        }
        let plan = if operation == "pickupchara" {
            plan_pickup_characters(artifact, &vm.memory, arguments)?
        } else {
            plan_sort_characters(generation, artifact, &vm.memory, arguments)?
        };
        commit_character_reorder(artifact, &mut vm.memory, plan)?;
        return Ok(());
    }

    // Validate the calculated destination before any in-place edit, preserving
    // the previous all-or-nothing error behavior without cloning all VM memory.
    validate_named_integer_slot(artifact, &vm.memory, "CHARANUM")?;
    let memory = &mut vm.memory;
    match operation {
        "addchara" | "addspchara" => {
            let requested_sp = operation == "addspchara";
            let templates = arguments
                .iter()
                .map(|argument| {
                    let VmValue::Integer(number) = argument else {
                        return Err(VmError::InvalidArguments(
                            "ADDCHARA arguments must be integers".into(),
                        ));
                    };
                    artifact
                        .project_data
                        .static_data
                        .characters
                        .iter()
                        .find(|template| {
                            template.no == *number && template.is_sp_character == requested_sp
                        })
                        .ok_or_else(|| {
                            script_native_error(
                                crate::ScriptFaultKind::Resolve,
                                format!("character template {number} does not exist"),
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            for template in templates {
                memory.push_character(artifact, Some(template));
            }
        }
        "adddefchara" => {
            let mut csv_numbers = vec![0];
            if artifact
                .project_data
                .static_data
                .game_base
                .default_character
                > 0
            {
                csv_numbers.push(
                    artifact
                        .project_data
                        .static_data
                        .game_base
                        .default_character,
                );
            }
            for csv_number in csv_numbers {
                let template = artifact
                    .project_data
                    .static_data
                    .characters
                    .iter()
                    .find(|template| template.csv_no == csv_number);
                memory.push_character(artifact, template);
            }
        }
        "addvoidchara" => memory.push_character(artifact, None),
        "delchara" => {
            let mut indices = arguments
                .iter()
                .map(|value| match value {
                    VmValue::Integer(value) => usize::try_from(*value).map_err(|_| {
                        script_native_error(
                            crate::ScriptFaultKind::Bounds,
                            "DELCHARA index is negative".into(),
                        )
                    }),
                    _ => Err(VmError::InvalidArguments(
                        "DELCHARA arguments must be integers".into(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            indices.sort_unstable();
            if indices.windows(2).any(|pair| pair[0] == pair[1])
                || indices
                    .last()
                    .is_some_and(|index| *index >= memory.characters.len())
            {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "DELCHARA index is duplicated or out of range".into(),
                ));
            }
            let order = (0..memory.characters.len())
                .filter(|index| indices.binary_search(index).is_err())
                .collect::<Vec<_>>();
            memory
                .array_leases
                .remap_characters(None, &memory.characters, &order)?;
            for index in indices.into_iter().rev() {
                memory.characters.remove(index);
            }
        }
        "delallchara" => {
            memory
                .array_leases
                .remap_characters(None, &memory.characters, &[])?;
            memory.characters.clear();
        }
        "swapchara" | "copychara" => {
            let left = usize::try_from(integer_argument(arguments, 0)?).map_err(|_| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "character index is negative".into(),
                )
            })?;
            let right = usize::try_from(integer_argument(arguments, 1)?).map_err(|_| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "character index is negative".into(),
                )
            })?;
            if left >= memory.characters.len() || right >= memory.characters.len() {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "character index is out of range".into(),
                ));
            }
            if operation == "swapchara" {
                let mut order = (0..memory.characters.len()).collect::<Vec<_>>();
                order.swap(left, right);
                memory
                    .array_leases
                    .remap_characters(None, &memory.characters, &order)?;
                memory.characters.swap(left, right);
            } else {
                memory.array_leases.character_values_replaced();
                memory.characters[right] = memory.characters[left].clone();
            }
        }
        "addcopychara" => {
            let original_len = memory.characters.len();
            let mut additions = Vec::with_capacity(arguments.len());
            for argument in arguments {
                let VmValue::Integer(index) = argument else {
                    return Err(VmError::InvalidArguments(
                        "ADDCOPYCHARA arguments must be integers".into(),
                    ));
                };
                let index = usize::try_from(*index).map_err(|_| {
                    script_native_error(
                        crate::ScriptFaultKind::Bounds,
                        "ADDCOPYCHARA index is negative".into(),
                    )
                })?;
                let character = if index < original_len {
                    memory.characters.get(index)
                } else {
                    additions.get(index - original_len)
                }
                .cloned()
                .ok_or_else(|| {
                    script_native_error(
                        crate::ScriptFaultKind::Bounds,
                        "ADDCOPYCHARA index is out of range".into(),
                    )
                })?;
                additions.push(character);
            }
            memory.characters.extend(additions);
        }
        "reset_stain" => {
            let character = usize::try_from(integer_argument(arguments, 0)?).map_err(|_| {
                script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "RESET_STAIN index is negative".into(),
                )
            })?;
            if character >= memory.characters.len() {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Bounds,
                    "RESET_STAIN character index is out of range".into(),
                ));
            }
            let definition = character_definition(artifact, "STAIN")
                .ok_or_else(|| VmError::InvalidState("STAIN variable is missing".into()))?;
            let cell = memory
                .cell_mut(generation, definition.key, definition.storage, character)
                .ok_or_else(|| VmError::InvalidState("STAIN storage is unavailable".into()))?;
            let destinations = cell.integers_mut().ok_or_else(|| {
                VmError::InvalidState("STAIN storage is not an integer array".into())
            })?;
            for (index, destination) in destinations.iter_mut().enumerate() {
                *destination = artifact
                    .project_data
                    .static_data
                    .replace
                    .stain_default
                    .get(index)
                    .copied()
                    .unwrap_or(0);
            }
        }
        _ => {
            return Err(VmError::InvalidArguments(
                "unknown character mutation".into(),
            ));
        }
    }
    // CHARANUM is exposed as a calculated variable by the language frontend, but
    // the VM stores calculated cells so normal bytecode loads stay inexpensive.
    // Refresh it after the validated in-place mutation.
    let character_count = i64::try_from(memory.characters.len()).unwrap_or(i64::MAX);
    write_named_integer(artifact, memory, "CHARANUM", character_count)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
struct CharacterReorderPlan {
    order: Vec<usize>,
    scalar_updates: Vec<(&'static str, i64)>,
}

fn character_sort_key<'a>(
    artifact: &'a erabasic_bytecode::BytecodeArtifact,
    arguments: &[VmValue],
) -> Result<(&'a erabasic_bytecode::BytecodeGlobal, Vec<u64>, bool), VmError> {
    let selected = match arguments.first() {
        None => (
            character_definition(artifact, "NO")
                .ok_or_else(|| VmError::InvalidState("NO variable is missing".into()))?,
            Vec::new(),
            false,
        ),
        Some(VmValue::String(order))
            if order.eq_ignore_ascii_case("FORWARD") || order.eq_ignore_ascii_case("BACK") =>
        {
            (
                character_definition(artifact, "NO")
                    .ok_or_else(|| VmError::InvalidState("NO variable is missing".into()))?,
                Vec::new(),
                order.eq_ignore_ascii_case("BACK"),
            )
        }
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => {
            let definition = artifact
                .globals
                .iter()
                .find(|definition| definition.key == place.variable)
                .ok_or_else(|| VmError::InvalidState("SORTCHARA variable is missing".into()))?;
            if definition.storage != BytecodeStorage::Character {
                return Err(script_native_error(
                    crate::ScriptFaultKind::Argument,
                    "SORTCHARA key must be a character variable".into(),
                ));
            }
            let descending = matches!(arguments.get(1), Some(VmValue::String(value)) if value.eq_ignore_ascii_case("BACK"));
            (definition, place.indices.clone(), descending)
        }
        Some(VmValue::String(_)) => {
            return Err(script_native_error(
                crate::ScriptFaultKind::Argument,
                "SORTCHARA key or order is invalid".into(),
            ));
        }
        _ => {
            return Err(VmError::InvalidArguments(
                "SORTCHARA key or order is invalid".into(),
            ));
        }
    };
    Ok(selected)
}

fn plan_sort_characters(
    generation: crate::GenerationId,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &crate::Memory,
    arguments: &[VmValue],
) -> Result<CharacterReorderPlan, VmError> {
    if memory.characters.len() <= 1 {
        return Ok(CharacterReorderPlan {
            order: (0..memory.characters.len()).collect(),
            scalar_updates: vec![(
                "CHARANUM",
                i64::try_from(memory.characters.len()).unwrap_or(i64::MAX),
            )],
        });
    }
    let (definition, indices, descending) = character_sort_key(artifact, arguments)?;
    let master = read_named_integer(artifact, memory, "MASTER").unwrap_or(-1);
    let target = read_named_integer(artifact, memory, "TARGET").unwrap_or(-1);
    let assi = read_named_integer(artifact, memory, "ASSI").unwrap_or(-1);
    let master_index = usize::try_from(master)
        .ok()
        .filter(|index| *index < memory.characters.len());
    let mut order = (0..memory.characters.len())
        .filter(|index| Some(*index) != master_index)
        .map(|index| {
            let value = memory
                .cell(generation, definition, index)
                .ok_or_else(|| {
                    VmError::InvalidState("SORTCHARA key storage is unavailable".into())
                })?
                .read_execution(&indices)
                .map_err(VmError::ScriptFailure)?;
            Ok((index, value))
        })
        .collect::<Result<Vec<_>, VmError>>()?;
    order.sort_by(|(_, left), (_, right)| match (left, right) {
        (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
        (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
        _ => std::cmp::Ordering::Equal,
    });
    if descending {
        order.reverse();
    }
    let mut final_order = order.iter().map(|(index, _)| *index).collect::<Vec<_>>();
    if let Some(master_index) = master_index {
        final_order.insert(master_index, master_index);
    }
    let new_index = |old_index: i64| {
        usize::try_from(old_index).ok().and_then(|old_index| {
            final_order
                .iter()
                .position(|candidate| *candidate == old_index)
        })
    };
    let mut scalar_updates = vec![(
        "CHARANUM",
        i64::try_from(final_order.len()).unwrap_or(i64::MAX),
    )];
    for (name, old_index) in [("TARGET", target), ("ASSI", assi)] {
        if let Some(index) = new_index(old_index) {
            scalar_updates.push((name, i64::try_from(index).unwrap_or(i64::MAX)));
        }
    }
    Ok(CharacterReorderPlan {
        order: final_order,
        scalar_updates,
    })
}

fn plan_pickup_characters(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &crate::Memory,
    arguments: &[VmValue],
) -> Result<CharacterReorderPlan, VmError> {
    let mut selected = Vec::new();
    for argument in arguments {
        let VmValue::Integer(value) = argument else {
            return Err(VmError::InvalidArguments(
                "PICKUPCHARA arguments must be integers".into(),
            ));
        };
        if *value < 0 {
            continue;
        }
        let index = usize::try_from(*value).unwrap_or(usize::MAX);
        if index >= memory.characters.len() {
            return Err(script_native_error(
                crate::ScriptFaultKind::Bounds,
                "PICKUPCHARA index is out of range".into(),
            ));
        }
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    let old_special = ["TARGET", "ASSI", "MASTER"]
        .map(|name| read_named_integer(artifact, memory, name).unwrap_or(-1));
    let mut scalar_updates = vec![(
        "CHARANUM",
        i64::try_from(selected.len()).unwrap_or(i64::MAX),
    )];
    for (name, old) in ["TARGET", "ASSI", "MASTER"].into_iter().zip(old_special) {
        let replacement = usize::try_from(old)
            .ok()
            .and_then(|old| selected.iter().position(|candidate| *candidate == old))
            .map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        scalar_updates.push((name, replacement));
    }
    Ok(CharacterReorderPlan {
        order: selected,
        scalar_updates,
    })
}

fn commit_character_reorder(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut crate::Memory,
    plan: CharacterReorderPlan,
) -> Result<(), VmError> {
    let originals = plan
        .scalar_updates
        .iter()
        .map(|(name, _)| {
            read_named_integer(artifact, memory, name)
                .map(|value| (*name, value))
                .ok_or_else(|| VmError::InvalidState(format!("{name} is not an integer slot")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (index, (name, value)) in plan.scalar_updates.iter().enumerate() {
        if let Err(error) = write_named_integer(artifact, memory, name, *value) {
            for (name, value) in originals[..index].iter().rev() {
                let _ = write_named_integer(artifact, memory, name, *value);
            }
            return Err(error);
        }
    }
    memory
        .array_leases
        .remap_characters(None, &memory.characters, &plan.order)?;
    let mut old = std::mem::take(&mut memory.characters)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    memory.characters = plan
        .order
        .into_iter()
        .map(|index| {
            old[index]
                .take()
                .expect("validated reorder plan contains unique in-range indices")
        })
        .collect();
    Ok(())
}

pub(super) fn read_named_integer(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &crate::Memory,
    name: &str,
) -> Option<i64> {
    let definition = shared_definition(artifact, name)?;
    match memory.shared.get(&definition.key)?.first()? {
        VmValue::Integer(value) => Some(value),
        _ => None,
    }
}

pub(super) fn write_named_integer(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut crate::Memory,
    name: &str,
    value: i64,
) -> Result<(), VmError> {
    let definition = shared_definition(artifact, name)
        .ok_or_else(|| VmError::InvalidState(format!("{name} is not defined")))?;
    let cell = memory
        .shared
        .get_mut(&definition.key)
        .ok_or_else(|| VmError::InvalidState(format!("{name} storage is unavailable")))?;
    cell.set(0, VmValue::Integer(value))
        .map_err(VmError::InvalidState)
}

fn validate_named_integer_slot(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &crate::Memory,
    name: &str,
) -> Result<(), VmError> {
    let definition = shared_definition(artifact, name)
        .ok_or_else(|| VmError::InvalidState(format!("{name} is not defined")))?;
    if definition.value_type != erabasic_bytecode::BytecodeType::Integer {
        return Err(VmError::InvalidState(format!(
            "{name} is not an integer variable"
        )));
    }
    let value = memory
        .shared
        .get(&definition.key)
        .and_then(crate::VariableCell::first)
        .ok_or_else(|| VmError::InvalidState(format!("{name} storage is unavailable")))?;
    if !matches!(value, VmValue::Integer(_)) {
        return Err(VmError::InvalidState(format!(
            "{name} storage is not an integer slot"
        )));
    }
    Ok(())
}
