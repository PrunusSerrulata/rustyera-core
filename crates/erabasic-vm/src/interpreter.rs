use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeFunctionKind, BytecodeStorage, BytecodeType, HostSnapshotCapability, ImportKind,
    Opcode, SymbolKey, opcode,
};

use crate::state::{EventDispatch, EventDispatchEntry, ForLoopState, ProgramGeneration};
use crate::{
    Fiber, FiberId, FiberState, HostCallRequest, HostCallResult, HostReady, HostWaitStability,
    NativeCallRequest, NativePlaceView, NativeReady, NativeServiceRegistry, PlaceDescriptor,
    RunBudget, Vm, VmError, VmEvent, VmFault, VmFaultCode, VmHost, VmRunReport, VmRunStop, VmValue,
    WaitingHost, bind_persistent_arguments, make_frame, prepare_dynamic_arguments,
    validate_arguments,
};

mod character_ops;
pub(crate) mod dynamic_form;
mod extended_ops;
mod native_ops;
mod operand;

use character_ops::{character_series, execute_character_mutation, execute_character_query};
use dynamic_form::{RuntimeFormStep, begin_runtime_form, resume_runtime_form};
use extended_ops::{
    array_snapshot_any_rank, execute_array_copy, execute_array_multi_sort,
    execute_array_multi_sort_ex, execute_random_place_transaction, execute_regex_match,
    global_unindexed_place, indexed_place,
};
use native_ops::{
    array_place, array_snapshot, execute_array_mutation, execute_array_query, execute_bit_mutation,
    execute_encode_to_uni_result, execute_erdname, execute_find_element, execute_get_var,
    execute_getnum, execute_index_by_name, execute_integer_mutation, execute_set_var,
    execute_split_transaction, execute_strjoin, execute_swap_transaction, execute_variable_fill,
    integer_argument, native_implicit_place_views, native_place_views, optional_index,
    validate_native_ready,
};
use operand::{
    assign_binary_tag, binary_value, exact, map_vm_error, pop, pop_arguments, pop_indices,
    read_u16, read_u32, unary_value,
};

enum StepOutcome {
    Continue,
    BulkProgress(u64),
    DeferredNative,
    Yielded,
    Blocked,
    Completed(Option<VmValue>),
}

#[derive(Clone, Copy)]
struct ExecutionPolicy {
    allow_function_memo: bool,
    remaining_quantum: u32,
    remaining_instructions: u64,
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

struct InstructionPosition<'a> {
    generation: crate::GenerationId,
    function: SymbolKey,
    instruction: usize,
    variable: Option<&'a erabasic_bytecode::BytecodeGlobal>,
    encoded: DispatchInstruction<'a>,
}

#[derive(Clone)]
struct FunctionCursor {
    generation: crate::GenerationId,
    function: SymbolKey,
    index: usize,
    program: Arc<ProgramGeneration>,
}

struct DispatchInstruction<'a> {
    opcode: u16,
    payload: &'a [u8],
}

