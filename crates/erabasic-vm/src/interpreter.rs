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
                        } else if matches!(native_name.as_str(), "varset" | "cvarset") {
                            execute_variable_fill(self, fiber, &native_name, &arguments)
                                .map_err(map_vm_error)?;
                            None
                        } else if native_name == "arraymsort" {
                            Some(
                                execute_array_multi_sort(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "arraymsortex" {
                            Some(
                                execute_array_multi_sort_ex(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
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
                            Some(
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
                            Some(
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
                            None
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

#[allow(clippy::too_many_lines)]
fn execute_variable_fill(
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
        .and_then(|generation| {
            generation
                .artifact
                .globals
                .iter()
                .find(|definition| definition.key == place.variable)
        })
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
        let mut values = array_snapshot(vm, fiber, &place)?;
        let mut start = optional_nonnegative(arguments, 2, 0, "VARSET start")?;
        let mut end = optional_nonnegative(arguments, 3, values.len(), "VARSET end")?;
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        if end > values.len() {
            return Err(VmError::InvalidArguments("VARSET range is invalid".into()));
        }
        values[start..end].fill(value);
        return commit_array(vm, fiber, &place, values);
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
            ..place.clone()
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

fn optional_nonnegative(
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

#[allow(clippy::too_many_lines)]
fn execute_array_query(
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
    let character_range = operation.contains("carray");
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

fn optional_index(
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

fn character_series(
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
        return Err(VmError::InvalidArguments(
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

#[allow(clippy::too_many_lines)]
fn execute_character_query(
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
    if operation == "charanum" {
        return Ok(VmValue::Integer(
            i64::try_from(vm.memory.characters.len()).unwrap_or(i64::MAX),
        ));
    }
    if matches!(operation, "getchara" | "getspchara") {
        let number = integer_argument(arguments, 0)?;
        let requested_sp = operation == "getspchara"
            || matches!(arguments.get(1), Some(VmValue::Integer(value)) if *value != 0);
        let no = artifact
            .globals
            .iter()
            .find(|definition| definition.name.eq_ignore_ascii_case("NO"))
            .ok_or_else(|| VmError::InvalidState("NO is not defined".into()))?;
        let cflag = artifact
            .globals
            .iter()
            .find(|definition| definition.name.eq_ignore_ascii_case("CFLAG"));
        for (index, character) in vm.memory.characters.iter().enumerate() {
            let value = character.get(&no.key).and_then(|cell| cell.values.first());
            if value != Some(&VmValue::Integer(number)) {
                continue;
            }
            if operation == "getchara" && arguments.get(1).is_none() {
                return Ok(VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX)));
            }
            let is_sp = cflag
                .and_then(|definition| character.get(&definition.key))
                .and_then(|cell| cell.values.first())
                .is_some_and(|value| matches!(value, VmValue::Integer(value) if *value != 0));
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
            return Err(VmError::InvalidArguments(
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
        usize::try_from(integer_argument(arguments, 1)?)
            .map_err(|_| VmError::InvalidArguments("CSV field index is negative".into()))?
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
    let requested_sp =
        matches!(arguments.get(sp_argument), Some(VmValue::Integer(value)) if *value != 0);
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
        VmError::InvalidArguments(format!("character CSV number {number} does not exist"))
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
fn execute_character_mutation(
    vm: &mut Vm,
    operation: &str,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let artifact = vm.artifact().clone();
    let mut memory = vm.memory.clone();
    match operation {
        "addchara" | "addspchara" => {
            let requested_sp = operation == "addspchara";
            for argument in arguments {
                let VmValue::Integer(number) = argument else {
                    return Err(VmError::InvalidArguments(
                        "ADDCHARA arguments must be integers".into(),
                    ));
                };
                let template = artifact
                    .project_data
                    .static_data
                    .characters
                    .iter()
                    .find(|template| {
                        template.no == *number && template.is_sp_character == requested_sp
                    })
                    .ok_or_else(|| {
                        VmError::InvalidArguments(format!(
                            "character template {number} does not exist"
                        ))
                    })?;
                memory.push_character(&artifact, Some(template));
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
                memory.push_character(&artifact, template);
            }
        }
        "addvoidchara" => memory.push_character(&artifact, None),
        "delchara" => {
            let mut indices = arguments
                .iter()
                .map(|value| match value {
                    VmValue::Integer(value) => usize::try_from(*value).map_err(|_| {
                        VmError::InvalidArguments("DELCHARA index is negative".into())
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
                return Err(VmError::InvalidArguments(
                    "DELCHARA index is duplicated or out of range".into(),
                ));
            }
            for index in indices.into_iter().rev() {
                memory.characters.remove(index);
            }
        }
        "delallchara" => memory.characters.clear(),
        "swapchara" | "copychara" => {
            let left = usize::try_from(integer_argument(arguments, 0)?)
                .map_err(|_| VmError::InvalidArguments("character index is negative".into()))?;
            let right = usize::try_from(integer_argument(arguments, 1)?)
                .map_err(|_| VmError::InvalidArguments("character index is negative".into()))?;
            if left >= memory.characters.len() || right >= memory.characters.len() {
                return Err(VmError::InvalidArguments(
                    "character index is out of range".into(),
                ));
            }
            if operation == "swapchara" {
                memory.characters.swap(left, right);
            } else {
                memory.characters[right] = memory.characters[left].clone();
            }
        }
        "addcopychara" => {
            for argument in arguments {
                let VmValue::Integer(index) = argument else {
                    return Err(VmError::InvalidArguments(
                        "ADDCOPYCHARA arguments must be integers".into(),
                    ));
                };
                let index = usize::try_from(*index).map_err(|_| {
                    VmError::InvalidArguments("ADDCOPYCHARA index is negative".into())
                })?;
                let character = memory.characters.get(index).cloned().ok_or_else(|| {
                    VmError::InvalidArguments("ADDCOPYCHARA index is out of range".into())
                })?;
                memory.characters.push(character);
            }
        }
        "pickupchara" => pickup_characters(&artifact, &mut memory, arguments)?,
        "reset_stain" => {
            let character = usize::try_from(integer_argument(arguments, 0)?)
                .map_err(|_| VmError::InvalidArguments("RESET_STAIN index is negative".into()))?;
            if character >= memory.characters.len() {
                return Err(VmError::InvalidArguments(
                    "RESET_STAIN character index is out of range".into(),
                ));
            }
            let definition = artifact
                .globals
                .iter()
                .find(|definition| definition.name.eq_ignore_ascii_case("STAIN"))
                .ok_or_else(|| VmError::InvalidState("STAIN variable is missing".into()))?;
            let cell = memory
                .cell_mut(vm.current_generation, definition, character)
                .ok_or_else(|| VmError::InvalidState("STAIN storage is unavailable".into()))?;
            for (index, destination) in cell.values.iter_mut().enumerate() {
                *destination = VmValue::Integer(
                    artifact
                        .project_data
                        .static_data
                        .replace
                        .stain_default
                        .get(index)
                        .copied()
                        .unwrap_or(0),
                );
            }
        }
        "sortchara" => sort_characters(vm.current_generation, &artifact, &mut memory, arguments)?,
        _ => {
            return Err(VmError::InvalidArguments(
                "unknown character mutation".into(),
            ));
        }
    }
    // CHARANUM is exposed as a calculated variable by the language frontend, but
    // the VM stores calculated cells so normal bytecode loads stay inexpensive.
    // Refresh it in the same candidate memory image to keep the mutation atomic.
    let character_count = i64::try_from(memory.characters.len()).unwrap_or(i64::MAX);
    write_named_integer(&artifact, &mut memory, "CHARANUM", character_count)?;
    vm.memory = memory;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn sort_characters(
    generation: crate::GenerationId,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut crate::Memory,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    if memory.characters.len() <= 1 {
        return Ok(());
    }
    let (definition, indices, descending) = match arguments.first() {
        None => (
            artifact
                .globals
                .iter()
                .find(|definition| definition.name.eq_ignore_ascii_case("NO"))
                .ok_or_else(|| VmError::InvalidState("NO variable is missing".into()))?,
            Vec::new(),
            false,
        ),
        Some(VmValue::String(order))
            if order.eq_ignore_ascii_case("FORWARD") || order.eq_ignore_ascii_case("BACK") =>
        {
            (
                artifact
                    .globals
                    .iter()
                    .find(|definition| definition.name.eq_ignore_ascii_case("NO"))
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
                return Err(VmError::InvalidArguments(
                    "SORTCHARA key must be a character variable".into(),
                ));
            }
            let descending = matches!(arguments.get(1), Some(VmValue::String(value)) if value.eq_ignore_ascii_case("BACK"));
            (definition, place.indices.clone(), descending)
        }
        _ => {
            return Err(VmError::InvalidArguments(
                "SORTCHARA key or order is invalid".into(),
            ));
        }
    };
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
                .read(&indices)
                .map_err(VmError::InvalidState)?;
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
    let old = memory.characters.clone();
    let mut sorted = order
        .iter()
        .map(|(index, _)| old[*index].clone())
        .collect::<Vec<_>>();
    if let Some(master_index) = master_index {
        sorted.insert(master_index, old[master_index].clone());
    }
    memory.characters = sorted;
    let new_index = |old_index: i64| {
        usize::try_from(old_index).ok().and_then(|old_index| {
            if Some(old_index) == master_index {
                master_index
            } else {
                order
                    .iter()
                    .position(|(candidate, _)| *candidate == old_index)
                    .map(|position| {
                        position
                            + usize::from(master_index.is_some_and(|master| position >= master))
                    })
            }
        })
    };
    for (name, old_index) in [("TARGET", target), ("ASSI", assi)] {
        if let Some(index) = new_index(old_index) {
            write_named_integer(
                artifact,
                memory,
                name,
                i64::try_from(index).unwrap_or(i64::MAX),
            )?;
        }
    }
    Ok(())
}

fn pickup_characters(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut crate::Memory,
    arguments: &[VmValue],
) -> Result<(), VmError> {
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
            return Err(VmError::InvalidArguments(
                "PICKUPCHARA index is out of range".into(),
            ));
        }
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    let old_special = ["TARGET", "ASSI", "MASTER"]
        .map(|name| read_named_integer(artifact, memory, name).unwrap_or(-1));
    let characters = selected
        .iter()
        .map(|index| memory.characters[*index].clone())
        .collect();
    memory.characters = characters;
    for (name, old) in ["TARGET", "ASSI", "MASTER"].into_iter().zip(old_special) {
        let replacement = usize::try_from(old)
            .ok()
            .and_then(|old| selected.iter().position(|candidate| *candidate == old))
            .map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        write_named_integer(artifact, memory, name, replacement)?;
    }
    Ok(())
}

fn read_named_integer(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &crate::Memory,
    name: &str,
) -> Option<i64> {
    let definition = artifact
        .globals
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))?;
    match memory.shared.get(&definition.key)?.values.first()? {
        VmValue::Integer(value) => Some(*value),
        _ => None,
    }
}

fn write_named_integer(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    memory: &mut crate::Memory,
    name: &str,
    value: i64,
) -> Result<(), VmError> {
    let definition = artifact
        .globals
        .iter()
        .find(|definition| definition.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| VmError::InvalidState(format!("{name} is not defined")))?;
    let slot = memory
        .shared
        .get_mut(&definition.key)
        .and_then(|cell| cell.values.first_mut())
        .ok_or_else(|| VmError::InvalidState(format!("{name} storage is unavailable")))?;
    *slot = VmValue::Integer(value);
    Ok(())
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
    let (source, source_type, source_dimensions) =
        array_copy_place(vm, fiber, arguments.first(), "source", false)?;
    let (destination, destination_type, destination_dimensions) =
        array_copy_place(vm, fiber, arguments.get(1), "destination", true)?;
    if source_type != destination_type {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY array types differ".into(),
        ));
    }
    if source_dimensions != destination_dimensions {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY dimensions differ".into(),
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

fn execute_array_multi_sort(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if arguments.is_empty() {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORT requires at least one array".into(),
        ));
    }
    let mut arrays = Vec::with_capacity(arguments.len());
    for (index, argument) in arguments.iter().enumerate() {
        let place = match argument {
            VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => place.clone(),
            _ => {
                return Err(VmError::InvalidArguments(format!(
                    "ARRAYMSORT argument {} must be an array place",
                    index + 1
                )));
            }
        };
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
            .ok_or_else(|| VmError::InvalidState("ARRAYMSORT variable is missing".into()))?;
        if definition.storage == BytecodeStorage::Character
            || !definition.mutable
            || place.character.is_some()
            || !place.indices.is_empty()
            || !(1..=3).contains(&definition.dimensions.len())
            || (index == 0 && definition.dimensions.len() != 1)
        {
            return Err(VmError::InvalidArguments(format!(
                "ARRAYMSORT argument {} is not a mutable non-character array of the required rank",
                index + 1
            )));
        }
        let dimensions = definition.dimensions.clone();
        let values = array_snapshot_any_rank(vm, fiber, &place)?;
        arrays.push((place, dimensions, values));
    }

    let key_values = &arrays[0].2;
    let key_count = key_values
        .iter()
        .position(|value| {
            matches!(value, VmValue::Integer(0))
                || matches!(value, VmValue::String(value) if value.is_empty())
        })
        .unwrap_or(key_values.len());
    let mut order: Vec<usize> = (0..key_count).collect();
    order.sort_by(
        |left, right| match (&key_values[*left], &key_values[*right]) {
            (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
            (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
            _ => std::cmp::Ordering::Equal,
        },
    );

    // Validate every first dimension and build every candidate before the first write.
    let mut candidates = Vec::with_capacity(arrays.len());
    for (place, dimensions, values) in arrays {
        let first = usize::try_from(dimensions[0])
            .map_err(|_| VmError::InvalidState("ARRAYMSORT dimension is too large".into()))?;
        if first < key_count {
            return Ok(VmValue::Integer(0));
        }
        let row_width = values.len().checked_div(first).ok_or_else(|| {
            VmError::InvalidState("ARRAYMSORT array has an invalid first dimension".into())
        })?;
        let mut candidate = values.clone();
        for (destination, source) in order.iter().copied().enumerate() {
            let destination_start = destination * row_width;
            let source_start = source * row_width;
            candidate[destination_start..destination_start + row_width]
                .clone_from_slice(&values[source_start..source_start + row_width]);
        }
        candidates.push((place, candidate));
    }
    for (place, candidate) in candidates {
        commit_array_any_rank(vm, fiber, &place, candidate)?;
    }
    Ok(VmValue::Integer(1))
}

fn execute_array_multi_sort_ex(
    vm: &mut Vm,
    fiber: &mut Fiber,
    arguments: &[VmValue],
) -> Result<VmValue, VmError> {
    if arguments.len() < 2 {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORTEX requires a key and variable-name array".into(),
        ));
    }
    let (key, _, key_dimensions) = array_copy_place(vm, fiber, arguments.first(), "key", false)?;
    if key_dimensions.len() != 1 {
        return Err(VmError::InvalidArguments(
            "ARRAYMSORTEX key must be one-dimensional".into(),
        ));
    }
    let key_values = array_snapshot_any_rank(vm, fiber, &key)?;
    let names_place = array_place(&arguments[1..])?;
    let names = array_snapshot(vm, fiber, names_place)?;
    let ascending = !matches!(arguments.get(2), Some(VmValue::Integer(0)));
    let fixed = match integer_argument(arguments, 3) {
        Err(_) | Ok(i64::MIN) => None,
        Ok(0) => return Ok(VmValue::Integer(0)),
        Ok(value) if value > 0 => Some(usize::try_from(value).unwrap_or(usize::MAX)),
        Ok(_) => None,
    };
    if fixed.is_none()
        && key_values
            .iter()
            .any(|value| matches!(value, VmValue::String(value) if value.is_empty()))
    {
        return Ok(VmValue::Integer(0));
    }
    let key_count = fixed.map_or_else(
        || {
            key_values
                .iter()
                .position(|value| matches!(value, VmValue::Integer(0)))
                .unwrap_or(key_values.len())
        },
        |length| length.min(key_values.len()),
    );
    let mut order = (0..key_count).collect::<Vec<_>>();
    order.sort_by(
        |left, right| match (&key_values[*left], &key_values[*right]) {
            (VmValue::Integer(left), VmValue::Integer(right)) => left.cmp(right),
            (VmValue::String(left), VmValue::String(right)) => left.cmp(right),
            _ => std::cmp::Ordering::Equal,
        },
    );
    if !ascending {
        order.reverse();
    }
    let mut candidates = Vec::new();
    for name in names {
        let VmValue::String(name) = name else {
            return Err(VmError::InvalidArguments(
                "ARRAYMSORTEX variable-name array must contain strings".into(),
            ));
        };
        if name.is_empty() {
            break;
        }
        let (place, _, dimensions) =
            array_copy_place(vm, fiber, Some(&VmValue::String(name)), "target", true)?;
        let values = array_snapshot_any_rank(vm, fiber, &place)?;
        let first = usize::try_from(dimensions[0])
            .map_err(|_| VmError::InvalidState("ARRAYMSORTEX dimension is too large".into()))?;
        if first < key_count {
            return Ok(VmValue::Integer(0));
        }
        let row_width = values.len() / first;
        let mut candidate = values.clone();
        for (destination, source) in order.iter().copied().enumerate() {
            candidate[destination * row_width..(destination + 1) * row_width]
                .clone_from_slice(&values[source * row_width..(source + 1) * row_width]);
        }
        candidates.push((place, candidate));
    }
    for (place, candidate) in candidates {
        commit_array_any_rank(vm, fiber, &place, candidate)?;
    }
    Ok(VmValue::Integer(1))
}

fn array_copy_place(
    vm: &Vm,
    fiber: &Fiber,
    value: Option<&VmValue>,
    role: &str,
    destination: bool,
) -> Result<(PlaceDescriptor, BytecodeType, Vec<u64>), VmError> {
    let generation = fiber.frames.last().expect("frame exists").generation;
    let artifact = &vm
        .generations
        .get(&generation)
        .ok_or_else(|| VmError::InvalidState("ARRAYCOPY generation is missing".into()))?
        .artifact;
    let (place, value_type) = match value {
        Some(VmValue::IntegerPlace(place)) => (place.clone(), BytecodeType::Integer),
        Some(VmValue::StringPlace(place)) => (place.clone(), BytecodeType::String),
        Some(VmValue::String(name)) => {
            let definition = artifact
                .globals
                .iter()
                .find(|definition| definition.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    VmError::InvalidArguments(format!(
                        "ARRAYCOPY {role} variable {name:?} does not exist"
                    ))
                })?;
            (
                PlaceDescriptor {
                    variable: definition.key,
                    ..PlaceDescriptor::default()
                },
                definition.value_type,
            )
        }
        _ => {
            return Err(VmError::InvalidArguments(format!(
                "ARRAYCOPY {role} must be an array place or variable-name string"
            )));
        }
    };
    let definition = artifact
        .globals
        .iter()
        .find(|definition| definition.key == place.variable)
        .ok_or_else(|| VmError::InvalidState("ARRAYCOPY variable is missing".into()))?;
    if definition.storage == BytecodeStorage::Character {
        return Err(VmError::InvalidArguments(format!(
            "ARRAYCOPY {role} cannot be a character variable"
        )));
    }
    if destination && !definition.mutable {
        return Err(VmError::InvalidArguments(
            "ARRAYCOPY destination is read-only".into(),
        ));
    }
    if !(1..=3).contains(&definition.dimensions.len()) || !place.indices.is_empty() {
        return Err(VmError::InvalidArguments(format!(
            "ARRAYCOPY {role} must be an unindexed one to three dimensional array"
        )));
    }
    Ok((place, value_type, definition.dimensions.clone()))
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
