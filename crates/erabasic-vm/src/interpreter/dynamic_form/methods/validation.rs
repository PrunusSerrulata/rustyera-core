#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeFormContinuation {
    pub(in super::super) fn validate_method_call(
        &self,
        vm: &Vm,
        fiber: &Fiber,
        call: &RuntimeUserCall,
        awaiting_argument: bool,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid_state("stored form generation is missing"))?;
        let target = program
            .function(call.call.function)
            .ok_or_else(|| invalid_state("stored form target is missing"))?;
        let retained = call.specs.len().min(call.call.bindings.len());
        let resolved = resolve_user_call(
            program,
            self.generation,
            &target.name,
            &UserCallSpec {
                mode: call.call.mode,
                allow_missing: false,
                missing_target: 0,
                arguments: call.specs.clone(),
            },
        )
        .map_err(|_| invalid_state("stored form user signature cannot resolve"))?;
        if call.reference_scope == 0
            || call.reference_scope >= self.next_reference_scope
            || call.call.generation != self.generation
            || call.specs.len() != call.arguments.len()
            || call.captured.len() != retained
            || call.next_slot > call.specs.len()
            || call.argument_checkpoint.is_some_and(|checkpoint| {
                !matches!(
                    self.checkpoints.last(),
                    Some(state)
                        if state.id == checkpoint
                            && matches!(
                                state.kind,
                                super::checkpoints::FormatCheckpointKind::CallTextArguments(_)
                            )
                )
            })
            || (awaiting_argument && call.next_slot >= retained)
            || resolved.as_ref() != Some(&call.call)
        {
            return Err(invalid_state(
                "stored form user signature/slot state differs",
            ));
        }
        for (slot, spec) in call.specs.iter().enumerate() {
            let retained = slot < call.captured.len();
            let omitted = matches!(spec, UserArgumentSpec::Omitted);
            let captured = call.captured.get(slot).and_then(Option::as_ref);
            let argument = call.arguments[slot].as_ref();
            if captured.is_some() != (slot < call.next_slot && retained && !omitted)
                || (slot < call.next_slot && argument.is_some())
                || (slot == call.next_slot && awaiting_argument && argument.is_some())
            {
                return Err(invalid_state(
                    "stored form user actual/capture position differs",
                ));
            }
            if let UserArgumentSpec::Variable(key) = spec {
                let definition = program
                    .global(*key)
                    .ok_or_else(|| invalid_state("stored form variable is missing"))?;
                if program
                    .scoped_variable(self.function, &definition.name)
                    .map(|definition| definition.key)
                    != Some(*key)
                {
                    return Err(invalid_state(
                        "stored form variable escaped its caller scope",
                    ));
                }
            }
            if slot >= call.next_slot
                && !(slot == call.next_slot && awaiting_argument)
                && self
                    .planned_argument_spec(program, call.plan, argument)
                    .map_err(|_| invalid_state("stored actual cannot be typed"))?
                    != *spec
            {
                return Err(invalid_state("stored form argument shape differs"));
            }
            if let Some(value) = captured {
                if value.value_type() != target.parameters[slot].value_type {
                    return Err(invalid_state("stored form captured type differs"));
                }
                if matches!(
                    call.call.bindings[slot],
                    UserArgumentBinding::ArrayReference
                ) {
                    vm.validate_captured_user_reference(fiber, &call.call, slot, value)
                        .map_err(|_| {
                            invalid_state("stored form captured REF backing is invalid")
                        })?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)] // Snapshot validation keeps all continuation invariants adjacent.
    pub(crate) fn valid_method_state(&self, vm: &Vm, fiber: &Fiber) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        let Some(owner) = fiber.frames.iter().position(|frame| frame.id == self.frame) else {
            return false;
        };
        if self.next_reference_scope == 0
            || !self.valid_call_plans(vm)
            || !self.checkpoints_valid()
            || self.checkpoints.iter().any(|checkpoint| {
                checkpoint.owner_stack_depth > fiber.frames[owner].stack.len()
                    || checkpoint.owner_user_calls != fiber.frames[owner].user_calls.len()
            })
        {
            return false;
        }
        if let Some(wait) = &self.awaiting_user_call {
            if !self.valid_method_sources(program, &wait.call)
                || self
                    .validate_method_call(vm, fiber, &wait.call, false)
                    .is_err()
            {
                return false;
            }
            if let Some(callee) = fiber.frames.get(owner + 1) {
                if !self.valid_child_call(callee)
                    || fiber.frames[owner].stack.len() != wait.owner_stack_depth
                {
                    return false;
                }
            } else {
                let expected = wait.call.call.mode.expected_result();
                if fiber.frames[owner].stack.len()
                    != wait.owner_stack_depth + usize::from(expected.is_some())
                    || expected.is_some_and(|expected| {
                        fiber.frames[owner].stack.last().map(VmValue::value_type) != Some(expected)
                    })
                {
                    return false;
                }
            }
        }
        if self.next_map_call == 0 {
            return false;
        }
        let leases = self.map_leases().collect::<Vec<_>>();
        if leases
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != leases.len()
        {
            return false;
        }
        if self.next_bit_call == 0 {
            return false;
        }
        self.work.iter().all(|task| match task {
            RuntimeFormTask::GateInputHost { plan, key, .. } => {
                program
                    .artifact
                    .manifest
                    .compatibility
                    .supports_snake_input()
                    && self.validate_planned_expression(program, *plan, key)
                    && self.planned_expression_type(*plan, key).ok() == Some(BytecodeType::Integer)
            }
            RuntimeFormTask::FinishInputHost { name, count } => {
                super::input_host::allowed(name)
                    && program
                        .artifact
                        .manifest
                        .compatibility
                        .supports_snake_input()
                    && program.artifact.host_imports.iter().any(|host| {
                        host.import.namespace == "rustyera.input"
                            && host.import.name.eq_ignore_ascii_case(name)
                            && host.import.parameters.len() == *count
                            && host.import.result == Some(BytecodeType::Integer)
                    })
            }
            // Controller Host completions are transient. A stable snapshot cannot
            // invent an unreturned input observation or a gate result stack slot.
            RuntimeFormTask::ReadInputHost { .. } => false,
            RuntimeFormTask::MapCapture {
                bound,
                site,
                arguments,
            } => self.valid_map_binding(program, bound, *site, arguments),
            RuntimeFormTask::MapFinish(call) => self.valid_map_task(vm, fiber, call),
            RuntimeFormTask::MapValuesEnabled { call, output } => {
                self.valid_map_task(vm, fiber, call)
                    && Self::valid_map_output_source(call, output.as_ref())
            }
            RuntimeFormTask::BitCapture { spec, site, source } => {
                self.valid_bit_capture(vm, *spec, *site, source)
            }
            RuntimeFormTask::BitFinish(call) => self.valid_bit_task(vm, call),
            RuntimeFormTask::FinishNative {
                site,
                bound,
                source,
            } => self.valid_native_task(program, *site, bound, source),
            RuntimeFormTask::MethodArgument(call) => {
                self.valid_method_sources(program, call)
                    && self.validate_method_call(vm, fiber, call, false).is_ok()
            }
            RuntimeFormTask::CaptureMethodArgument(call) => {
                self.valid_method_sources(program, call)
                    && self.validate_method_call(vm, fiber, call, true).is_ok()
            }
            RuntimeFormTask::ResolveMethod {
                plan,
                result,
                fallback,
                arguments,
            } => {
                fallback.as_ref().is_none_or(|value| {
                    self.validate_planned_expression(program, *plan, value)
                        && self.planned_expression_type(*plan, value).ok()
                            == Some(result.bytecode_type())
                }) && arguments.iter().all(|value| {
                    value.as_ref().is_none_or(|expression| {
                        self.validate_planned_expression(program, *plan, expression)
                    }) && self
                        .planned_argument_spec(program, *plan, value.as_ref())
                        .is_ok()
                })
            }
            RuntimeFormTask::ExistVarFirst { plan, source, mode } => {
                self.validate_planned_expression(program, *plan, source)
                    && self.planned_expression_type(*plan, source).ok()
                        == Some(BytecodeType::String)
                    && mode.as_ref().is_none_or(|mode| {
                        self.validate_planned_expression(program, *plan, mode)
                            && self.planned_expression_type(*plan, mode).ok()
                                == Some(BytecodeType::Integer)
                    })
            }
            RuntimeFormTask::ExistVarMode { plan, source } => {
                self.validate_planned_expression(program, *plan, source)
                    && self.planned_expression_type(*plan, source).ok()
                        == Some(BytecodeType::String)
            }
            RuntimeFormTask::MutateVariable {
                variable,
                indices,
                mode,
            } => self.valid_mutation_task(vm, *variable, *indices, *mode),
            RuntimeFormTask::ParseCallText { spec, .. } => {
                matches!(self.completion, RuntimeFormRoot::Call { spec: root, .. } if root == *spec)
            }
            _ => true,
        }) && self.check_resources(vm).is_ok()
    }

    fn valid_method_sources(
        &self,
        program: &crate::ProgramGeneration,
        call: &RuntimeUserCall,
    ) -> bool {
        call.arguments
            .iter()
            .flatten()
            .all(|expression| self.validate_planned_expression(program, call.plan, expression))
    }

    #[allow(clippy::too_many_lines)] // Resource accounting mirrors every serialized task variant.
    pub(in super::super) fn method_resources(&self) -> Option<(usize, usize)> {
        let mut slots = 0usize;
        let mut bytes = 0usize;
        let mut expressions = Vec::new();
        for task in &self.work {
            match task {
                RuntimeFormTask::GateInputHost { key, .. }
                | RuntimeFormTask::ReadInputHost {
                    gate: Some((key, _)),
                    ..
                } => expressions.push(key),
                RuntimeFormTask::FinishInputHost { count, .. } => {
                    slots = slots.checked_add(*count)?;
                }
                RuntimeFormTask::MapCapture {
                    bound, arguments, ..
                } => {
                    slots = slots
                        .checked_add(bound.import.parameters.len())?
                        .checked_add(bound.omitted_arguments.len())?;
                    bytes = bytes
                        .checked_add(bound.import.name.len())?
                        .checked_add(bound.import.namespace.len())?;
                    slots = slots.checked_add(arguments.len())?;
                    expressions.extend(arguments.iter().flatten());
                }
                RuntimeFormTask::MapValuesEnabled { call, output } => {
                    slots = slots
                        .checked_add(1)?
                        .checked_add(call.source.len())?
                        .checked_add(call.bound.import.parameters.len())?;
                    bytes = bytes
                        .checked_add(call.name.len())?
                        .checked_add(call.bound.import.name.len())?
                        .checked_add(call.bound.import.namespace.len())?;
                    expressions.extend(call.source.iter().flatten());
                    expressions.extend(output.iter());
                }
                RuntimeFormTask::MapFinish(call) => {
                    slots = slots
                        .checked_add(1)?
                        .checked_add(call.source.len())?
                        .checked_add(call.bound.import.parameters.len())?;
                    bytes = bytes
                        .checked_add(call.name.len())?
                        .checked_add(call.bound.import.name.len())?
                        .checked_add(call.bound.import.namespace.len())?;
                    expressions.extend(call.source.iter().flatten());
                }
                RuntimeFormTask::BitCapture { source, .. } => {
                    slots = slots.checked_add(source.len())?;
                    expressions.extend(source.iter().flatten());
                }
                RuntimeFormTask::BitFinish(call) => {
                    slots = slots.checked_add(1)?.checked_add(call.source.len())?;
                    expressions.extend(call.source.iter().flatten());
                }
                RuntimeFormTask::MatchBegin(call)
                | RuntimeFormTask::MatchEnd(call)
                | RuntimeFormTask::MatchNeedle(call)
                | RuntimeFormTask::MatchScan(call) => {
                    slots = slots.checked_add(12)?.checked_add(call.arguments.len())?;
                    expressions.extend(call.arguments.iter().flatten());
                    if let erabasic_bytecode::MatchInput::Name(name) = &call.spec.input {
                        bytes = bytes.checked_add(name.len())?;
                    }
                    if let Some(VmValue::String(value)) = &call.state.needle {
                        bytes = bytes.checked_add(value.len())?;
                    }
                }
                RuntimeFormTask::FinishNative { bound, source, .. } => {
                    slots = slots
                        .checked_add(source.len())?
                        .checked_add(bound.import.parameters.len())?
                        .checked_add(bound.omitted_arguments.len())?;
                    bytes = bytes
                        .checked_add(bound.import.name.len())?
                        .checked_add(bound.import.namespace.len())?;
                    expressions.extend(source.iter().flatten());
                }
                RuntimeFormTask::ExistVarFirst { source, mode, .. } => {
                    expressions.push(source);
                    expressions.extend(mode.iter());
                }
                RuntimeFormTask::ExistVarMode { source, .. } => expressions.push(source),
                RuntimeFormTask::MethodArgument(call)
                | RuntimeFormTask::CaptureMethodArgument(call) => {
                    slots = slots
                        .checked_add(call.specs.len())?
                        .checked_add(call.captured.len())?
                        .checked_add(call.arguments.len())?
                        .checked_add(call.call.bindings.len())?;
                    expressions.extend(call.arguments.iter().flatten());
                    for value in call.captured.iter().flatten() {
                        if let VmValue::String(value) = value {
                            bytes = bytes.checked_add(value.len())?;
                        }
                    }
                    for binding in &call.call.bindings {
                        if let UserArgumentBinding::Default(
                            erabasic_bytecode::BytecodeConstant::String(value),
                        ) = binding
                        {
                            bytes = bytes.checked_add(value.len())?;
                        }
                    }
                }
                RuntimeFormTask::ResolveMethod {
                    arguments,
                    fallback,
                    ..
                } => {
                    slots = slots.checked_add(arguments.len())?;
                    expressions.extend(arguments.iter().flatten());
                    expressions.extend(fallback.iter());
                }
                _ => {}
            }
        }
        if let Some(wait) = &self.awaiting_user_call {
            let call = &wait.call;
            slots = slots
                .checked_add(call.specs.len())?
                .checked_add(call.arguments.len())?
                .checked_add(call.captured.len())?
                .checked_add(call.call.bindings.len())?;
            expressions.extend(call.arguments.iter().flatten());
            for value in call.captured.iter().flatten() {
                if let VmValue::String(value) = value {
                    bytes = bytes.checked_add(value.len())?;
                }
            }
            for binding in &call.call.bindings {
                if let UserArgumentBinding::Default(erabasic_bytecode::BytecodeConstant::String(
                    value,
                )) = binding
                {
                    bytes = bytes.checked_add(value.len())?;
                }
            }
        }
        for task in &self.work {
            if let RuntimeFormTask::BeginCheckedForm(source)
            | RuntimeFormTask::ParseCallText { source, .. } = task
            {
                bytes = bytes.checked_add(source.len())?;
            }
        }
        let (nodes, text_bytes) = retained_expression_resources(expressions)?;
        slots = slots.checked_add(nodes)?;
        bytes = bytes.checked_add(text_bytes)?;
        Some((slots, bytes))
    }
}
