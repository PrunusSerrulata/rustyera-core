#[allow(clippy::wildcard_imports)]
use super::*;

pub(super) fn usize_cursor(cursor: Option<u64>) -> Result<Option<usize>, RuntimeError> {
    cursor
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| RuntimeError::ResourceLimit("debug cursor is too large"))
        })
        .transpose()
}

pub(super) fn vm_step_kind(kind: StepKind) -> VmStepKind {
    match kind {
        StepKind::Instruction => VmStepKind::Instruction,
        StepKind::SourceLine => VmStepKind::SourceLine,
        StepKind::Into => VmStepKind::Into,
        StepKind::Over => VmStepKind::Over,
        StepKind::Out => VmStepKind::Out,
    }
}

pub(super) fn protocol_source(
    source: erabasic_bytecode::ResolvedSourceLocation,
) -> DebugSourceLocation {
    DebugSourceLocation {
        relative_path: source.relative_path,
        content_hash: ProtocolBytes::new(source.content_hash.0),
        byte_start: source.byte_start,
        byte_end: source.byte_end,
        line: source.line,
        byte_column: source.byte_column,
    }
}

pub(super) fn protocol_fiber(fiber: &erabasic_vm::VmDebugFiber) -> FiberSummary {
    let state = match &fiber.status {
        FiberStatus::Runnable => FiberState::Runnable,
        FiberStatus::WaitingHost(_) => FiberState::WaitingHost,
        FiberStatus::WaitingResume => FiberState::WaitingResume,
        FiberStatus::Completed(_) => FiberState::Completed,
        FiberStatus::Faulted(_) => FiberState::Faulted,
        FiberStatus::Cancelled => FiberState::Cancelled,
    };
    FiberSummary {
        fiber_id: fiber.id.0,
        state,
        primary: fiber.primary,
        frame_count: u32::try_from(fiber.frame_count).unwrap_or(u32::MAX),
    }
}

pub(super) fn protocol_frame(frame: erabasic_vm::VmDebugFrame) -> FrameSummary {
    FrameSummary {
        frame_id: frame.id.0,
        generation: frame.generation.0,
        function_key: ProtocolBytes::new(frame.function.0),
        function_name: frame.function_name,
        instruction: frame.instruction,
        source: frame.source.map(protocol_source),
    }
}

pub(super) fn protocol_value(value: VmValue) -> DebugValue {
    protocol_value_in_generation(value, 0)
}

pub(super) fn protocol_value_in_generation(value: VmValue, generation: u64) -> DebugValue {
    match value {
        VmValue::Integer(value) => DebugValue::Integer(value),
        VmValue::String(value) => DebugValue::String(value),
        VmValue::IntegerPlace(place) => {
            DebugValue::Place(protocol_place(*place, ValueKind::Integer, generation))
        }
        VmValue::StringPlace(place) => {
            DebugValue::Place(protocol_place(*place, ValueKind::String, generation))
        }
    }
}

pub(super) fn protocol_place(
    place: PlaceDescriptor,
    value_kind: ValueKind,
    generation: u64,
) -> era_debug_protocol::DebugPlace {
    era_debug_protocol::DebugPlace {
        symbol_key: ProtocolBytes::new(place.variable.0),
        value_kind,
        indices: place.indices,
        character: place.character,
        fiber_id: place.fiber.map(|value| value.0),
        frame_id: place.frame.map(|value| value.0),
        generation,
    }
}

pub(super) fn vm_value(value: &DebugValue) -> Result<VmValue, &'static str> {
    match value {
        DebugValue::Integer(value) => Ok(VmValue::Integer(*value)),
        DebugValue::String(value) => Ok(VmValue::String(value.clone())),
        _ => Err("VM variables accept only integer or string values"),
    }
}