impl DispatchInstruction<'static> {
    fn trap() -> Self {
        Self {
            opcode: Opcode::Trap as u16,
            payload: &[],
        }
    }
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
        let mut function_cursor = None;
        // Debug controls cannot be installed concurrently while this mutable VM
        // slice is running. Once active, keep checking until the slice ends so
        // resume-skip and step-plan transitions retain their existing behavior.
        let debug_checks_active = self.debug_checks_active();
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
                let continuation_origin = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.runtime_form.as_ref())
                    .map(dynamic_form::RuntimeFormContinuation::origin);
                if continuation_origin.is_none()
                    && debug_checks_active
                    && let Some(stop) = self.debug_stop_before(&fiber)
                {
                    report.events.push(VmEvent::DebugStopped(stop));
                    break;
                }
                let position_result =
                    if let Some((generation, function, instruction)) = continuation_origin {
                        self.instruction_position_at(
                            generation,
                            function,
                            instruction,
                            &mut function_cursor,
                        )
                    } else {
                        self.instruction_position(&fiber, &mut function_cursor)
                    };
                let position = match position_result {
                    Ok(position) => position,
                    Err(error) => {
                        let fallback = fiber.frames.last().map_or(
                            InstructionPosition {
                                generation: self.current_generation,
                                function: SymbolKey::default(),
                                instruction: 0,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            },
                            |frame| InstructionPosition {
                                generation: frame.generation,
                                function: frame.function,
                                instruction: frame.instruction,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            },
                        );
                        let fault = self.make_fault(
                            fiber.id,
                            &fallback,
                            VmFaultCode::InvalidInstruction,
                            error.to_string(),
                        );
                        fiber.clear_runtime_forms();
                        fiber.state = FiberState::Faulted(fault.clone());
                        report.events.push(VmEvent::FiberFaulted {
                            fiber: fiber.id,
                            fault,
                        });
                        break;
                    }
                };
                if continuation_origin.is_none()
                    && position.encoded.opcode == Opcode::CallHost as u16
                    && report.host_calls >= budget.maximum_host_calls
                {
                    budget_exhausted = true;
                    break;
                }
                let host_before = report.host_calls;
                let policy = ExecutionPolicy {
                    allow_function_memo: !debug_checks_active,
                    remaining_quantum: quantum.saturating_sub(used),
                    remaining_instructions: budget
                        .maximum_instructions
                        .saturating_sub(report.instructions),
                };
                let outcome = if continuation_origin.is_some() {
                    resume_runtime_form(self, &mut fiber, natives).and_then(|step| match step {
                        RuntimeFormStep::Pending => Ok(StepOutcome::DeferredNative),
                        RuntimeFormStep::Complete(value) => {
                            let frame = fiber.frames.last_mut().ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "STRFORM owner frame disappeared before completion",
                                )
                            })?;
                            frame.stack.push(VmValue::String(value));
                            Ok(StepOutcome::Continue)
                        }
                    })
                } else {
                    self.execute_instruction(
                        &mut fiber,
                        &position,
                        host,
                        natives,
                        &mut report.host_calls,
                        policy,
                    )
                };
                let additional_instructions = match &outcome {
                    Ok(StepOutcome::BulkProgress(instructions)) => *instructions,
                    _ => 0,
                };
                report.instructions = report
                    .instructions
                    .saturating_add(1)
                    .saturating_add(additional_instructions);
                used = used
                    .saturating_add(1)
                    .saturating_add(u32::try_from(additional_instructions).unwrap_or(u32::MAX));
                if report.host_calls != host_before {
                    fiber.mark_progress();
                }
                match outcome {
                    Ok(StepOutcome::Continue | StepOutcome::BulkProgress(_)) => {
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                            break;
                        }
                    }
                    Ok(StepOutcome::DeferredNative) => {}
                    Ok(StepOutcome::Yielded) => {
                        fiber.mark_progress();
                        yielded = true;
                        report
                            .events
                            .push(VmEvent::FiberYielded { fiber: fiber.id });
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, false)
                        {
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
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, true, false)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Ok(StepOutcome::Completed(value)) => {
                        report.events.push(VmEvent::FiberCompleted {
                            fiber: fiber.id,
                            value,
                        });
                        if debug_checks_active
                            && let Some(stop) = self.debug_stop_after(&fiber, false, true)
                        {
                            report.events.push(VmEvent::DebugStopped(stop));
                        }
                        break;
                    }
                    Err(error) => {
                        fiber.clear_runtime_forms();
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
                    fiber.clear_runtime_forms();
                    fiber.state = FiberState::Faulted(fault.clone());
                    report.events.push(VmEvent::FiberFaulted {
                        fiber: fiber.id,
                        fault,
                    });
                    break;
                }
            }

            if matches!(fiber.state, FiberState::Runnable) {
                // A fiber quantum is scheduler preemption, not evidence that the caller's
                // instruction budget was exhausted. Large finite EraBasic routines can span
                // many quanta in one run slice (for example, the eraTW all-items scan). Count
                // only slices that actually consume the caller-visible budget so such work is
                // not mistaken for persistent runaway execution.
                if report.instructions >= budget.maximum_instructions && !yielded {
                    fiber.consecutive_budget_exhaustions =
                        fiber.consecutive_budget_exhaustions.saturating_add(1);
                    if fiber.consecutive_budget_exhaustions
                        > self.config.maximum_consecutive_budget_exhaustions
                    {
                        let position = self
                            .instruction_position(&fiber, &mut function_cursor)
                            .unwrap_or(InstructionPosition {
                                generation: self.current_generation,
                                function: SymbolKey::default(),
                                instruction: 0,
                                variable: None,
                                encoded: DispatchInstruction::trap(),
                            });
                        let fault = self.make_fault(
                            fiber.id,
                            &position,
                            VmFaultCode::RunawayExecution,
                            "instruction-budget watchdog detected persistent execution without progress",
                        );
                        fiber.clear_runtime_forms();
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
            if matches!(fiber.state, FiberState::Faulted(_) | FiberState::Cancelled) {
                for frame in &fiber.frames {
                    self.active_function_memos.remove(&frame.id);
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

    fn instruction_position<'cursor>(
        &self,
        fiber: &Fiber,
        cursor: &'cursor mut Option<FunctionCursor>,
    ) -> Result<InstructionPosition<'cursor>, VmError> {
        let frame = fiber
            .frames
            .last()
            .ok_or_else(|| VmError::InvalidState("runnable fiber has no frame".into()))?;
        self.instruction_position_at(frame.generation, frame.function, frame.instruction, cursor)
    }

    fn instruction_position_at<'cursor>(
        &self,
        generation: crate::GenerationId,
        function_key: SymbolKey,
        instruction: usize,
        cursor: &'cursor mut Option<FunctionCursor>,
    ) -> Result<InstructionPosition<'cursor>, VmError> {
        if cursor
            .as_ref()
            .is_none_or(|cursor| cursor.generation != generation)
        {
            let program =
                Arc::clone(self.generations.get(&generation).ok_or_else(|| {
                    VmError::InvalidState("frame generation was reclaimed".into())
                })?);
            let index = *program
                .function_index(function_key)
                .ok_or(VmError::MissingFunction(function_key))?;
            *cursor = Some(FunctionCursor {
                generation,
                function: function_key,
                index,
                program,
            });
        } else if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.function != function_key)
        {
            let cursor = cursor.as_mut().expect("the generation cursor exists");
            cursor.index = *cursor
                .program
                .function_index(function_key)
                .ok_or(VmError::MissingFunction(function_key))?;
            cursor.function = function_key;
        }
        let cursor = cursor
            .as_ref()
            .expect("the generation cursor was initialized");
        let function = cursor
            .program
            .artifact
            .functions
            .get(cursor.index)
            .filter(|function| function.key == function_key)
            .ok_or(VmError::MissingFunction(function_key))?;
        let encoded = function
            .code
            .get(instruction)
            .ok_or_else(|| VmError::InvalidState("instruction pointer left its function".into()))?;
        // The cursor owns the generation Arc, so this payload borrow is independent
        // of `self` and remains valid across mutable VM dispatch for this instruction.
        Ok(InstructionPosition {
            generation,
            function: function_key,
            instruction,
            variable: cursor.program.instruction_global(cursor.index, instruction),
            encoded: DispatchInstruction {
                opcode: encoded.opcode,
                payload: &encoded.payload,
            },
        })
    }

    #[allow(clippy::too_many_lines)]
    fn execute_instruction(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
        host_calls: &mut u32,
        policy: ExecutionPolicy,
    ) -> Result<StepOutcome, StepError> {
        let opcode = Opcode::try_from(position.encoded.opcode).map_err(|opcode| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                format!("unknown opcode {opcode}"),
            )
        })?;
        if opcode == Opcode::PushString
            && let Some(additional_instructions) =
                self.try_literal_group_match(fiber, position, policy)
        {
            return Ok(StepOutcome::BulkProgress(additional_instructions));
        }
        let frame = fiber
            .frames
            .last_mut()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "missing frame"))?;
        frame.instruction = frame.instruction.saturating_add(1);
        match opcode {
            Opcode::Nop => {}
            Opcode::PushInteger => {
                let bytes = exact::<8>(position.encoded.payload)?;
                frame
                    .stack
                    .push(VmValue::Integer(i64::from_le_bytes(bytes)));
            }
            Opcode::PushString => {
                let length = read_u32(position.encoded.payload, 0)? as usize;
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
                let count = read_u16(position.encoded.payload, 16)? as usize;
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
                let definition = position.variable.ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::MissingSymbol,
                        "variable operand does not identify a defined global",
                    )
                })?;
                let key = definition.key;
                let character = if definition.storage == BytecodeStorage::Character {
                    if indices.as_slice().len() > definition.dimensions.len() {
                        Some(indices.as_slice()[0])
                    } else {
                        Some(self.target_character_for_generation(position.generation) as u64)
                    }
                } else {
                    None
                };
                let value_indices = if character.is_some()
                    && indices.as_slice().len() > definition.dimensions.len()
                {
                    &indices.as_slice()[1..]
                } else {
                    indices.as_slice()
                };
                if opcode == Opcode::MakePlace {
                    let place = PlaceDescriptor {
                        variable: key,
                        indices: value_indices.to_vec(),
                        character,
                        fiber: Some(fiber.id),
                        frame: (definition.storage == BytecodeStorage::FunctionLocal)
                            .then_some(frame.id),
                    };
                    let value = match definition.value_type {
                        BytecodeType::Integer => VmValue::IntegerPlace(Box::new(place)),
                        BytecodeType::String => VmValue::StringPlace(Box::new(place)),
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
                    let value = self
                        .read_variable_resolved(
                            fiber,
                            position.generation,
                            definition,
                            value_indices,
                            character,
                            (definition.storage == BytecodeStorage::FunctionLocal)
                                .then_some(frame.id),
                        )
                        .map_err(map_vm_error)?;
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .stack
                        .push(value);
                } else {
                    let mut value = value.expect("store value was popped");
                    if operation != 0 {
                        let previous = self
                            .read_variable_resolved(
                                fiber,
                                position.generation,
                                definition,
                                value_indices,
                                character,
                                (definition.storage == BytecodeStorage::FunctionLocal)
                                    .then_some(frame.id),
                            )
                            .map_err(map_vm_error)?;
                        value = binary_value(assign_binary_tag(operation)?, previous, value)?;
                    }
                    self.write_variable_resolved(
                        fiber,
                        position.generation,
                        definition,
                        value_indices,
                        character,
                        (definition.storage == BytecodeStorage::FunctionLocal).then_some(frame.id),
                        value,
                    )
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
                let count = read_u16(position.encoded.payload, 0)? as usize;
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
            Opcode::ForStart => {
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let VmValue::Integer(step) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR step expects an integer",
                    ));
                };
                let VmValue::Integer(end) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR end expects an integer",
                    ));
                };
                let VmValue::Integer(start) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR start expects an integer",
                    ));
                };
                let VmValue::IntegerPlace(counter) = pop(stack)? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter expects an integer place",
                    ));
                };
                self.write_place(fiber, &counter, VmValue::Integer(start))
                    .map_err(map_vm_error)?;
                let active = (step > 0 && start < end) || (step < 0 && start > end);
                if active
                    && let Some(additional_instructions) = self
                        .try_bulk_fill_loop(fiber, position, &counter, start, end, step, policy)?
                {
                    return Ok(StepOutcome::BulkProgress(additional_instructions));
                }
                if active {
                    fiber
                        .frames
                        .last_mut()
                        .expect("frame exists")
                        .for_loops
                        .push(ForLoopState {
                            counter: *counter,
                            end,
                            step,
                        });
                }
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::Integer(i64::from(active)));
            }
            Opcode::ForNext => {
                let state = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.for_loops.last())
                    .cloned()
                    .ok_or_else(|| {
                        StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "NEXT has no active FOR loop",
                        )
                    })?;
                let VmValue::Integer(current) = self
                    .read_place(fiber, &state.counter)
                    .map_err(map_vm_error)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter storage is not integer",
                    ));
                };
                let next = current.wrapping_add(state.step);
                self.write_place(fiber, &state.counter, VmValue::Integer(next))
                    .map_err(map_vm_error)?;
                let active =
                    (state.step > 0 && next < state.end) || (state.step < 0 && next > state.end);
                let frame = fiber.frames.last_mut().expect("frame exists");
                if !active {
                    frame.for_loops.pop();
                }
                frame.stack.push(VmValue::Integer(i64::from(active)));
            }
            Opcode::ForBreak => {
                let state = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.for_loops.last())
                    .cloned()
                    .ok_or_else(|| {
                        StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "BREAK has no active FOR loop",
                        )
                    })?;
                let VmValue::Integer(current) = self
                    .read_place(fiber, &state.counter)
                    .map_err(map_vm_error)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "FOR counter storage is not integer",
                    ));
                };
                self.write_place(
                    fiber,
                    &state.counter,
                    VmValue::Integer(current.wrapping_add(state.step)),
                )
                .map_err(map_vm_error)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .for_loops
                    .pop()
                    .expect("active FOR loop was checked");
            }
            Opcode::SelectStart => {
                let value = pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?;
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .select_values
                    .push(value);
            }
            Opcode::SelectCompare => {
                let operation = *position.encoded.payload.first().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "CASE comparison operation is missing",
                    )
                })?;
                let selector = fiber
                    .frames
                    .last()
                    .and_then(|frame| frame.select_values.last())
                    .cloned()
                    .ok_or_else(|| {
                        StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "CASE is outside SELECTCASE",
                        )
                    })?;
                let stack = &mut fiber.frames.last_mut().expect("frame exists").stack;
                let matched = if operation == 6 {
                    let upper = pop(stack)?;
                    let lower = pop(stack)?;
                    let VmValue::Integer(lower_match) = binary_value(10, selector.clone(), lower)?
                    else {
                        unreachable!("comparison produces integer")
                    };
                    let VmValue::Integer(upper_match) = binary_value(8, selector, upper)? else {
                        unreachable!("comparison produces integer")
                    };
                    lower_match != 0 && upper_match != 0
                } else {
                    let operand = pop(stack)?;
                    let binary_operation = match operation {
                        0 => 11,
                        1 => 12,
                        2 => 7,
                        3 => 8,
                        4 => 9,
                        5 => 10,
                        7 => 13,
                        _ => {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "unknown CASE comparison operation",
                            ));
                        }
                    };
                    let VmValue::Integer(value) =
                        binary_value(binary_operation, selector, operand)?
                    else {
                        unreachable!("comparison produces integer")
                    };
                    value != 0
                };
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .stack
                    .push(VmValue::Integer(i64::from(matched)));
            }
            Opcode::SelectEnd => {
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .select_values
                    .pop()
                    .ok_or_else(|| {
                        StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "ENDSELECT is outside SELECTCASE",
                        )
                    })?;
            }
            Opcode::Jump | Opcode::JumpIfFalse => {
                let target = read_u32(position.encoded.payload, 0)? as usize;
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
                let missing_target = read_u32(position.encoded.payload, 0)? as usize;
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
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
                if let Some(target) = generation.function_by_name(&name) {
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
                let argument_count = read_u16(position.encoded.payload, 0)? as usize;
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
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
                let target = generation.function_by_name(&name).ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "resolved function disappeared")
                })?;
                let mut arguments = arguments;
                for (parameter, argument) in target.parameters.iter().zip(&mut arguments) {
                    if parameter.by_reference {
                        continue;
                    }
                    let place = match argument {
                        VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => place.clone(),
                        _ => continue,
                    };
                    *argument = self.read_place(fiber, &place).map_err(map_vm_error)?;
                }
                let arguments =
                    prepare_dynamic_arguments(target, arguments, artifact.call_compatibility)
                        .map_err(map_vm_error)?;
                self.memory.ensure_function_statics(
                    position.generation,
                    target.key,
                    generation.function_statics(target.key),
                );
                bind_persistent_arguments(
                    &mut self.memory,
                    position.generation,
                    target,
                    generation,
                    &arguments,
                )
                .map_err(map_vm_error)?;
                let event_context = fiber.frames.last().expect("frame exists").event_context
                    || target.kind == BytecodeFunctionKind::Event;
                let frame = make_frame(
                    new_frame,
                    position.generation,
                    target,
                    generation.function_locals(target.key),
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
                let import_index = read_u32(position.encoded.payload, 0)? as usize;
                let argument_count = read_u16(position.encoded.payload, 4)? as usize;
                let new_frame = (opcode == Opcode::Call).then(|| self.allocate_frame_id());
                let generation = Arc::clone(
                    self.generations
                        .get(&position.generation)
                        .expect("validated frame generation exists"),
                );
                let artifact = &generation.artifact;
                let function = generation
                    .function(position.function)
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
                        let target = generation.function(import.key).ok_or_else(|| {
                            StepError::new(VmFaultCode::MissingSymbol, "called function is missing")
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
                        validate_arguments(target, &arguments).map_err(map_vm_error)?;
                        self.memory.ensure_function_statics(
                            position.generation,
                            target.key,
                            generation.function_statics(target.key),
                        );
                        bind_persistent_arguments(
                            &mut self.memory,
                            position.generation,
                            target,
                            &generation,
                            &arguments,
                        )
                        .map_err(map_vm_error)?;
                        if policy.allow_function_memo
                            && let Some(value) = self.try_memoized_indexed_read(
                                fiber,
                                position.generation,
                                target.key,
                                &arguments,
                            )?
                        {
                            fiber
                                .frames
                                .last_mut()
                                .expect("caller frame exists")
                                .stack
                                .push(value);
                            return Ok(StepOutcome::Continue);
                        }
                        let frame_id = new_frame.expect("function call reserved a frame id");
                        let memo_key = (policy.allow_function_memo
                            && usize::try_from(policy.remaining_quantum)
                                .ok()
                                .is_some_and(|remaining| target.code.len() < remaining))
                        .then(|| {
                            self.function_memo_key(position.generation, target.key, &arguments)
                        })
                        .flatten();
                        if let Some(entry) = memo_key
                            .as_ref()
                            .and_then(|key| self.function_memo_cache.get(key))
                            .cloned()
                        {
                            self.replay_function_memo_entry(position.generation, &entry)
                                .map_err(map_vm_error)?;
                            fiber
                                .frames
                                .last_mut()
                                .expect("caller frame exists")
                                .stack
                                .push(entry.result);
                            return Ok(StepOutcome::Continue);
                        }
                        let frame = make_frame(
                            frame_id,
                            position.generation,
                            target,
                            generation.function_locals(target.key),
                            arguments,
                            target.result.is_some(),
                            fiber
                                .frames
                                .last()
                                .expect("caller frame exists")
                                .event_context
                                || target.kind == BytecodeFunctionKind::Event,
                        );
                        if let Some(key) = memo_key {
                            self.active_function_memos.insert(frame_id, key);
                        }
                        fiber.frames.push(frame);
                    }
                    (Opcode::CallNative, ImportKind::Native) => {
                        let target_index =
                            generation.native_import_index(import.key).ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?;
                        let target = artifact
                            .native_imports
                            .get(target_index)
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?
                            .import
                            .clone();
                        let result_type = target.result;
                        let native_name = generation
                            .normalized_native_name(target_index)
                            .ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    "native import is missing",
                                )
                            })?;
                        let native_name = native_name.as_ref();
                        let mut rollback = None;
                        let ready = if native_name == "strform" {
                            if result_type != Some(BytecodeType::String) || arguments.len() != 1 {
                                return Err(StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "STRFORM import signature is invalid",
                                ));
                            }
                            let value = arguments.first().and_then(|value| match value {
                                VmValue::String(value) => Some(value.as_str()),
                                _ => None,
                            });
                            let value = value.ok_or_else(|| {
                                StepError::new(
                                    VmFaultCode::TypeMismatch,
                                    "STRFORM expects a string",
                                )
                            })?;
                            begin_runtime_form(
                                self,
                                fiber,
                                natives,
                                position.generation,
                                position.function,
                                position.instruction,
                                value,
                            )?;
                            return Ok(StepOutcome::DeferredNative);
                        } else if matches!(native_name, "initrand" | "dumprand") {
                            execute_random_place_transaction(
                                &mut self.memory,
                                position.generation,
                                artifact,
                                natives,
                                native_name,
                            )
                            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
                            NativeReady::default()
                        } else if native_name == "__mutate_integer" {
                            NativeReady::value(
                                execute_integer_mutation(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "swap" | "swapvar") {
                            execute_swap_transaction(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(native_name, "setbit" | "clearbit" | "invertbit") {
                            execute_bit_mutation(self, fiber, native_name, &arguments)
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
                        } else if native_name == "erdname" {
                            NativeReady::value(
                                execute_erdname(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if native_name == "__indexbyname" {
                            NativeReady::value(
                                execute_index_by_name(self, fiber, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "setvar" {
                            NativeReady::value(
                                execute_set_var(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "getvar" | "getvars") {
                            NativeReady::value(
                                execute_get_var(self, fiber, &arguments, native_name == "getvars")
                                    .map_err(map_vm_error)?,
                            )
                        } else if native_name == "__encodetouni_result" {
                            execute_encode_to_uni_result(self, fiber, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "strjoin" {
                            NativeReady::value(
                                execute_strjoin(self, fiber, &arguments).map_err(map_vm_error)?,
                            )
                        } else if matches!(native_name, "arrayremove" | "arrayshift" | "arraysort")
                        {
                            execute_array_mutation(self, fiber, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if native_name == "arraycopy" {
                            execute_array_copy(self, fiber, &arguments).map_err(map_vm_error)?;
                            NativeReady::default()
                        } else if matches!(native_name, "varset" | "cvarset") {
                            execute_variable_fill(self, fiber, native_name, &arguments)
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
                        } else if matches!(native_name, "findelement" | "findlastelement") {
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
                            native_name,
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
                                execute_array_query(self, fiber, native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name,
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
                                execute_character_query(self, fiber, native_name, &arguments)
                                    .map_err(map_vm_error)?,
                            )
                        } else if matches!(
                            native_name,
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
                            execute_character_mutation(self, native_name, &arguments)
                                .map_err(map_vm_error)?;
                            NativeReady::default()
                        } else {
                            let (ready, checkpoint) = self.call_registered_native(
                                fiber, import.key, target, arguments, natives,
                            )?;
                            rollback = checkpoint;
                            ready
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
                        let target_index =
                            generation.host_import_index(import.key).ok_or_else(|| {
                                StepError::new(VmFaultCode::MissingSymbol, "host import is missing")
                            })?;
                        let target = artifact
                            .host_imports
                            .get(target_index)
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
                let missing_target = read_u32(position.encoded.payload, 0)? as usize;
                let VmValue::String(name) =
                    pop(&mut fiber.frames.last_mut().expect("frame exists").stack)?
                else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "dynamic label target must be a string",
                    ));
                };
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let function = generation
                    .function(position.function)
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
                let generation = self
                    .generations
                    .get(&position.generation)
                    .expect("validated frame generation exists");
                let artifact = &generation.artifact;
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
                let target = generation.function(active.function).ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "event function is missing")
                })?;
                self.memory.ensure_function_statics(
                    position.generation,
                    target.key,
                    generation.function_statics(target.key),
                );
                fiber
                    .frames
                    .last_mut()
                    .expect("frame exists")
                    .event_dispatch = Some(EventDispatch { active, pending });
                fiber.frames.push(make_frame(
                    frame_id,
                    position.generation,
                    target,
                    generation.function_locals(target.key),
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
                if let Some(key) = self.active_function_memos.remove(&returned_frame.id)
                    && policy.allow_function_memo
                    && let Some(value) = value.as_ref()
                    && let Some(entry) = self.capture_function_memo_entry(&key, value.clone())
                {
                    if self.function_memo_cache.len() >= 65_536 {
                        self.function_memo_cache.clear();
                    }
                    self.function_memo_cache.insert(key, entry);
                }
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
                        let program = self
                            .generations
                            .get(&generation)
                            .expect("validated frame generation exists");
                        let target = program.function(next.function).ok_or_else(|| {
                            StepError::new(VmFaultCode::MissingSymbol, "event function is missing")
                        })?;
                        self.memory.ensure_function_statics(
                            generation,
                            target.key,
                            program.function_statics(target.key),
                        );
                        fiber.frames.push(make_frame(
                            frame_id,
                            generation,
                            target,
                            program.function_locals(target.key),
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
                let message = String::from_utf8_lossy(position.encoded.payload);
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

    fn call_registered_native(
        &mut self,
        fiber: &mut Fiber,
        key: SymbolKey,
        import: erabasic_bytecode::RuntimeImport,
        arguments: Vec<VmValue>,
        natives: &mut NativeServiceRegistry,
    ) -> Result<(NativeReady, Option<Vec<u8>>), StepError> {
        let places = native_place_views(self, fiber, &arguments).map_err(map_vm_error)?;
        let implicit_place_names = natives
            .implicit_place_names(key)
            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
        let implicit_places =
            native_implicit_place_views(self, fiber, implicit_place_names).map_err(map_vm_error)?;
        let rollback = natives
            .checkpoint(key)
            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
        let ready = natives
            .call(
                key,
                NativeCallRequest {
                    import,
                    arguments,
                    places,
                    implicit_places,
                },
            )
            .map_err(|error| StepError::new(VmFaultCode::Native, error))?;
        Ok((ready, rollback))
    }

    fn try_memoized_indexed_read(
        &mut self,
        fiber: &mut Fiber,
        generation_id: crate::GenerationId,
        function: SymbolKey,
        arguments: &[VmValue],
    ) -> Result<Option<VmValue>, StepError> {
        let Some(plan) = self
            .generations
            .get(&generation_id)
            .and_then(|generation| generation.memoized_indexed_read_plan(function))
            .cloned()
        else {
            return Ok(None);
        };
        let Some(VmValue::Integer(index)) = arguments.get(plan.index_parameter) else {
            return Ok(None);
        };
        let Some(selector) = arguments.get(plan.selector_parameter) else {
            return Ok(None);
        };
        let selector_arguments = [
            VmValue::String(plan.selector_prefix.clone()),
            selector.clone(),
        ];
        let Some(key) =
            self.function_memo_key(generation_id, plan.selector_function, &selector_arguments)
        else {
            return Ok(None);
        };
        let Some(entry) = self.function_memo_cache.get(&key).cloned() else {
            return Ok(None);
        };
        let VmValue::Integer(selector_index) = entry.result else {
            return Ok(None);
        };
        let Some(index) = u64::try_from(*index).ok() else {
            return Ok(None);
        };
        let Some(selector_index) = u64::try_from(selector_index).ok() else {
            return Ok(None);
        };
        let (selector_function, scratch, target) = {
            let generation = self
                .generations
                .get(&generation_id)
                .expect("validated frame generation exists");
            let selector_function = generation
                .function(plan.selector_function)
                .expect("memoized selector function exists")
                .clone();
            let scratch = generation
                .global(plan.scratch)
                .expect("indexed read scratch exists")
                .clone();
            let target = generation
                .global(plan.target)
                .expect("indexed read target exists")
                .clone();
            (selector_function, scratch, target)
        };
        self.memory.ensure_function_statics(
            generation_id,
            selector_function.key,
            self.generations
                .get(&generation_id)
                .expect("validated frame generation exists")
                .function_statics(selector_function.key),
        );
        let generation = self
            .generations
            .get(&generation_id)
            .expect("validated frame generation exists");
        bind_persistent_arguments(
            &mut self.memory,
            generation_id,
            &selector_function,
            generation,
            &selector_arguments,
        )
        .map_err(map_vm_error)?;
        self.replay_function_memo_entry(generation_id, &entry)
            .map_err(map_vm_error)?;
        self.memory
            .cell_mut(generation_id, &scratch, 0)
            .ok_or_else(|| {
                StepError::new(
                    VmFaultCode::MissingSymbol,
                    "indexed read scratch is missing",
                )
            })?
            .write(
                &[],
                VmValue::Integer(i64::try_from(selector_index).unwrap_or(i64::MAX)),
            )
            .map_err(|error| StepError::new(VmFaultCode::InvalidInstruction, error))?;
        self.read_variable_resolved(
            fiber,
            generation_id,
            &target,
            &[index, selector_index],
            None,
            None,
        )
        .map(Some)
        .map_err(map_vm_error)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_bulk_fill_loop(
        &mut self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        counter: &PlaceDescriptor,
        start: i64,
        end: i64,
        step: i64,
        policy: ExecutionPolicy,
    ) -> Result<Option<u64>, StepError> {
        if !policy.allow_function_memo
            || step != 1
            || counter.character.is_some()
            || !counter.indices.is_empty()
        {
            return Ok(None);
        }
        let Some(plan) = self
            .generations
            .get(&position.generation)
            .and_then(|generation| {
                generation.bulk_fill_loop_plan(position.function, position.instruction)
            })
            .cloned()
        else {
            return Ok(None);
        };
        if counter.variable != plan.counter {
            return Ok(None);
        }
        let iterations = u64::try_from(end.wrapping_sub(start)).unwrap_or(u64::MAX);
        let logical_instructions = iterations.saturating_mul(7).saturating_add(2);
        if logical_instructions > policy.remaining_instructions
            || logical_instructions > u64::from(policy.remaining_quantum)
            || fiber
                .backward_branches_without_progress
                .saturating_add(iterations.saturating_sub(1))
                > self.config.maximum_backward_branches_without_progress
        {
            return Ok(None);
        }
        let (prefix, target) = {
            let generation = self
                .generations
                .get(&position.generation)
                .expect("validated frame generation exists");
            let Some(prefix) = generation.global(plan.prefix).cloned() else {
                return Ok(None);
            };
            let Some(target) = generation.global(plan.target).cloned() else {
                return Ok(None);
            };
            (prefix, target)
        };
        let frame = fiber.frames.last().expect("frame exists");
        let VmValue::Integer(prefix_index) = self
            .read_variable_resolved(
                fiber,
                position.generation,
                &prefix,
                &[],
                None,
                (prefix.storage == BytecodeStorage::FunctionLocal).then_some(frame.id),
            )
            .map_err(map_vm_error)?
        else {
            return Ok(None);
        };
        let Some((flat_start, flat_end)) =
            bulk_fill_flat_range(&target.dimensions, prefix_index, start, end)
        else {
            return Ok(None);
        };
        self.fill_place_array_range(
            fiber,
            &PlaceDescriptor {
                variable: target.key,
                indices: Vec::new(),
                character: None,
                fiber: Some(fiber.id),
                frame: None,
            },
            flat_start,
            flat_end,
            plan.value,
        )
        .map_err(map_vm_error)?;
        self.write_place(fiber, counter, VmValue::Integer(end))
            .map_err(map_vm_error)?;
        let frame = fiber.frames.last_mut().expect("frame exists");
        frame.instruction = plan.after_loop;
        fiber.backward_branches_without_progress = fiber
            .backward_branches_without_progress
            .saturating_add(iterations.saturating_sub(1));
        Ok(Some(logical_instructions.saturating_sub(1)))
    }

    fn try_literal_group_match(
        &self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        policy: ExecutionPolicy,
    ) -> Option<u64> {
        if !policy.allow_function_memo {
            return None;
        }
        let plan = self
            .generations
            .get(&position.generation)?
            .literal_group_match_plan(position.function, position.instruction)?;
        let logical_instructions = u64::try_from(plan.candidates.len()).ok()?.saturating_add(1);
        if logical_instructions > policy.remaining_instructions
            || logical_instructions > u64::from(policy.remaining_quantum)
        {
            return None;
        }
        let frame = fiber.frames.last_mut()?;
        let VmValue::String(value) = frame.stack.last()? else {
            return None;
        };
        let matches = plan
            .candidates
            .iter()
            .filter(|candidate| candidate.as_ref() == value)
            .count();
        frame.stack.pop();
        frame
            .stack
            .push(VmValue::Integer(i64::try_from(matches).unwrap_or(i64::MAX)));
        frame.instruction = plan.after_call;
        Some(logical_instructions.saturating_sub(1))
    }

    fn make_fault(
        &self,
        fiber: FiberId,
        position: &InstructionPosition<'_>,
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
        position: &InstructionPosition<'_>,
        command: &str,
    ) -> crate::VmExecutionOrigin {
        let generation = self.generations.get(&position.generation);
        let function = generation.and_then(|generation| generation.function(position.function));
        let source = generation.and_then(|generation| {
            generation.source_location(position.function, position.instruction)
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

    fn command_for_position(&self, position: &InstructionPosition<'_>) -> String {
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

fn bulk_fill_flat_range(
    dimensions: &[u64],
    prefix: i64,
    start: i64,
    end: i64,
) -> Option<(usize, usize)> {
    let &[rows, columns] = dimensions else {
        return None;
    };
    let prefix = u64::try_from(prefix).ok()?;
    let start = u64::try_from(start).ok()?;
    let end = u64::try_from(end).ok()?;
    if prefix >= rows || end > columns {
        return None;
    }
    let row = prefix.checked_mul(columns)?;
    Some((
        usize::try_from(row.checked_add(start)?).ok()?,
        usize::try_from(row.checked_add(end)?).ok()?,
    ))
}
