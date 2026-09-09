#[allow(clippy::wildcard_imports)]
use super::*;

impl Vm {
    pub(super) fn reconcile_structured_jump(
        &self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        target: usize,
    ) -> bool {
        let transition = if let Some((program, index)) = position.resolved_program {
            program.structured_jump_transition_at_index(index, position.instruction, target)
        } else {
            self.generations
                .get(&position.generation)
                .and_then(|generation| {
                    generation.structured_jump_transition(
                        position.function,
                        position.instruction,
                        target,
                    )
                })
        };
        let Some(transition) = transition else {
            return false;
        };
        let frame = fiber.frames.last_mut().expect("frame exists");
        frame.for_loops.truncate(transition.retain_loops);
        frame.select_values.truncate(transition.retain_selects);
        for scope in &transition.entered {
            match scope {
                StructuredScopeKind::Loop => frame.for_loops.push(ForLoopState::bypassed()),
                StructuredScopeKind::Select => {
                    frame.select_values.push(bypassed_select_value());
                }
            }
        }
        !transition.entered.is_empty()
    }

    pub(super) fn call_registered_native(
        &mut self,
        fiber: &mut Fiber,
        key: SymbolKey,
        import: erabasic_bytecode::RuntimeImport,
        arguments: Vec<VmValue>,
        natives: &mut NativeServiceRegistry,
    ) -> Result<(NativeReady, Option<Vec<u8>>), StepError> {
        self.call_registered_native_with_omissions(
            fiber,
            key,
            import,
            arguments,
            Vec::new(),
            natives,
        )
    }

    pub(super) fn call_registered_native_with_omissions(
        &mut self,
        fiber: &mut Fiber,
        key: SymbolKey,
        import: erabasic_bytecode::RuntimeImport,
        arguments: Vec<VmValue>,
        omitted_arguments: Vec<usize>,
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
        let ready = match natives.call(
            key,
            NativeCallRequest {
                service_key: key,
                omitted_arguments,
                import,
                arguments,
                places,
                implicit_places,
            },
        ) {
            Ok(ready) => ready,
            Err(error) => {
                if let Some(state) = &rollback {
                    natives.rollback(key, state).map_err(|failure| {
                        StepError::classified(
                            crate::FaultCategory::HostContract,
                            VmFaultCode::Native,
                            format!("native rollback failed: {failure}"),
                        )
                    })?;
                }
                return Err(error);
            }
        };
        Ok((ready, rollback))
    }

