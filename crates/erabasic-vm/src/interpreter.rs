use erabasic_bytecode::{
    BytecodeStorage, BytecodeType, EncodedInstruction, HostSnapshotCapability, ImportKind, Opcode,
    SymbolKey, opcode,
};

use crate::{
    Fiber, FiberId, FiberState, HostCallRequest, HostCallResult, HostWaitStability,
    NativeCallRequest, NativeServiceRegistry, PlaceDescriptor, RunBudget, Vm, VmError, VmEvent,
    VmFault, VmFaultCode, VmHost, VmRunReport, VmRunStop, VmValue, WaitingHost, find_global,
    make_frame, validate_arguments,
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
        let quantum = budget.fiber_quantum.max(1);
        let mut budget_exhausted = false;
        while let Some(fiber_id) = self.runnable.pop_front() {
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
                    Ok(StepOutcome::Continue) => {}
                    Ok(StepOutcome::Yielded) => {
                        fiber.mark_progress();
                        yielded = true;
                        report
                            .events
                            .push(VmEvent::FiberYielded { fiber: fiber.id });
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
                        break;
                    }
                    Ok(StepOutcome::Completed(value)) => {
                        report.events.push(VmEvent::FiberCompleted {
                            fiber: fiber.id,
                            value,
                        });
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
            if budget_exhausted {
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
                        validate_arguments(&target, &arguments).map_err(map_vm_error)?;
                        fiber.frames.push(make_frame(
                            new_frame.expect("function call reserved a frame id"),
                            position.generation,
                            &target,
                            artifact,
                            arguments,
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
                        let value = if matches!(native_name.as_str(), "initrand" | "dumprand") {
                            execute_random_place_transaction(
                                &mut self.memory,
                                position.generation,
                                artifact,
                                natives,
                                &native_name,
                            )
                            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
                            None
                        } else if matches!(native_name.as_str(), "swap" | "swapvar") {
                            execute_swap_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            None
                        } else if matches!(
                            native_name.as_str(),
                            "arrayremove" | "arrayshift" | "arraysort"
                        ) {
                            execute_array_mutation(self, fiber, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            None
                        } else if native_name == "arraycopy" {
                            execute_array_copy(self, fiber, &arguments).map_err(map_vm_error)?;
                            None
                        } else if matches!(native_name.as_str(), "findelement" | "findlastelement")
                        {
                            Some(
                                execute_find_element(
                                    self,
                                    fiber,
                                    native_name == "findlastelement",
                                    &arguments,
                                )
                                .map_err(map_vm_error)?,
                            )
                        } else if native_name == "regexpmatch" {
                            Some(
                                execute_regex_match(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else {
                            natives
                                .call(
                                    import.key,
                                    NativeCallRequest {
                                        import: target,
                                        arguments,
                                    },
                                )
                                .map_err(|error| StepError::new(VmFaultCode::Native, error))?
                        };
                        push_call_result(
                            &mut fiber.frames.last_mut().expect("frame exists").stack,
                            result_type,
                            value,
                            "native",
                        )?;
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
                        match host.call(HostCallRequest {
                            id: request,
                            fiber: fiber.id,
                            import: target.import.clone(),
                            arguments,
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
            Opcode::Return => {
                let has_value = position.encoded.payload.first().copied().unwrap_or(0) != 0;
                let value = has_value
                    .then(|| pop(&mut fiber.frames.last_mut().expect("frame exists").stack))
                    .transpose()?;
                fiber.frames.pop();
                if let Some(caller) = fiber.frames.last_mut() {
                    if let Some(value) = value {
                        caller.stack.push(value);
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
        let source = self
            .generations
            .get(&position.generation)
            .and_then(|generation| {
                let function = generation
                    .artifact
                    .functions
                    .iter()
                    .find(|function| function.key == position.function)?;
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
        VmFault {
            code,
            message: message.into(),
            fiber,
            generation: position.generation,
            function: position.function,
            instruction: u32::try_from(position.instruction).unwrap_or(u32::MAX),
            source,
        }
    }
}

fn execute_swap_transaction(
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

fn array_place(arguments: &[VmValue]) -> Result<&PlaceDescriptor, VmError> {
    match arguments.first() {
        Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) => Ok(place),
        _ => Err(VmError::InvalidArguments(
            "array operation requires a variable reference".into(),
        )),
    }
}

fn integer_argument(arguments: &[VmValue], index: usize) -> Result<i64, VmError> {
    match arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(VmError::InvalidArguments(format!(
            "argument {} must be integer",
            index + 1
        ))),
    }
}

fn array_snapshot(
    vm: &Vm,
    fiber: &Fiber,
    place: &PlaceDescriptor,
) -> Result<Vec<VmValue>, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let artifact = &vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("array generation is missing".into()))?
        .artifact;
    let definition = artifact
        .globals
        .iter()
        .find(|definition| definition.key == place.variable)
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if definition.dimensions.len() != 1 || !place.indices.is_empty() {
        return Err(VmError::InvalidArguments(
            "array operation requires an unindexed one-dimensional variable".into(),
        ));
    }
    let length = usize::try_from(definition.dimensions[0])
        .map_err(|_| VmError::InvalidState("array length exceeds this platform".into()))?;
    (0..length)
        .map(|index| {
            let mut element = place.clone();
            element.indices = vec![index as u64];
            vm.read_place(fiber, &element)
        })
        .collect()
}

fn commit_array(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
    // Every element was read successfully before this commit, so all addresses
    // and types have already been validated.
    for (index, value) in values.into_iter().enumerate() {
        let mut element = place.clone();
        element.indices = vec![index as u64];
        vm.write_place(fiber, &element, value)?;
    }
    Ok(())
}

fn execute_array_mutation(
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

fn execute_find_element(
    vm: &Vm,
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
    let matched = |value: &VmValue| -> Result<bool, VmError> {
        match (value, needle) {
            (VmValue::Integer(value), VmValue::Integer(needle)) => Ok(value == needle),
            (VmValue::String(value), VmValue::String(needle)) => {
                let regex =
                    crate::regex_compat::compile(needle).map_err(VmError::InvalidArguments)?;
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

fn execute_regex_match(
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
    let regex = crate::regex_compat::compile(pattern).map_err(VmError::InvalidArguments)?;
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
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
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
                    VmValue::Integer(i64::try_from(regex.captures_len()).unwrap_or(i64::MAX)),
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
                VmValue::IntegerPlace(place) => place.clone(),
                _ => {
                    return Err(VmError::InvalidArguments(
                        "REGEXPMATCH group-count output must be an integer place".into(),
                    ));
                }
            };
            let values = match &arguments[3] {
                VmValue::StringPlace(place) => place.clone(),
                _ => {
                    return Err(VmError::InvalidArguments(
                        "REGEXPMATCH capture output must be a string-array place".into(),
                    ));
                }
            };
            let mut writes = vec![(
                group_count,
                VmValue::Integer(i64::try_from(regex.captures_len()).unwrap_or(i64::MAX)),
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

fn global_unindexed_place(vm: &Vm, fiber: &Fiber, name: &str) -> Result<PlaceDescriptor, VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let variable = vm
        .generations
        .get(&generation)
        .and_then(|generation| {
            generation
                .artifact
                .globals
                .iter()
                .find(|definition| definition.name.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| VmError::InvalidState(format!("{name} is not defined")))?;
    Ok(PlaceDescriptor {
        variable: variable.key,
        ..PlaceDescriptor::default()
    })
}

fn indexed_place(place: &PlaceDescriptor, index: usize) -> PlaceDescriptor {
    let mut result = place.clone();
    result.indices = vec![u64::try_from(index).unwrap_or(u64::MAX)];
    result
}

fn commit_place_writes(
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

fn execute_array_copy(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let (source, source_type) = array_copy_place(arguments.first(), "source")?;
    let (destination, destination_type) = array_copy_place(arguments.get(1), "destination")?;
    if source_type != destination_type {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY array types differ".into(),
        ));
    }
    let source_values = array_snapshot_any_rank(vm, fiber, &source)?;
    let destination_values = array_snapshot_any_rank(vm, fiber, &destination)?;
    if source_values.len() != destination_values.len() {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY dimensions differ".into(),
        ));
    }
    commit_array_any_rank(vm, fiber, &destination, source_values)
}

fn array_copy_place(
    value: Option<&VmValue>,
    role: &str,
) -> Result<(PlaceDescriptor, BytecodeType), VmError> {
    match value {
        Some(VmValue::IntegerPlace(place)) => Ok((place.clone(), BytecodeType::Integer)),
        Some(VmValue::StringPlace(place)) => Ok((place.clone(), BytecodeType::String)),
        _ => Err(VmError::InvalidArguments(format!(
            "ARRAYCOPY {role} must be an array place"
        ))),
    }
}

fn array_snapshot_any_rank(
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
        .and_then(|generation| {
            generation
                .artifact
                .globals
                .iter()
                .find(|definition| definition.key == place.variable)
        })
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    if !(1..=3).contains(&definition.dimensions.len()) {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY requires a one to three dimensional array".into(),
        ));
    }
    let dimensions = definition
        .dimensions
        .iter()
        .map(|dimension| {
            usize::try_from(*dimension)
                .map_err(|_| VmError::InvalidState("array dimension exceeds this platform".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = dimensions
        .iter()
        .try_fold(1usize, |count, dimension| count.checked_mul(*dimension))
        .ok_or_else(|| VmError::InvalidState("array element count overflows".into()))?;
    (0..count)
        .map(|flat| {
            let mut element = place.clone();
            element.indices = unflatten_indices(flat, &dimensions);
            vm.read_place(fiber, &element)
        })
        .collect()
}

fn commit_array_any_rank(
    vm: &mut Vm,
    fiber: &mut Fiber,
    place: &PlaceDescriptor,
    values: Vec<VmValue>,
) -> Result<(), VmError> {
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
        .ok_or_else(|| VmError::InvalidState("array variable is missing".into()))?;
    let dimensions = definition
        .dimensions
        .iter()
        .map(|dimension| {
            usize::try_from(*dimension)
                .map_err(|_| VmError::InvalidState("array dimension exceeds this platform".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (flat, value) in values.into_iter().enumerate() {
        let mut element = place.clone();
        element.indices = unflatten_indices(flat, &dimensions);
        vm.write_place(fiber, &element, value)?;
    }
    Ok(())
}

fn unflatten_indices(mut flat: usize, dimensions: &[usize]) -> Vec<u64> {
    let mut indices = vec![0; dimensions.len()];
    for dimension in (0..dimensions.len()).rev() {
        indices[dimension] = u64::try_from(flat % dimensions[dimension]).unwrap_or(u64::MAX);
        flat /= dimensions[dimension];
    }
    indices
}

fn execute_random_place_transaction(
    memory: &mut crate::Memory,
    generation: crate::GenerationId,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    natives: &mut NativeServiceRegistry,
    operation: &str,
) -> Result<(), String> {
    let definition = artifact
        .globals
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case("RANDDATA"))
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
            .values
            .iter()
            .map(|value| match value {
                VmValue::Integer(value) => Ok(*value),
                _ => Err("RANDDATA contains a non-integer value".to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Native state is only replaced after the entire array and index validate.
        natives.set_random_values(&values)
    } else {
        let values = natives.random_values()?;
        let cell = memory
            .cell_mut(generation, definition, 0)
            .ok_or_else(|| "RANDDATA storage is unavailable".to_owned())?;
        if cell.values.len() != values.len() {
            return Err("RANDDATA storage changed during DUMPRAND".into());
        }
        // Every target slot was validated above, so this commit cannot partially fail.
        for (target, value) in cell.values.iter_mut().zip(values) {
            *target = VmValue::Integer(value);
        }
        Ok(())
    }
}

fn exact<const N: usize>(payload: &[u8]) -> Result<[u8; N], StepError> {
    payload.try_into().map_err(|_| {
        StepError::new(
            VmFaultCode::InvalidInstruction,
            format!("expected {N} operand bytes, found {}", payload.len()),
        )
    })
}

fn read_u16(payload: &[u8], offset: usize) -> Result<u16, StepError> {
    Ok(u16::from_le_bytes(exact_slice(payload, offset)?))
}

fn read_u32(payload: &[u8], offset: usize) -> Result<u32, StepError> {
    Ok(u32::from_le_bytes(exact_slice(payload, offset)?))
}

fn exact_slice<const N: usize>(payload: &[u8], offset: usize) -> Result<[u8; N], StepError> {
    payload
        .get(offset..offset + N)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "truncated operand"))
}

fn read_key(payload: &[u8]) -> Result<SymbolKey, StepError> {
    Ok(SymbolKey(exact_slice(payload, 0)?))
}

fn pop(stack: &mut Vec<VmValue>) -> Result<VmValue, StepError> {
    stack
        .pop()
        .ok_or_else(|| StepError::new(VmFaultCode::StackUnderflow, "operand stack underflow"))
}

fn pop_indices(stack: &mut Vec<VmValue>, count: usize) -> Result<Vec<u64>, StepError> {
    let mut indices = Vec::with_capacity(count);
    for _ in 0..count {
        let VmValue::Integer(value) = pop(stack)? else {
            return Err(StepError::new(
                VmFaultCode::TypeMismatch,
                "variable indices must be integers",
            ));
        };
        indices.push(u64::try_from(value).map_err(|_| {
            StepError::new(VmFaultCode::Bounds, "variable index cannot be negative")
        })?);
    }
    indices.reverse();
    Ok(indices)
}

fn pop_arguments(stack: &mut Vec<VmValue>, count: usize) -> Result<Vec<VmValue>, StepError> {
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        arguments.push(pop(stack)?);
    }
    arguments.reverse();
    Ok(arguments)
}

fn push_call_result(
    stack: &mut Vec<VmValue>,
    expected: Option<BytecodeType>,
    value: Option<VmValue>,
    source: &str,
) -> Result<(), StepError> {
    match (expected, value) {
        (None, None) => Ok(()),
        (Some(expected), Some(value)) if value.value_type() == expected => {
            stack.push(value);
            Ok(())
        }
        (expected, value) => Err(StepError::new(
            VmFaultCode::TypeMismatch,
            format!(
                "{source} result mismatch: expected {expected:?}, found {:?}",
                value.as_ref().map(VmValue::value_type)
            ),
        )),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn unary_value(operation: u8, value: VmValue) -> Result<VmValue, StepError> {
    let VmValue::Integer(value) = value else {
        return Err(StepError::new(
            VmFaultCode::TypeMismatch,
            "unary operation expects an integer",
        ));
    };
    Ok(VmValue::Integer(match operation {
        0 => value,
        1 => value.wrapping_neg(),
        2 => i64::from(value == 0),
        3 => !value,
        4 | 6 => value.wrapping_add(1),
        5 | 7 => value.wrapping_sub(1),
        _ => {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "unknown unary operation",
            ));
        }
    }))
}

#[allow(clippy::too_many_lines)]
fn binary_value(operation: u8, left: VmValue, right: VmValue) -> Result<VmValue, StepError> {
    match (left, right) {
        (VmValue::Integer(left), VmValue::Integer(right)) => {
            let value = match operation {
                0 => left.wrapping_mul(right),
                1 => left.checked_div(right).ok_or_else(|| {
                    StepError::new(
                        if right == 0 {
                            VmFaultCode::DivideByZero
                        } else {
                            VmFaultCode::InvalidInstruction
                        },
                        "integer division failed",
                    )
                })?,
                2 => left.checked_rem(right).ok_or_else(|| {
                    StepError::new(VmFaultCode::DivideByZero, "integer remainder failed")
                })?,
                3 => left.wrapping_add(right),
                4 => left.wrapping_sub(right),
                5 => left.wrapping_shl(u32::try_from(right & 63).unwrap_or(0)),
                6 => left.wrapping_shr(u32::try_from(right & 63).unwrap_or(0)),
                7 => i64::from(left < right),
                8 => i64::from(left <= right),
                9 => i64::from(left > right),
                10 => i64::from(left >= right),
                11 => i64::from(left == right),
                12 => i64::from(left != right),
                13 => left & right,
                14 => left ^ right,
                15 => left | right,
                16 => i64::from(left != 0 && right != 0),
                17 => i64::from((left != 0) ^ (right != 0)),
                18 => i64::from(left != 0 || right != 0),
                19 => i64::from(!(left != 0 && right != 0)),
                20 => i64::from(!(left != 0 || right != 0)),
                _ => {
                    return Err(StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "unknown binary operation",
                    ));
                }
            };
            Ok(VmValue::Integer(value))
        }
        (VmValue::String(left), VmValue::String(right)) => Ok(match operation {
            3 => VmValue::String(left + &right),
            7 => VmValue::Integer(i64::from(left < right)),
            8 => VmValue::Integer(i64::from(left <= right)),
            9 => VmValue::Integer(i64::from(left > right)),
            10 => VmValue::Integer(i64::from(left >= right)),
            11 => VmValue::Integer(i64::from(left == right)),
            12 => VmValue::Integer(i64::from(left != right)),
            _ => {
                return Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "binary operation is not defined for strings",
                ));
            }
        }),
        _ => Err(StepError::new(
            VmFaultCode::TypeMismatch,
            "binary operands have different types",
        )),
    }
}

fn assign_binary_tag(operation: u8) -> Result<u8, StepError> {
    Ok(match operation {
        1 => 3,
        2 => 4,
        3 => 0,
        4 => 1,
        5 => 2,
        6 => 13,
        7 => 15,
        8 => 14,
        9 => 5,
        10 => 6,
        _ => {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "unknown assignment operation",
            ));
        }
    })
}

#[allow(clippy::needless_pass_by_value)]
fn map_vm_error(error: VmError) -> StepError {
    let code = match error {
        VmError::InvalidArguments(_) => VmFaultCode::TypeMismatch,
        VmError::ResourceLimit(_) => VmFaultCode::ResourceLimit,
        VmError::MissingFunction(_) => VmFaultCode::MissingSymbol,
        _ => VmFaultCode::Bounds,
    };
    StepError::new(code, error.to_string())
}
