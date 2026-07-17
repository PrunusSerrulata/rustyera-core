use erabasic_bytecode::{
    BytecodeFunctionKind, BytecodeStorage, BytecodeType, EncodedInstruction,
    HostSnapshotCapability, ImportKind, Opcode, SymbolKey, opcode,
};

use crate::state::{EventDispatch, EventDispatchEntry};
use crate::{
    Fiber, FiberId, FiberState, HostCallRequest, HostCallResult, HostReady, HostWaitStability,
    NativeCallRequest, NativePlaceView, NativeReady, NativeServiceRegistry, PlaceDescriptor,
    RunBudget, Vm, VmError, VmEvent, VmFault, VmFaultCode, VmHost, VmRunReport, VmRunStop, VmValue,
    WaitingHost, bind_persistent_arguments, find_global, make_frame, prepare_dynamic_arguments,
    validate_arguments,
};

mod character_ops;
mod extended_ops;
mod native_ops;
mod operand;

use character_ops::{character_series, execute_character_mutation, execute_character_query};
use extended_ops::{
    array_snapshot_any_rank, execute_array_copy, execute_array_multi_sort,
    execute_array_multi_sort_ex, execute_random_place_transaction, execute_regex_match,
    global_unindexed_place,
};
use native_ops::{
    array_place, array_snapshot, execute_array_mutation, execute_array_query, execute_bit_mutation,
    execute_find_element, execute_getnum, execute_split_transaction, execute_strjoin,
    execute_swap_transaction, execute_variable_fill, integer_argument, native_implicit_place_views,
    native_place_views, optional_index, validate_native_ready,
};
use operand::{
    assign_binary_tag, binary_value, exact, map_vm_error, pop, pop_arguments, pop_indices,
    read_key, read_u16, read_u32, unary_value,
};

enum StepOutcome {
    Continue,
    Yielded,
    Blocked,
    Completed(Option<VmValue>),
}

struct StepError {
    code: VmFaultCode,
    message: String,
}

