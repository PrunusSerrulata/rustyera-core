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
    let mut locals: VariableMap = local_definitions
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
        user_call: None,
        event_context,
        event_dispatch: None,
        runtime_form: None,
        user_calls: Vec::new(),
        existvar_checks: Vec::new(),
        map_calls: Vec::new(),
        bit_calls: Vec::new(),
        match_calls: Vec::new(),
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
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        let Some(destination) =
            persistent_argument_destination(memory, generation, parameter, program)?
        else {
            continue;
        };
        memory
            .cell_mut(
                generation,
                destination.definition.key,
                destination.definition.storage,
                destination.character,
            )
            .ok_or_else(|| VmError::InvalidState("parameter storage is missing".into()))?
            .write(destination.indices, argument.clone())
            .map_err(VmError::InvalidState)?;
    }
    Ok(())
}

pub(crate) struct PersistentArgumentDestination<'a> {
    pub definition: &'a BytecodeGlobal,
    pub character: usize,
    pub implicit_target: bool,
    pub indices: &'a [u64],
}

pub(crate) fn persistent_argument_destination<'a>(
    memory: &Memory,
    generation: GenerationId,
    parameter: &'a erabasic_bytecode::BytecodeParameter,
    program: &'a ProgramGeneration,
) -> Result<Option<PersistentArgumentDestination<'a>>, VmError> {
    let Some(definition) = program.global(parameter.key) else {
        return Err(VmError::InvalidState("parameter storage is missing".into()));
    };
    if definition.storage == BytecodeStorage::FunctionLocal {
        return Ok(None);
    }
    if definition.storage == BytecodeStorage::Character {
        if parameter.indices.len() > definition.dimensions.len() {
            let character = usize::try_from(parameter.indices[0]).unwrap_or(usize::MAX);
            return Ok(Some(PersistentArgumentDestination {
                definition,
                character,
                implicit_target: false,
                indices: &parameter.indices[1..],
            }));
        }
        return Ok(Some(PersistentArgumentDestination {
            definition,
            character: memory.target_character(&program.artifact, generation),
            implicit_target: true,
            indices: &parameter.indices,
        }));
    }
    Ok(Some(PersistentArgumentDestination {
        definition,
        character: 0,
        implicit_target: false,
        indices: &parameter.indices,
    }))
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
