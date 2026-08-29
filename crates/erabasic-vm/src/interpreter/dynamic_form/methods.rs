#[allow(clippy::wildcard_imports)]
use super::*;
use crate::state::user_calls::{
    ResolvedUserCall, UserArgumentBinding, UserCallOrigin, resolve_user_call,
};
use erabasic_bytecode::{MethodResult, UserArgumentSpec, UserCallMode, UserCallSpec};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeUserCall {
    pub reference_bindings: bool,
    pub plan: u64,
    pub call: ResolvedUserCall,
    pub specs: Vec<UserArgumentSpec>,
    pub arguments: Vec<Option<Expr>>,
    pub captured: Vec<Option<VmValue>>,
    pub next_slot: usize,
    pub argument_checkpoint: Option<u64>,
}

pub(super) fn method_result(name: &str) -> Option<MethodResult> {
    if name.eq_ignore_ascii_case("GETMETH") {
        Some(MethodResult::Integer)
    } else if name.eq_ignore_ascii_case("GETMETHS") {
        Some(MethodResult::String)
    } else {
        None
    }
}

fn bad_type(message: impl Into<String>) -> StepError {
    StepError::script(
        crate::ScriptFaultKind::Argument,
        VmFaultCode::TypeMismatch,
        message,
    )
}

