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
            Opcode::LoadVariable | Opcode::StoreVariable => {
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
                if opcode == Opcode::LoadVariable {
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
                        let value = natives
                            .call(
                                import.key,
                                NativeCallRequest {
                                    import: target,
                                    arguments,
                                },
                            )
                            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
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
