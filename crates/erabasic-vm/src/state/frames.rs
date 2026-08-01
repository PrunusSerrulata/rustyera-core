#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn make_frame<'a>(
    id: FrameId,
    generation: GenerationId,
    function: &BytecodeFunction,
    local_definitions: impl IntoIterator<Item = &'a erabasic_bytecode::BytecodeGlobal>,
    arguments: Vec<VmValue>,
    return_value_to_caller: bool,
    event_context: bool,
) -> Frame {
    let mut locals: BTreeMap<_, _> = local_definitions
        .into_iter()
        .map(|definition| (definition.key, VariableCell::new(definition)))
        .collect();
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        if let Some(cell) = locals.get_mut(&parameter.key) {
            if parameter.by_reference {
                // REF declarations describe the target shape, but their frame
                // storage always contains one opaque alias. This also covers a
                // scalar `#DIM REF value`: its declaration has shape `[1]`, so
                // treating only zero-sized REF arrays specially would attempt
                // to write an IntegerPlace into an Integer cell.
                cell.replace_shape(parameter.value_type, vec![1], vec![argument])
                    .expect("validated reference argument matches its parameter");
            } else {
                cell.write(&parameter.indices, argument)
                    .expect("validated parameter destination fits its local storage");
            }
        }
    }
    Frame {
        id,
        generation,
        function: function.key,
        instruction: 0,
        stack: Vec::new(),
        for_loops: Vec::new(),
        select_values: Vec::new(),
        locals,
        return_value_to_caller,
        event_context,
        event_dispatch: None,
    }
}

pub(crate) fn validate_arguments(
    function: &BytecodeFunction,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    if arguments.len() != function.parameters.len() {
        return Err(VmError::InvalidArguments(format!(
            "function {} expects {} arguments, found {}",
            function.name,
            function.parameters.len(),
            arguments.len()
        )));
    }
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        if parameter.value_type != argument.value_type() {
            return Err(VmError::InvalidArguments(format!(
                "function {} expects {:?}, found {:?}",
                function.name,
                parameter.value_type,
                argument.value_type()
            )));
        }
    }
    Ok(())
}

pub(crate) fn bind_persistent_arguments(
    memory: &mut Memory,
    generation: GenerationId,
    function: &BytecodeFunction,
    program: &ProgramGeneration,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let artifact = &program.artifact;
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        let Some(definition) = program.global(parameter.key) else {
            return Err(VmError::InvalidState("parameter storage is missing".into()));
        };
        if definition.storage == BytecodeStorage::FunctionLocal {
            continue;
        }
        let (character, indices) = if definition.storage == BytecodeStorage::Character
            && parameter.indices.len() > definition.dimensions.len()
        {
            (
                usize::try_from(parameter.indices[0]).unwrap_or(usize::MAX),
                &parameter.indices[1..],
            )
        } else {
            (
                if definition.storage == BytecodeStorage::Character {
                    memory.target_character(artifact, generation)
                } else {
                    0
                },
                parameter.indices.as_slice(),
            )
        };
        memory
            .cell_mut(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("parameter storage is missing".into()))?
            .write(indices, argument.clone())
            .map_err(VmError::InvalidState)?;
    }
    Ok(())
}

pub(crate) fn prepare_dynamic_arguments(
    function: &BytecodeFunction,
    mut arguments: Vec<VmValue>,
    compatibility: erabasic_bytecode::BytecodeCallCompatibility,
) -> Result<Vec<VmValue>, VmError> {
    if arguments.len() > function.parameters.len() {
        return Err(VmError::InvalidArguments(format!(
            "function {} expects at most {} arguments, found {}",
            function.name,
            function.parameters.len(),
            arguments.len()
        )));
    }
    while arguments.len() < function.parameters.len() {
        arguments.push(VmValue::Integer(i64::MIN));
    }
    for (parameter, argument) in function.parameters.iter().zip(&mut arguments) {
        if matches!(argument, VmValue::Integer(value) if *value == i64::MIN) {
            if parameter.by_reference {
                return Err(VmError::InvalidArguments(format!(
                    "function {} omits a reference argument",
                    function.name
                )));
            }
            *argument = match &parameter.default {
                Some(BytecodeConstant::Integer(value)) => VmValue::Integer(*value),
                Some(BytecodeConstant::String(value)) => VmValue::String(value.clone()),
                None if compatibility.allow_omitted_arguments => match parameter.value_type {
                    BytecodeType::Integer => VmValue::Integer(0),
                    BytecodeType::String => VmValue::String(String::new()),
                    BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                        return Err(VmError::InvalidArguments(format!(
                            "function {} omits a reference argument",
                            function.name
                        )));
                    }
                },
                None => {
                    return Err(VmError::InvalidArguments(format!(
                        "function {} omits a required argument",
                        function.name
                    )));
                }
            };
        }
        if compatibility.auto_convert_integer_to_string
            && parameter.value_type == BytecodeType::String
            && matches!(argument, VmValue::Integer(_))
            && !parameter.by_reference
        {
            let VmValue::Integer(value) = argument else {
                unreachable!("checked integer argument")
            };
            *argument = VmValue::String(value.to_string());
        }
    }
    validate_arguments(function, &arguments)?;
    Ok(arguments)
}

pub(crate) fn find_global(
    artifact: &BytecodeArtifact,
    key: SymbolKey,
) -> Result<&erabasic_bytecode::BytecodeGlobal, VmError> {
    artifact
        .globals
        .iter()
        .find(|definition| definition.key == key)
        .ok_or_else(|| VmError::InvalidState(format!("variable {key:?} is not defined")))
}

pub(super) fn find_frame(
    fiber: &Fiber,
    frame: Option<FrameId>,
    owner: Option<SymbolKey>,
) -> Result<&Frame, VmError> {
    fiber
        .frames
        .iter()
        .rev()
        .find(|candidate| {
            frame.is_none_or(|frame| candidate.id == frame)
                && owner.is_none_or(|owner| candidate.function == owner)
        })
        .ok_or_else(|| VmError::InvalidState("place frame is no longer active".into()))
}

pub(super) fn find_frame_mut(
    fiber: &mut Fiber,
    frame: Option<FrameId>,
    owner: Option<SymbolKey>,
) -> Result<&mut Frame, VmError> {
    fiber
        .frames
        .iter_mut()
        .rev()
        .find(|candidate| {
            frame.is_none_or(|frame| candidate.id == frame)
                && owner.is_none_or(|owner| candidate.function == owner)
        })
        .ok_or_else(|| VmError::InvalidState("place frame is no longer active".into()))
}