pub(super) fn vm_variable_reference(
    value: &VariableReference,
) -> Result<VmDebugVariableRef, &'static str> {
    let bytes: [u8; 16] = value
        .symbol_key
        .as_slice()
        .try_into()
        .map_err(|_| "variable symbol key must contain 16 bytes")?;
    Ok(VmDebugVariableRef {
        target: PlaceDescriptor {
            backing: None,
            variable: SymbolKey(bytes),
            indices: value.indices.clone(),
            character: value.character,
            fiber: value.fiber_id.map(FiberId),
            frame: value.frame_id.map(FrameId),
        },
        generation: GenerationId(value.generation),
    })
}

pub(super) fn protocol_variable_value(value: VmDebugVariable) -> VariableValue {
    let storage = if value.target.target.fiber.is_some() {
        VariableStorage::Local
    } else if value.target.target.character.is_some() {
        VariableStorage::Character
    } else {
        VariableStorage::Global
    };
    VariableValue {
        reference: VariableReference {
            symbol_key: ProtocolBytes::new(value.target.target.variable.0),
            storage,
            fiber_id: value.target.target.fiber.map(|item| item.0),
            frame_id: value.target.target.frame.map(|item| item.0),
            generation: value.target.generation.0,
            character: value.target.target.character,
            indices: value.target.target.indices,
        },
        value: protocol_value(value.value),
        revision: value.revision,
    }
}

pub(super) fn protocol_storage(storage: BytecodeStorage) -> VariableStorage {
    match storage {
        BytecodeStorage::FunctionLocal => VariableStorage::Local,
        BytecodeStorage::FunctionStatic => VariableStorage::FunctionStatic,
        BytecodeStorage::Character => VariableStorage::Character,
        _ => VariableStorage::Global,
    }
}

pub(super) fn game_field_descriptors() -> Vec<GameFieldDescriptor> {
    vec![
        GameFieldDescriptor {
            key: "input.message_skip".into(),
            value_kind: ValueKind::Boolean,
            mutability: FieldMutability::DebugWritable,
            description: "Runtime-owned message-skip latch".into(),
        },
        GameFieldDescriptor {
            key: "runtime.logical_time_ns".into(),
            value_kind: ValueKind::Integer,
            mutability: FieldMutability::ReadOnly,
            description: "Authoritative logical clock".into(),
        },
        GameFieldDescriptor {
            key: "runtime.phase".into(),
            value_kind: ValueKind::String,
            mutability: FieldMutability::ReadOnly,
            description: "Current runtime lifecycle phase".into(),
        },
        GameFieldDescriptor {
            key: "runtime.revision".into(),
            value_kind: ValueKind::Integer,
            mutability: FieldMutability::ReadOnly,
            description: "Runtime mutation revision".into(),
        },
    ]
}

pub(super) fn vm_breakpoint(value: &Breakpoint) -> Result<VmBreakpoint, &'static str> {
    let location = match &value.location {
        BreakpointLocation::Function { symbol_key } => {
            let bytes: [u8; 16] = symbol_key
                .as_slice()
                .try_into()
                .map_err(|_| "function symbol key must contain 16 bytes")?;
            VmBreakpointLocation::Function(SymbolKey(bytes))
        }
        BreakpointLocation::Source {
            relative_path,
            content_hash,
            byte_offset,
        } => {
            let bytes: [u8; 32] = content_hash
                .as_slice()
                .try_into()
                .map_err(|_| "source content hash must contain 32 bytes")?;
            VmBreakpointLocation::Source {
                relative_path: relative_path.clone(),
                content_hash: Digest(bytes),
                byte_offset: *byte_offset,
            }
        }
    };
    Ok(VmBreakpoint {
        id: value.breakpoint_id,
        enabled: value.enabled,
        hit_count: 0,
        location,
    })
}

pub(super) fn protocol_breakpoint(value: VmResolvedBreakpoint) -> ResolvedBreakpoint {
    ResolvedBreakpoint {
        breakpoint_id: value.id,
        generation: value.generation.0,
        binding: match value.binding {
            VmBreakpointBinding::Verified => BreakpointBinding::Verified,
            VmBreakpointBinding::Moved => BreakpointBinding::Moved,
            VmBreakpointBinding::Unbound => BreakpointBinding::Unbound,
        },
        source: value.source.map(protocol_source),
        message: value.message,
        hit_count: value.hit_count,
    }
}