fn invalid_state(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeUserWait {
    pub call: RuntimeUserCall,
    pub callee: FrameId,
    pub owner_stack_depth: usize,
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_method(
        &mut self,
        name: &str,
        arguments: &[Option<Expr>],
    ) -> Result<bool, StepError> {
        let result = method_result(name);
        let exists = name.eq_ignore_ascii_case("EXISTMETH");
        if result.is_none() && !exists {
            return Ok(false);
        }
        let target = arguments
            .first()
            .and_then(Option::as_ref)
            .ok_or_else(|| bad_type("dynamic method name cannot be omitted"))?;
        if exists {
            if arguments.len() != 1 {
                return Err(bad_type("EXISTMETH expects one argument"));
            }
            self.work.push(RuntimeFormTask::ExistsMethod);
        } else {
            self.work.push(RuntimeFormTask::ResolveMethod {
                plan: self
                    .current_call_plan
                    .ok_or_else(|| invalid_state("method lacks its source plan"))?,
                result: result.expect("method result was checked"),
                fallback: arguments.get(1).cloned().flatten(),
                arguments: arguments.get(2..).unwrap_or_default().to_vec(),
            });
        }
        self.work.push(RuntimeFormTask::Evaluate(target.clone()));
        Ok(true)
    }

    pub(super) fn schedule_direct_user_call(
        &mut self,
        vm: &Vm,
        name: &str,
        arguments: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid_state("form generation is missing"))?;
        let target = program
            .function_by_name(name)
            .ok_or_else(|| invalid_state("known direct form method disappeared"))?;
        let result = match target.result {
            Some(BytecodeType::Integer) => MethodResult::Integer,
            Some(BytecodeType::String) => MethodResult::String,
            _ => return Err(bad_type("direct form method has no scalar result")),
        };
        self.values.push(VmValue::String(name.into()));
        self.work.push(RuntimeFormTask::ResolveMethod {
            plan: self
                .current_call_plan
                .ok_or_else(|| invalid_state("method lacks its source plan"))?,
            result,
            fallback: None,
            arguments,
        });
        Ok(())
    }

    pub(super) fn resolve_method(
        &mut self,
        vm: &mut Vm,
        plan: u64,
        result: MethodResult,
        fallback: Option<Expr>,
        arguments: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        let VmValue::String(name) = self.pop_value("form method name is missing")? else {
            return Err(bad_type("form method name must be a string"));
        };
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid_state("form generation is missing"))?;
        if let Some(fallback) = &fallback
            && self.planned_expression_type(plan, fallback)? != result.bytecode_type()
        {
            return Err(bad_type("dynamic method fallback has an incompatible type"));
        }
        // Parse/type/name checks cover the full syntactic list before selecting the retained prefix.
        let specs = arguments
            .iter()
            .map(|argument| self.planned_argument_spec(program, plan, argument.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let call = resolve_user_call(
            program,
            self.generation,
            &name,
            &UserCallSpec {
                mode: result.into(),
                allow_missing: fallback.is_some(),
                missing_target: 0,
                arguments: specs.clone(),
            },
        )
        .map_err(map_vm_error)?;
        if let Some(call) = call {
            self.queue_resolved_call(vm, call, specs, arguments)?;
        } else if let Some(fallback) = fallback {
            self.work.push(RuntimeFormTask::Evaluate(fallback));
        } else {
            return Err(StepError::script(
                crate::ScriptFaultKind::Resolve,
                VmFaultCode::MissingSymbol,
                format!("dynamic method {name} is missing"),
            ));
        }
        Ok(())
    }

    pub(super) fn queue_resolved_call(
        &mut self,
        vm: &mut Vm,
        call: ResolvedUserCall,
        specs: Vec<UserArgumentSpec>,
        arguments: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        vm.queue_user_call_diagnostic(&call, specs.len());
        let retained = specs.len().min(call.bindings.len());
        self.work
            .push(RuntimeFormTask::MethodArgument(RuntimeUserCall {
                reference_bindings: self.reference_bindings,
                plan: self
                    .current_call_plan
                    .ok_or_else(|| invalid_state("user call lacks its source plan"))?,
                call,
                captured: vec![None; retained],
                specs,
                arguments,
                next_slot: 0,
                argument_checkpoint: None,
            }));
        Ok(())
    }

    pub(super) fn advance_method_arguments(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        mut call: RuntimeUserCall,
    ) -> Result<(), StepError> {
        self.validate_method_call(vm, fiber, &call, false)?;
        while call.next_slot < call.specs.len() {
            let slot = call.next_slot;
            if slot >= call.captured.len() || matches!(call.specs[slot], UserArgumentSpec::Omitted)
            {
                call.arguments[slot] = None;
                call.next_slot += 1;
                continue;
            }
            if matches!(
                call.call.bindings.get(slot),
                Some(UserArgumentBinding::ArrayReference)
            ) {
                let UserArgumentSpec::Variable(variable) = call.specs[slot] else {
                    return Err(bad_type("REF argument has no variable identity"));
                };
                let place = vm
                    .user_call_variable_place(fiber, self.generation, self.frame, variable)
                    .map_err(map_vm_error)?;
                call.captured[slot] = Some(
                    vm.capture_user_argument(
                        fiber,
                        self.frame,
                        &call.call,
                        &call.specs,
                        slot,
                        place,
                    )
                    .map_err(map_vm_error)?,
                );
                call.arguments[slot] = None;
                call.next_slot += 1;
            } else {
                let expression = call.arguments[slot]
                    .take()
                    .ok_or_else(|| bad_type("method actual expression is missing"))?;
                self.work.push(RuntimeFormTask::CaptureMethodArgument(call));
                self.work.push(RuntimeFormTask::Evaluate(expression));
                return Ok(());
            }
        }
        if let Some(checkpoint) = call.argument_checkpoint {
            self.finish_call_text_argument_checkpoint(checkpoint)?;
            call.argument_checkpoint = None;
        }
        let owner_stack_depth = owner_frame(fiber, self.frame)?.stack.len();
        vm.invoke_user_call(
            fiber,
            self.frame,
            &call.call,
            &call.specs,
            &call.captured,
            UserCallOrigin::RuntimeForm,
        )
        .map_err(map_vm_error)?;
        let callee = fiber
            .frames
            .last()
            .ok_or_else(|| invalid_state("user-call did not create a callee"))?
            .id;
        self.awaiting_user_call = Some(RuntimeUserWait {
            call,
            callee,
            owner_stack_depth,
        });
        Ok(())
    }

    pub(super) fn finish_user_wait(&mut self, vm: &Vm, fiber: &mut Fiber) -> Result<(), StepError> {
        let wait = self
            .awaiting_user_call
            .as_ref()
            .ok_or_else(|| invalid_state("form user wait is missing"))?;
        if fiber.frames.iter().any(|frame| frame.id == wait.callee) {
            return Err(invalid_state("form resumed before its callee returned"));
        }
        self.validate_method_call(vm, fiber, &wait.call, false)?;
        let owner = owner_frame_mut(fiber, self.frame)?;
        let expected = wait.call.call.mode.expected_result();
        if owner.stack.len() != wait.owner_stack_depth + usize::from(expected.is_some()) {
            return Err(invalid_state("form callee result boundary differs"));
        }
        if let Some(expected) = expected {
            let value = owner
                .stack
                .pop()
                .ok_or_else(|| invalid_state("form callee result is missing"))?;
            if value.value_type() != expected {
                return Err(invalid_state("form callee result type differs"));
            }
            self.values.push(value);
        }
        self.awaiting_user_call = None;
        Ok(())
    }

    pub(crate) fn expected_child(&self) -> Option<FrameId> {
        self.awaiting_user_call.as_ref().map(|wait| wait.callee)
    }

    pub(crate) fn valid_child_call(&self, callee: &crate::state::Frame) -> bool {
        let Some(wait) = &self.awaiting_user_call else {
            return false;
        };
        let permitted_mode = match wait.call.call.mode {
            UserCallMode::MethodInteger | UserCallMode::MethodString => true,
            UserCallMode::Procedure | UserCallMode::JumpProcedure => matches!(self.completion,
                RuntimeFormRoot::Call { spec, .. } if spec.mode.user_call_mode() == wait.call.call.mode),
            UserCallMode::MethodDiscard => false,
        };
        permitted_mode
            && callee.id == wait.callee
            && callee.generation == wait.call.call.generation
            && callee.function == wait.call.call.function
            && callee.user_call.as_ref().is_some_and(|origin| {
                origin.caller == self.frame
                    && origin.mode == wait.call.call.mode
                    && origin.origin == UserCallOrigin::RuntimeForm
            })
    }

    pub(super) fn validate_method_call(
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
        if call.call.generation != self.generation
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

    pub(crate) fn valid_method_state(&self, vm: &Vm, fiber: &Fiber) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        let Some(owner) = fiber.frames.iter().position(|frame| frame.id == self.frame) else {
            return false;
        };
        if !self.valid_call_plans(vm)
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
        self.work.iter().all(|task| match task {
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

    pub(super) fn method_resources(&self) -> Option<(usize, usize)> {
        let mut slots = 0usize;
        let mut bytes = 0usize;
        let mut expressions = Vec::new();
        for task in &self.work {
            match task {
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

pub(super) fn retained_expression_resources(expressions: Vec<&Expr>) -> Option<(usize, usize)> {
    enum Node<'a> {
        Expression(&'a Expr),
        Form(&'a FormattedString),
    }
    let mut pending = expressions
        .into_iter()
        .map(Node::Expression)
        .collect::<Vec<_>>();
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some(node) = pending.pop() {
        nodes = nodes.checked_add(1)?;
        match node {
            Node::Expression(expression) => match &expression.kind {
                ExprKind::Integer(_) | ExprKind::Error => {}
                ExprKind::String(value) | ExprKind::Identifier(value) => {
                    bytes = bytes.checked_add(value.len())?;
                }
                ExprKind::Variable { name, indices } => {
                    bytes = bytes.checked_add(name.len())?;
                    pending.extend(indices.iter().map(Node::Expression));
                }
                ExprKind::Call { name, args } => {
                    bytes = bytes.checked_add(name.len())?;
                    pending.extend(args.iter().flatten().map(Node::Expression));
                }
                ExprKind::Unary { operand, .. }
                | ExprKind::Postfix { operand, .. }
                | ExprKind::Group(operand) => pending.push(Node::Expression(operand)),
                ExprKind::Binary { left, right, .. } => {
                    pending.push(Node::Expression(left));
                    pending.push(Node::Expression(right));
                }
                ExprKind::Ternary {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    pending.extend(
                        [condition.as_ref(), then_expr.as_ref(), else_expr.as_ref()]
                            .map(Node::Expression),
                    );
                }
                ExprKind::Formatted(formatted) => pending.push(Node::Form(formatted)),
            },
            Node::Form(formatted) => {
                for part in &formatted.parts {
                    nodes = nodes.checked_add(1)?;
                    match part {
                        FormPart::Text(value) => bytes = bytes.checked_add(value.len())?,
                        FormPart::Triple { .. } => {}
                        FormPart::StringInterpolation {
                            expression, width, ..
                        }
                        | FormPart::IntegerInterpolation {
                            expression, width, ..
                        } => {
                            pending.push(Node::Expression(expression));
                            pending.extend(width.iter().map(|value| Node::Expression(value)));
                        }
                        FormPart::Conditional {
                            condition,
                            then_value,
                            else_value,
                            ..
                        } => {
                            pending.push(Node::Expression(condition));
                            pending.push(Node::Form(then_value));
                            pending.extend(else_value.iter().map(|value| Node::Form(value)));
                        }
                    }
                }
            }
        }
    }
    Some((nodes, bytes))
}