    pub(super) fn try_memoized_indexed_read(
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
            .cell_mut(generation_id, scratch.key, scratch.storage, 0)
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
    pub(super) fn try_bulk_fill_loop(
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
            || counter.backing.is_some()
        {
            return Ok(None);
        }
        let Some((program, _)) = position.resolved_program else {
            return Ok(None);
        };
        let Some(plan) = program.bulk_fill_loop_plan(position.function, position.instruction)
        else {
            return Ok(None);
        };
        if counter.variable != plan.counter || counter.indices != plan.counter_indices {
            return Ok(None);
        }
        let iterations = u64::try_from(end.wrapping_sub(start)).unwrap_or(u64::MAX);
        let Some(logical_instructions) =
            bulk_fill_work(fiber, plan, iterations, policy, &self.config)
        else {
            return Ok(None);
        };
        let Some(prefix) = program.global(plan.prefix) else {
            return Ok(None);
        };
        let Some(target) = program.global(plan.target) else {
            return Ok(None);
        };
        let frame = fiber.frames.last().expect("frame exists");
        let Ok(VmValue::Integer(prefix_index)) = self.read_variable_resolved(
            fiber,
            position.generation,
            prefix,
            &plan.prefix_indices,
            None,
            (prefix.storage == BytecodeStorage::FunctionLocal).then_some(frame.id),
        ) else {
            return Ok(None);
        };
        let Some((character, flat_start, flat_end)) =
            bulk_fill_target_range(target, prefix_index, start, end)
        else {
            return Ok(None);
        };
        // Preflight without mutating storage, so invalid characters/cells retain the ordinary
        // StoreVariable failure position and all earlier per-instruction effects.
        if self
            .validate_script_character(target.storage, character.unwrap_or(0))
            .is_err()
            || self
                .memory
                .cell(position.generation, target, character.unwrap_or(0))
                .is_none_or(|cell| {
                    flat_end > cell.len()
                        || cell.value_type != BytecodeType::Integer
                        || (plan.value != VmValue::Integer(0) && cell.integers().is_none())
                })
        {
            return Ok(None);
        }
        self.fill_place_array_range(
            fiber,
            &PlaceDescriptor {
                backing: None,
                variable: target.key,
                indices: Vec::new(),
                character: character.and_then(|value| u64::try_from(value).ok()),
                fiber: Some(fiber.id),
                frame: None,
            },
            flat_start,
            flat_end,
            plan.value.clone(),
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

    pub(super) fn try_literal_group_match(
        &self,
        fiber: &mut Fiber,
        position: &InstructionPosition<'_>,
        policy: ExecutionPolicy,
    ) -> Option<u64> {
        if !policy.allow_function_memo {
            return None;
        }
        let plan = position.literal_group_match?;
        let logical_instructions = u64::try_from(plan.candidates.len()).ok()?.saturating_add(1);
        if logical_instructions > policy.remaining_instructions
            || logical_instructions > u64::from(policy.remaining_quantum)
        {
            return None;
        }
        let frame = fiber.frames.last_mut()?;
        // Preserve the intermediate stack peak and its exact failure position by
        // falling back when ordinary literal pushes could exhaust the stack.
        if frame.operand_slots()?.checked_add(plan.candidates.len())?
            > self.config.maximum_operand_stack
        {
            return None;
        }
        let matches = plan.candidates.match_count(frame.stack.last()?)?;
        frame.stack.pop();
        frame
            .stack
            .push(VmValue::Integer(i64::try_from(matches).unwrap_or(i64::MAX)));
        frame.instruction = plan.after_call;
        Some(logical_instructions.saturating_sub(1))
    }

    pub(super) fn make_fault(
        &self,
        fiber: FiberId,
        position: &InstructionPosition<'_>,
        code: VmFaultCode,
        message: impl Into<String>,
    ) -> VmFault {
        self.make_classified_fault(fiber, position, super::StepError::new(code, message))
    }

    pub(super) fn make_classified_fault(
        &self,
        fiber: FiberId,
        position: &InstructionPosition<'_>,
        failure: super::StepError,
    ) -> VmFault {
        let command = self.command_for_position(position);
        let origin = self.execution_origin(position, &command);
        VmFault::from_origin(fiber, origin, failure)
    }

    pub(super) fn execution_origin(
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

    pub(super) fn command_for_position(&self, position: &InstructionPosition<'_>) -> String {
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
fn bulk_fill_work(
    fiber: &Fiber,
    plan: &crate::state::BulkFillLoopPlan,
    iterations: u64,
    policy: ExecutionPolicy,
    config: &crate::VmConfig,
) -> Option<u64> {
    let logical = iterations
        .saturating_mul(plan.iteration_instructions)
        .saturating_add(2);
    let stack_peak = fiber
        .frames
        .last()?
        .operand_slots()?
        .checked_add(plan.stack_peak)?;
    (logical <= policy.remaining_instructions
        && logical <= u64::from(policy.remaining_quantum)
        && fiber
            .backward_branches_without_progress
            .saturating_add(iterations.saturating_sub(1))
            <= config.maximum_backward_branches_without_progress
        && stack_peak <= config.maximum_operand_stack)
        .then_some(logical)
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
    if prefix >= rows || start > end || end > columns {
        return None;
    }
    let row = prefix.checked_mul(columns)?;
    Some((
        usize::try_from(row.checked_add(start)?).ok()?,
        usize::try_from(row.checked_add(end)?).ok()?,
    ))
}

fn bulk_fill_target_range(
    target: &erabasic_bytecode::BytecodeGlobal,
    prefix: i64,
    start: i64,
    end: i64,
) -> Option<(Option<usize>, usize, usize)> {
    if target.storage == BytecodeStorage::Project {
        let (start, end) = bulk_fill_flat_range(&target.dimensions, prefix, start, end)?;
        return Some((None, start, end));
    }
    let &[length] = target.dimensions.as_slice() else {
        return None;
    };
    let character = usize::try_from(prefix).ok()?;
    let start = u64::try_from(start).ok()?;
    let end = u64::try_from(end).ok()?;
    if start > end || end > length {
        return None;
    }
    Some((
        Some(character),
        usize::try_from(start).ok()?,
        usize::try_from(end).ok()?,
    ))
}