impl StepError {
    fn new(code: VmFaultCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

struct InstructionPosition {
    generation: crate::GenerationId,
    function: SymbolKey,
    instruction: usize,
    encoded: EncodedInstruction,
}

impl Vm {
    #[allow(clippy::too_many_lines)]
    pub fn run_slice(
        &mut self,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        budget: RunBudget,
    ) -> VmRunReport {
        let mut report = VmRunReport {
            stop: VmRunStop::Idle,
            instructions: 0,
            host_calls: 0,
            events: Vec::new(),
        };
        if self.debug_is_paused() {
            return report;
        }
        if let Some(selected) = self.debug_step_fiber()
            && let Some(index) = self.runnable.iter().position(|fiber| *fiber == selected)
            && let Some(selected) = self.runnable.remove(index)
        {
            self.runnable.push_front(selected);
        }
        let quantum = budget.fiber_quantum.max(1);
        let mut budget_exhausted = false;
        while let Some(fiber_id) = self.runnable.pop_front() {
            if self
                .debug_step_fiber()
                .is_some_and(|selected| selected != fiber_id)
            {
                self.runnable.push_front(fiber_id);
                break;
            }
            if report.instructions >= budget.maximum_instructions {
                self.runnable.push_front(fiber_id);
                budget_exhausted = true;
                break;
            }
            let Some(mut fiber) = self.fibers.remove(&fiber_id) else {
                continue;
            };
            if !matches!(fiber.state, FiberState::Runnable) {
                self.fibers.insert(fiber_id, fiber);
                continue;
            }
            let mut used = 0u32;
            let mut yielded = false;
            while used < quantum && matches!(fiber.state, FiberState::Runnable) {
                if report.instructions >= budget.maximum_instructions {
                    budget_exhausted = true;
                    break;
                }
                if let Some(stop) = self.debug_stop_before(&fiber) {
                    report.events.push(VmEvent::DebugStopped(stop));
                    break;
                }
                let position = match self.instruction_position(&fiber) {
                    Ok(position) => position,
                    Err(error) => {
                        let fallback = fiber.frames.last().map_or(
                            InstructionPosition {
                                generation: self.current_generation,
                                function: SymbolKey::default(),
                                instruction: 0,
                                encoded: EncodedInstruction {
                                    opcode: Opcode::Trap as u16,
                                    payload: Vec::new(),
                                },
                            },
                            |frame| InstructionPosition {
                                generation: frame.generation,
                                function: frame.function,
                                instruction: frame.instruction,
                                encoded: EncodedInstruction {
                                    opcode: Opcode::Trap as u16,
                                    payload: Vec::new(),
                                },
                            },
                        );
                        let fault = self.make_fault(
                            fiber.id,
                            &fallback,
                            VmFaultCode::InvalidInstruction,
                            error.to_string(),
                        );
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                        break;
                    }
                };
                if position.encoded.opcode == Opcode::CallHost as u16
                    && report.host_calls >= budget.maximum_host_calls
                {
                    budget_exhausted = true;
                    break;
                }
                let host_before = report.host_calls;
                let outcome = self.execute_instruction(
                    &mut fiber,
                    &position,
                    host,
                    natives,
                    &mut report.host_calls,
                );
                report.instructions = report.instructions.saturating_add(1);
                used = used.saturating_add(1);
                if report.host_calls != host_before {
                    fiber.mark_progress();
                }
                match outcome {
                    Ok(StepOutcome::Continue) => {
                        if let Some(stop) = self.debug_stop_after(&fiber, false, false) {
                            report.events.push(VmEvent::DebugStopped(stop));
                            break;
                        }
                    }
                    Ok(StepOutcome::Yielded) => {
                        fiber.mark_progress();
                        yielded = true;
                        report
                            .events
                            .push(VmEvent::FiberYielded { fiber: fiber.id });
                        if let Some(stop) = self.debug_stop_after(&fiber, false, false) {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Ok(StepOutcome::Blocked) => {
                        fiber.mark_progress();
                        if let FiberState::WaitingHost(wait) = &fiber.state {
                            report.events.push(VmEvent::HostPending {
                                fiber: fiber.id,
                                request: wait.request,
                            });
                        }
                        if let Some(stop) = self.debug_stop_after(&fiber, true, false) {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Ok(StepOutcome::Completed(value)) => {
                        report.events.push(VmEvent::FiberCompleted {
                            fiber: fiber.id,
                            value,
                        });
                        if let Some(stop) = self.debug_stop_after(&fiber, false, true) {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Err(error) => {
                        let fault = self.make_fault(fiber.id, &position, error.code, error.message);
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                        break;
                    }
                }
                if fiber.backward_branches_without_progress
                    > self.config.maximum_backward_branches_without_progress
                {
                    let fault = self.make_fault(
                        fiber.id,
                        &position,
                        VmFaultCode::RunawayExecution,
                        "backward-branch watchdog detected execution without host progress",
                    );
                    fiber.state = FiberState::Faulted(fault.clone());
                    report.events.push(VmEvent::FiberFaulted {
                        fiber: fiber.id,
                        fault,
                    });
                    break;
                }
            }

            if matches!(fiber.state, FiberState::Runnable) {
                if used >= quantum && !yielded {
                    fiber.consecutive_budget_exhaustions =
                        fiber.consecutive_budget_exhaustions.saturating_add(1);
                    if fiber.consecutive_budget_exhaustions
                        > self.config.maximum_consecutive_budget_exhaustions
                    {
                        let position =
                            self.instruction_position(&fiber)
                                .unwrap_or(InstructionPosition {
                                    generation: self.current_generation,
                                    function: SymbolKey::default(),
                                    instruction: 0,
                                    encoded: EncodedInstruction {
                                        opcode: Opcode::Trap as u16,
                                        payload: Vec::new(),
                                    },
                                });
                        let fault = self.make_fault(
                            fiber.id,
                            &position,
                            VmFaultCode::RunawayExecution,
                            "instruction-budget watchdog detected persistent execution without progress",
                        );
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                    }
                }
                if matches!(fiber.state, FiberState::Runnable) {
                    self.runnable.push_back(fiber_id);
                }
            }
            self.fibers.insert(fiber_id, fiber);
            if budget_exhausted || self.debug_is_paused() {
                break;
            }
        }
        self.reclaim_generations();
        report.stop = if budget_exhausted || !self.runnable.is_empty() {
            VmRunStop::BudgetExhausted
        } else {
            VmRunStop::Idle
        };
        report
    }

    fn instruction_position(&self, fiber: &Fiber) -> Result<InstructionPosition, VmError> {
        let frame = fiber
            .frames
            .last()
            .ok_or_else(|| VmError::InvalidState("runnable fiber has no frame".into()))?;
        let generation = self
            .generations
            .get(&frame.generation)
            .ok_or_else(|| VmError::InvalidState("frame generation was reclaimed".into()))?;
        let function = generation
            .artifact
            .functions
            .iter()
            .find(|function| function.key == frame.function)
            .ok_or(VmError::MissingFunction(frame.function))?;
        let encoded = function
            .code
            .get(frame.instruction)
            .cloned()
            .ok_or_else(|| VmError::InvalidState("instruction pointer left its function".into()))?;
        Ok(InstructionPosition {
            generation: frame.generation,
            function: frame.function,
            instruction: frame.instruction,
            encoded,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn execute_instruction(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        host_calls: &mut u32,
    ) -> Result<StepOutcome, StepError> {
        let opcode = Opcode::try_from(position.encoded.opcode).map_err(|opcode| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                format!("unknown opcode {opcode}"),
            )
        })?;
        let frame = fiber
            .frames
            .last_mut()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        frame.instruction = frame.instruction.saturating_add(1);
        match opcode {
            Opcode::Nop => {}
            Opcode::PushInteger => {
                let bytes = exact::<8>(&position.encoded.payload)?;
                frame
                    .stack
                    .push(VmValue::Integer(i64::from_le_bytes(bytes)));
            }
            Opcode::PushString => {
                let length = read_u32(&position.encoded.payload, 0)? as usize;
                let bytes = position
                    .encoded
                    .payload
                    .get(4..4 + length)
                    .filter(|_| position.encoded.payload.len() == 4 + length)
                    .ok_or_else(|| {
                        StepError::new(VmFaultCode::InvalidInstruction, "invalid string operand")
                    })?;
                let value = std::str::from_utf8(bytes).map_err(|_| {
                    StepError::new(VmFaultCode::InvalidInstruction, "string is not UTF-8")
                })?;
                frame.stack.push(VmValue::String(value.into()));
            }
            Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace => {
                let key = read_key(&position.encoded.payload)?;
                let count = read_u16(&position.encoded.payload, 16)? as usize;
                let operation = *position.encoded.payload.get(18).ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "missing variable operation",
                    )
                })?;
                let value = (opcode == Opcode::StoreVariable)
                    .then(|| pop(&mut fiber.frames.last_mut().expect("frame exists").stack))
                    .transpose()?;
                let indices = pop_indices(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    count,
                )?;
                let frame = fiber.frames.last().expect("frame exists");
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                let definition = find_global(artifact, key).map_err(|error| {
                    StepError::new(VmFaultCode::MissingSymbol, error.to_string())
                })?;
                let character = (definition.storage == BytecodeStorage::Character)
                    .then(|| self.memory.target_character(artifact, position.generation) as u64);
                let place = PlaceDescriptor {
                    variable: key,
                    indices,
                    character,
                    fiber: Some(fiber.id),
                    frame: (definition.storage == BytecodeStorage::FunctionLocal)
                        .then_some(frame.id),
                };
                if opcode == Opcode::MakePlace {
                    let value = match definition.value_type {
                        BytecodeType::Integer => VmValue::IntegerPlace(place),
                        BytecodeType::String => VmValue::StringPlace(place),
                        BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "a variable schema cannot contain place values",
                            ));
                        }
                    };
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                } else if opcode == Opcode::LoadVariable {
                    let value = self.read_place(fiber, &place).map_err(map_vm_error)?;
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                } else {
                    let mut value = value.expect("store value was popped");
                    if operation != 0 {
                        let previous = self.read_place(fiber, &place).map_err(map_vm_error)?;
                        value = binary_value(assign_binary_tag(operation)?, previous, value)?;
                    }
                    self.write_place(fiber, &place, value)
                        .map_err(map_vm_error)?;
                }
            }
            Opcode::Unary => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing unary operation")
                })?;
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(unary_value(operation, value)?);
            }
            Opcode::Binary => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing binary operation")
                })?;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let right = pop(stack)?;
                let left = pop(stack)?;
                stack.push(binary_value(operation, left, right)?);
            }
            Opcode::ToString => {
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let value = match value {
                    VmValue::Integer(value) => value.to_string(),
                    VmValue::String(value) => value,
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => {
                        match self.read_place(fiber, &place).map_err(map_vm_error)? {
                            VmValue::Integer(value) => value.to_string(),
                            VmValue::String(value) => value,
                            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => {
                                return Err(StepError::new(
                                    VmFaultCode::TypeMismatch,
                                    "a place cannot contain another place",
                                ));
                            }
                        }
                    }
                };
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::String(value));
            }
            Opcode::Pop => {
                pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
            }
            Opcode::Dup => {
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let value = stack.last().cloned().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "dup underflows the stack")
                })?;
                stack.push(value);
            }
            Opcode::StorePlace => {
                let place = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                let place = match place {
                    VmValue::IntegerPlace(place) if matches!(value, VmValue::Integer(_)) => place,
                    VmValue::StringPlace(place) if matches!(value, VmValue::String(_)) => place,
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "indirect store place and value types differ",
                        ));
                    }
                };
                self.write_place(fiber, &place, value)
                    .map_err(map_vm_error)?;
            }
            Opcode::Concat => {
                let count = read_u16(&position.encoded.payload, 0)? as usize;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let mut parts = Vec::with_capacity(count);
                for _ in 0..count {
                    let VmValue::String(part) = pop(stack)? else {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "concat expects strings",
                        ));
                    };
                    parts.push(part);
                }
                parts.reverse();
                stack.push(VmValue::String(parts.concat()));
            }
            Opcode::Jump | Opcode::JumpIfFalse => {
                let target = read_u32(&position.encoded.payload, 0)? as usize;
                let take = if opcode == Opcode::JumpIfFalse {
                    let VmValue::Integer(condition) =
                        pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                    else {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "conditional jump expects an integer",
                        ));
                    };
                    condition == 0
                } else {
                    true
                };
                if take {
                    if target <= position.instruction {
                        fiber.backward_branches_without_progress =
                            fiber.backward_branches_without_progress.saturating_add(1);
                    }
                    fiber.frames.last_mut().expect("frame exists").instruction = target;
                }
            }
            Opcode::ResolveFunction => {
                let missing_target = read_u32(&position.encoded.payload, 0)? as usize;
                let allow_missing = position.encoded.payload.get(4).copied() == Some(1);
                let method = position.encoded.payload.get(5).copied() == Some(1);
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "dynamic function target must be a string",
                    ));
                };
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                if let Some(target) = artifact
                    .functions
                    .iter()
                    .find(|function| function.name.eq_ignore_ascii_case(&name))
                {
                    let kind_matches = if method {
                        target.kind == BytecodeFunctionKind::Method
                    } else {
                        target.kind != BytecodeFunctionKind::Method
                            && (target.kind != BytecodeFunctionKind::Event
                                || artifact.call_compatibility.allow_event_as_normal)
                    };
                    if !kind_matches {
                        if method && allow_missing {
                            fiber
                                .frames
                                .last_mut()
                                .expect("frame exists")
                                .stack
                                .push(VmValue::String(String::new()));
                            fiber.frames.last_mut().expect("frame exists").instruction =
                                missing_target;
                            return Ok(StepOutcome::Continue);
                        }
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            format!("dynamic target {name} has an incompatible function kind"),
                        ));
                    }
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(VmValue::String(target.name.clone()));
                } else if allow_missing {
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(VmValue::String(String::new()));
                    fiber.frames.last_mut().expect("frame exists").instruction = missing_target;
                } else {
                    return Err(StepError::new(
                        VmFaultCode::MissingSymbol,
                        format!("dynamic function {name} is missing"),
                    ));
                }
            }
            Opcode::InvokeDynamic => {
                let argument_count = read_u16(&position.encoded.payload, 0)? as usize;
                let tail = position.encoded.payload.get(2).copied() == Some(1);
                let new_frame = self.allocate_frame_id();
                let arguments = pop_arguments(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    argument_count,
                )?;
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "resolved function target must be a string",
                    ));
                };
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                let target = artifact
                    .functions
                    .iter()
                    .find(|function| function.name.eq_ignore_ascii_case(&name))
                    .cloned()
                    .ok_or_else(|| {
                        StepError::new(VmFaultCode::MissingSymbol, "resolved function disappeared")
                    })?;
                let arguments =
                    prepare_dynamic_arguments(&target, arguments, artifact.call_compatibility)
                        .map_err(map_vm_error)?;
                bind_persistent_arguments(
                    &mut self.memory,
                    position.generation,
                    &target,
                    artifact,
                    &arguments,
                )
                .map_err(map_vm_error)?;
                let event_context = fiber.frames.last().expect("frame exists").event_context
                    || target.kind == BytecodeFunctionKind::Event;
                let frame = make_frame(
                    new_frame,
                    position.generation,
                    &target,
                    artifact,
                    arguments,
                    false,
                    event_context,
                );
                if tail {
                    *fiber.frames.last_mut().expect("frame exists") = frame;
                } else {
                    if fiber.frames.len() >= self.config.maximum_call_depth {
                        return Err(StepError::new(
                            VmFaultCode::ResourceLimit,
                            "maximum call depth exceeded",
                        ));
                    }
                    fiber.frames.push(frame);
                }
            }
            Opcode::Call | Opcode::CallNative | Opcode::CallHost => {
                let import_index = read_u32(&position.encoded.payload, 0)? as usize;
                let argument_count = read_u16(&position.encoded.payload, 4)? as usize;
                let new_frame = (opcode == Opcode::Call).then(|| self.allocate_frame_id());
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                let function = artifact
                    .functions
                    .iter()
                    .find(|function| function.key == position.function)
                    .expect("validated function exists");
                let import = function.imports.get(import_index).cloned().ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "call import is missing")
                })?;
                let arguments = pop_arguments(
                    &mut fiber.frames.last_mut().expect("frame exists").stack,
                    argument_count,
                )?;
                match (opcode, import.kind) {
                    (Opcode::Call, ImportKind::Function) => {
                        if fiber.frames.len() >= self.config.maximum_call_depth {
                            return Err(StepError::new(
                                VmFaultCode::ResourceLimit,
                                "maximum call depth exceeded",
                            ));
                        }
                        let target = artifact
                            .functions
                            .iter()
                            .find(|function| function.key == import.key)
                            .cloned()
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "called function is missing",
                                )
                            })?;
                        if target.kind == BytecodeFunctionKind::Event
                            && !artifact.call_compatibility.allow_event_as_normal
                        {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!(
                                    "event function {} cannot be called as an ordinary function",
                                    target.name
                                ),
                            ));
                        }
                        validate_arguments(&target, &arguments).map_err(map_vm_error)?;
                        bind_persistent_arguments(
                            &mut self.memory,
                            position.generation,
                            &target,
                            artifact,
                            &arguments,
                        )
                        .map_err(map_vm_error)?;
                        fiber.frames.push(make_frame(
                            new_frame.expect("function call reserved a frame id"),
                            position.generation,
                            &target,
                            artifact,
                            arguments,
                            target.result.is_some(),
                            fiber
                                .frames
                                .last()
                                .expect("caller frame exists")
                                .event_context
                                || target.kind == BytecodeFunctionKind::Event,
                        ));
                    }
                    (Opcode::CallNative, ImportKind::Native) => {
                        let target = artifact
                            .native_imports
                            .iter()
                            .find(|candidate| candidate.import.key == import.key)
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?
                            .import
                            .clone();
                        let result_type = target.result;
                        let native_name = target.name.to_ascii_lowercase();
                        let mut rollback = None;
                        let ready = if matches!(native_name.as_str(), "initrand" | "dumprand") {
                            execute_random_place_transaction(
                                &mut self.memory,
                                position.generation,
                                artifact,
                                natives,
                                &native_name,
                            )
                            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
                            NativeReady::default()
                        } else if matches!(native_name.as_str(), "swap" | "swapvar") {
                            execute_swap_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(
                            native_name.as_str(),
                            "setbit" | "clearbit" | "invertbit"
                        ) {
                            execute_bit_mutation(self, fiber, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "split" {
                            execute_split_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "getnum" {
                            NativeReady::value(
                                execute_getnum(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if native_name == "strjoin" {
                            NativeReady::value(
                                execute_strjoin(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name.as_str(),
                            "arrayremove" | "arrayshift" | "arraysort"
                        ) {
                            execute_array_mutation(self, fiber, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "arraycopy" {
                            execute_array_copy(self, fiber, &arguments).map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(native_name.as_str(), "varset" | "cvarset") {
                            execute_variable_fill(self, fiber, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "arraymsort" {
                            NativeReady::value(
                                execute_array_multi_sort(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "arraymsortex" {
                            NativeReady::value(
                                execute_array_multi_sort_ex(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name.as_str(), "findelement" | "findlastelement")
                        {
                            NativeReady::value(
                                execute_find_element(
                                    self,
                                    fiber,
                                    native_name == "findlastelement",
                                    &arguments,
                                )
                                .map_err(map_vm_error)?,
                            )
                        } else if native_name == "regexpmatch" {
                            NativeReady::value(
                                execute_regex_match(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name.as_str(),
                            "sumarray"
                                | "sumcarray"
                                | "maxarray"
                                | "maxcarray"
                                | "minarray"
                                | "mincarray"
                                | "match"
                                | "cmatch"
                                | "inrangearray"
                                | "inrangecarray"
                                | "groupmatch"
                                | "nosames"
                                | "allsames"
                        ) {
                            NativeReady::value(
                                execute_array_query(self, fiber, &native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name.as_str(),
                            "charanum"
                                | "getchara"
                                | "getspchara"
                                | "existcsv"
                                | "csvname"
                                | "csvcallname"
                                | "csvnickname"
                                | "csvmastername"
                                | "csvcstr"
                                | "csvbase"
                                | "csvabl"
                                | "csvmark"
                                | "csvexp"
                                | "csvrelation"
                                | "csvtalent"
                                | "csvcflag"
                                | "csvequip"
                                | "csvjuel"
                                | "findchara"
                                | "findlastchara"
                        ) {
                            NativeReady::value(
                                execute_character_query(self, fiber, &native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name.as_str(),
                            "addchara"
                                | "addspchara"
                                | "adddefchara"
                                | "addvoidchara"
                                | "delchara"
                                | "delallchara"
                                | "swapchara"
                                | "copychara"
                                | "addcopychara"
                                | "pickupchara"
                                | "sortchara"
                                | "reset_stain"
                        ) {
                            execute_character_mutation(self, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else {
                            let places = native_place_views(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            let implicit_places =
                                native_implicit_place_views(self, fiber).map_err(map_vm_error)?;
                            rollback = natives
                                .checkpoint(import.key)
                                .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
                            natives
                                .call(
                                    import.key,
                                    NativeCallRequest {
                                        import: target,
                                        arguments,
                                        places,
                                        implicit_places,
                                    },
                                )
                                .map_err(|error| StepError::new(VmFaultCode::Native, error))?
                        };
                        let result = validate_native_ready(self, fiber, result_type, &ready)
                            .and_then(|()| {
                                self.apply_host_ready(
                                    fiber,
                                    result_type,
                                    HostReady {
                                        value: ready.value,
                                        writes: ready.writes,
                                    },
                                )
                            });
                        if let Err(error) = result {
                            if let Some(state) = rollback {
                                let _ = natives.rollback(import.key, &state);
                            }
                            return Err(map_vm_error(error));
                        }
                    }
                    (Opcode::CallHost, ImportKind::Host) => {
                        let target = artifact
                            .host_imports
                            .iter()
                            .find(|candidate| candidate.import.key == import.key)
                            .cloned()
                            .ok_or_else(|| {
                                StepError::new(VmFaultCode::MissingSymbol, "host import is missing")
                            })?;
                        let request = self.allocate_request_id();
                        *host_calls = host_calls.saturating_add(1);
                        let origin = self.execution_origin(position, &target.import.name);
                        match host.call(HostCallRequest {
                            id: request,
                            fiber: fiber.id,
                            import: target.import.clone(),
                            arguments,
                            origin: origin.clone(),
                        }) {
                            HostCallResult::Ready(ready) => self
                                .apply_host_ready(fiber, target.import.result, ready)
                                .map_err(map_vm_error)?,
                            HostCallResult::Pending {
                                stability,
                                rebind_payload,
                            } => {
                                if !target.effect.may_suspend {
                                    return Err(StepError::new(
                                        VmFaultCode::Host,
                                        "non-suspending host import returned pending",
                                    ));
                                }
                                if stability == HostWaitStability::StableInput
                                    && target.snapshot_capability
                                        != HostSnapshotCapability::StableWait
                                {
                                    return Err(StepError::new(
                                        VmFaultCode::Host,
                                        "host import reported a wait above its snapshot capability",
                                    ));
                                }
                                let result = target.import.result;
                                fiber.state = FiberState::WaitingHost(WaitingHost {
                                    request,
                                    import: target.import,
                                    result,
                                    stability,
                                    rebind_payload,
                                    origin: origin.clone(),
                                });
                                return Ok(StepOutcome::Blocked);
                            }
                            HostCallResult::Error(error) => {
                                return Err(StepError::new(VmFaultCode::Host, error));
                            }
                            HostCallResult::Deferred => {
                                let result = target.import.result;
                                fiber.state = FiberState::WaitingHost(WaitingHost {
                                    request,
                                    import: target.import,
                                    result,
                                    stability: HostWaitStability::Transient,
                                    rebind_payload: Vec::new(),
                                    origin,
                                });
                                return Ok(StepOutcome::Blocked);
                            }
                        }
                    }
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "call opcode and import kind differ",
                        ));
                    }
                }
            }
            Opcode::JumpDynamicLabel => {
                let missing_target = read_u32(&position.encoded.payload, 0)? as usize;
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "dynamic label target must be a string",
                    ));
                };
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                let function = artifact
                    .functions
                    .iter()
                    .find(|function| function.key == position.function)
                    .expect("validated function exists");
                fiber.frames.last_mut().expect("frame exists").instruction = function
                    .labels
                    .iter()
                    .find(|label| label.name.eq_ignore_ascii_case(&name))
                    .map_or(missing_target, |label| label.instruction as usize);
            }
            Opcode::InvokeEvent => {
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "event target must be a string",
                    ));
                };
                if fiber.frames.last().expect("frame exists").event_context {
                    return Err(StepError::new(
                        VmFaultCode::Trap,
                        "CALLEVENT is not allowed inside an event dispatch",
                    ));
                }
                let frame_id = self.allocate_frame_id();
                let artifact = &self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists")
                    .artifact;
                let Some(group) = artifact
                    .event_groups
                    .iter()
                    .find(|group| group.name.eq_ignore_ascii_case(&name))
                else {
                    if artifact.functions.iter().any(|function| {
                        function.name.eq_ignore_ascii_case(&name)
                            && function.kind != BytecodeFunctionKind::Event
                    }) {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            format!("CALLEVENT target {name} is not an event"),
                        ));
                    }
                    return Ok(StepOutcome::Continue);
                };
                let mut pending = std::collections::VecDeque::new();
                let groups: &[(&[erabasic_bytecode::BytecodeEventEntry], u8)] =
                    if group.only.is_empty() {
                        &[(&group.priority, 1), (&group.normal, 2), (&group.later, 3)]
                    } else {
                        &[(&group.only, 0)]
                    };
                for (entries, group_id) in groups {
                    pending.extend(entries.iter().map(|entry| EventDispatchEntry {
                        function: entry.function,
                        single: entry.single,
                        group: *group_id,
                    }));
                }
                let Some(active) = pending.pop_front() else {
                    return Ok(StepOutcome::Continue);
                };
                if fiber.frames.len() >= self.config.maximum_call_depth {
                    return Err(StepError::new(
                        VmFaultCode::ResourceLimit,
                        "maximum call depth exceeded",
                    ));
                }
                let target = artifact
                    .functions
                    .iter()
                    .find(|function| function.key == active.function)
                    .cloned()
                    .ok_or_else(|| {
                        StepError::new(VmFaultCode::MissingSymbol, "event function is missing")
                    })?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .event_dispatch = Some(EventDispatch { active, pending });
                fiber.frames.push(make_frame(
                    frame_id,
                    position.generation,
                    &target,
                    artifact,
                    Vec::new(),
                    false,
                    true,
                ));
            }
            Opcode::Return => {
                let has_value = position.encoded.payload.first().copied().unwrap_or(0) != 0;
                let value = has_value
                    .then(|| pop(&mut fiber.frames.last_mut().expect("frame exists").stack))
                    .transpose()?;
                let returned_frame = fiber.frames.pop().expect("returning frame exists");
                if let Some(caller) = fiber.frames.last_mut() {
                    if returned_frame.return_value_to_caller
                        && let Some(value) = value.clone()
                    {
                        caller.stack.push(value);
                    }
                    let next_event = caller.event_dispatch.as_mut().and_then(|dispatch| {
                        if dispatch.active.single && value == Some(VmValue::Integer(1)) {
                            while dispatch
                                .pending
                                .front()
                                .is_some_and(|entry| entry.group == dispatch.active.group)
                            {
                                dispatch.pending.pop_front();
                            }
                        }
                        dispatch.pending.pop_front().inspect(|next| {
                            dispatch.active = next.clone();
                        })
                    });
                    if let Some(next) = next_event {
                        let generation = caller.generation;
                        let frame_id = self.allocate_frame_id();
                        let artifact = &self
                            .generations
                            .get(&generation)
                            .expect("validated frame generation exists")
                            .artifact;
                        let target = artifact
                            .functions
                            .iter()
                            .find(|function| function.key == next.function)
                            .cloned()
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "event function is missing",
                                )
                            })?;
                        fiber.frames.push(make_frame(
                            frame_id,
                            generation,
                            &target,
                            artifact,
                            Vec::new(),
                            false,
                            true,
                        ));
                    } else if caller.event_dispatch.is_some() {
                        caller.event_dispatch = None;
                    }
                } else {
                    fiber.state = FiberState::Completed(value.clone());
                    return Ok(StepOutcome::Completed(value));
                }
            }
            Opcode::Yield => return Ok(StepOutcome::Yielded),
            Opcode::AwaitResume => {
                let tag = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "missing resume type")
                })?;
                let expected = opcode::decode_type(tag).ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "invalid resume type")
                })?;
                fiber.state = FiberState::WaitingResume(expected);
                return Ok(StepOutcome::Blocked);
            }
            Opcode::Trap => {
                let message = String::from_utf8_lossy(&position.encoded.payload);
                return Err(StepError::new(VmFaultCode::Trap, message));
            }
        }
        let stack_len = fiber.frames.last().map_or(0, |frame| frame.stack.len());
        if stack_len > self.config.maximum_operand_stack {
            return Err(StepError::new(
                VmFaultCode::ResourceLimit,
                "maximum operand stack exceeded",
            ));
        }
        Ok(StepOutcome::Continue)
    }

    fn make_fault(
        &self,
        fiber: FiberId,
        position: &InstructionPosition,
        code: VmFaultCode,
        message: impl Into<String>,
    ) -> VmFault {
        let command = self.command_for_position(position);
        let origin = self.execution_origin(position, &command);
        VmFault {
            code,
            message: message.into(),
            fiber,
            generation: position.generation,
            function: position.function,
            function_name: origin.function_name,
            instruction: u32::try_from(position.instruction).unwrap_or(u32::MAX),
            command,
            source: origin.source,
        }
    }

    fn execution_origin(
        &self,
        position: &InstructionPosition,
        command: &str,
    ) -> crate::VmExecutionOrigin {
        let generation = self.generations.get(&position.generation);
        let function = generation.and_then(|generation| {
            generation
                .artifact
                .functions
                .iter()
                .find(|function| function.key == position.function)
        });
        let source = generation.zip(function).and_then(|(generation, function)| {
            let offset = function
                .code
                .iter()
                .take(position.instruction)
                .map(EncodedInstruction::encoded_len)
                .sum();
            generation
                .artifact
                .source_map
                .resolve(position.function, offset)
        });
        crate::VmExecutionOrigin {
            generation: position.generation,
            function: position.function,
            function_name: function.map_or_else(String::new, |value| value.name.clone()),
            instruction: u32::try_from(position.instruction).unwrap_or(u32::MAX),
            command: command.to_owned(),
            source,
        }
    }

    fn command_for_position(&self, position: &InstructionPosition) -> String {
        let Ok(opcode) = Opcode::try_from(position.encoded.opcode) else {
            return format!("opcode:{}", position.encoded.opcode);
        };
        if matches!(opcode, Opcode::CallHost | Opcode::CallNative)
            && position.encoded.payload.len() >= 16
        {
            let mut bytes = [0; 16];
            bytes.copy_from_slice(&position.encoded.payload[..16]);
            let key = SymbolKey(bytes);
            if let Some(generation) = self.generations.get(&position.generation) {
                let name = generation
                    .artifact
                    .host_imports
                    .iter()
                    .map(|value| &value.import)
                    .chain(
                        generation
                            .artifact
                            .native_imports
                            .iter()
                            .map(|value| &value.import),
                    )
                    .find(|import| import.key == key)
                    .map(|import| import.name.clone());
                if let Some(name) = name {
                    return name;
                }
            }
        }
        format!("{opcode:?}")
    }
}
