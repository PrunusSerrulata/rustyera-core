#[allow(clippy::wildcard_imports)]
use super::*;
use crate::state::methods::{MethodBinding, ResolvedMethod, resolve_method_call};
use erabasic_bytecode::{MethodArgumentSpec, MethodResult};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeMethodCall {
    pub method: ResolvedMethod,
    pub specs: Vec<MethodArgumentSpec>,
    pub arguments: Vec<Option<Expr>>,
    pub captured: Vec<Option<VmValue>>,
    pub next_slot: usize,
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
    StepError::new(VmFaultCode::TypeMismatch, message)
}

pub(super) fn argument_spec(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: Option<&Expr>,
) -> Result<MethodArgumentSpec, StepError> {
    let Some(expression) = expression else {
        return Ok(MethodArgumentSpec::Omitted);
    };
    let mut expression = expression;
    while let ExprKind::Group(inner) = &expression.kind {
        expression = inner;
    }
    if let ExprKind::Identifier(name) | ExprKind::Variable { name, .. } = &expression.kind {
        let variable = program
            .scoped_variable(function, name)
            .ok_or_else(|| bad_type(format!("STRFORM method variable {name} is missing")))?;
        expression_type(program, function, expression, 0)?;
        return Ok(MethodArgumentSpec::Variable(variable.key));
    }
    Ok(MethodArgumentSpec::Value(expression_type(
        program, function, expression, 0,
    )?))
}

pub(super) fn expression_type(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &Expr,
    depth: usize,
) -> Result<BytecodeType, StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit("STRFORM method type nesting exceeds limit"));
    }
    let nested = |expression| expression_type(program, function, expression, depth + 1);
    match &expression.kind {
        ExprKind::Integer(_) => Ok(BytecodeType::Integer),
        ExprKind::String(_) => Ok(BytecodeType::String),
        ExprKind::Formatted(formatted) => {
            validate_form_types(program, function, formatted, depth + 1)?;
            Ok(BytecodeType::String)
        }
        ExprKind::Identifier(name) | ExprKind::Variable { name, .. } => {
            if let ExprKind::Variable { indices, .. } = &expression.kind {
                for index in indices {
                    if nested(index)? != BytecodeType::Integer {
                        return Err(bad_type("STRFORM variable index must be an integer"));
                    }
                }
            }
            program
                .scoped_variable(function, name)
                .map(|definition| definition.value_type)
                .ok_or_else(|| bad_type(format!("STRFORM variable {name} is missing")))
        }
        ExprKind::Group(inner) => nested(inner),
        ExprKind::Unary { op, operand } => {
            if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
                return Err(unsupported(
                    "STRFORM increment and decrement are not supported",
                ));
            }
            if nested(operand)? != BytecodeType::Integer {
                return Err(bad_type("STRFORM unary operand must be an integer"));
            }
            Ok(BytecodeType::Integer)
        }
        ExprKind::Postfix { .. } => Err(unsupported(
            "STRFORM increment and decrement are not supported",
        )),
        ExprKind::Call { name, args } => {
            for argument in args.iter().flatten() {
                nested(argument)?;
            }
            if let Some(target) = program.function_by_name(name) {
                return target
                    .result
                    .filter(|result| matches!(result, BytecodeType::Integer | BytecodeType::String))
                    .ok_or_else(|| bad_type(format!("STRFORM call {name} has no scalar result")));
            }
            if let Some(result) = method_result(name) {
                return Ok(result.bytecode_type());
            }
            if name.eq_ignore_ascii_case("EXISTMETH") {
                return Ok(BytecodeType::Integer);
            }
            if name.eq_ignore_ascii_case("STRFORM") {
                return Ok(BytecodeType::String);
            }
            program
                .artifact
                .native_imports
                .iter()
                .find(|import| import.import.name.eq_ignore_ascii_case(name))
                .and_then(|import| import.import.result)
                .ok_or_else(|| {
                    bad_type(format!("STRFORM callable {name} has no known result type"))
                })
        }
        ExprKind::Binary { op, left, right } => {
            let left = nested(left)?;
            let right = nested(right)?;
            binary_result_type(*op, left, right)
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            if nested(condition)? != BytecodeType::Integer {
                return Err(bad_type("STRFORM ternary condition must be an integer"));
            }
            let result = nested(then_expr)?;
            if !matches!(result, BytecodeType::Integer | BytecodeType::String)
                || result != nested(else_expr)?
            {
                return Err(bad_type("STRFORM ternary branches differ in type"));
            }
            Ok(result)
        }
        ExprKind::Error => Err(bad_type(
            "STRFORM method argument contains an invalid expression",
        )),
    }
}

fn validate_form_types(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    formatted: &FormattedString,
    depth: usize,
) -> Result<(), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit("STRFORM method type nesting exceeds limit"));
    }
    for part in &formatted.parts {
        match part {
            FormPart::Text(_) | FormPart::Triple { .. } => {}
            FormPart::StringInterpolation {
                expression, width, ..
            }
            | FormPart::IntegerInterpolation {
                expression, width, ..
            } => {
                let expected = if matches!(part, FormPart::StringInterpolation { .. }) {
                    BytecodeType::String
                } else {
                    BytecodeType::Integer
                };
                if expression_type(program, function, expression, depth + 1)? != expected {
                    return Err(bad_type("STRFORM interpolation has an incompatible type"));
                }
                if let Some(width) = width
                    && expression_type(program, function, width, depth + 1)?
                        != BytecodeType::Integer
                {
                    return Err(bad_type("STRFORM interpolation width must be an integer"));
                }
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                if expression_type(program, function, condition, depth + 1)?
                    != BytecodeType::Integer
                {
                    return Err(bad_type("STRFORM conditional must be an integer"));
                }
                validate_form_types(program, function, then_value, depth + 1)?;
                if let Some(else_value) = else_value {
                    validate_form_types(program, function, else_value, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

fn binary_result_type(
    op: BinaryOp,
    left: BytecodeType,
    right: BytecodeType,
) -> Result<BytecodeType, StepError> {
    if matches!(
        op,
        BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
    ) {
        if left == right && matches!(left, BytecodeType::Integer | BytecodeType::String) {
            return Ok(BytecodeType::Integer);
        }
        return Err(bad_type(
            "STRFORM comparison operands must have the same scalar type",
        ));
    }
    if (op == BinaryOp::Add && left == BytecodeType::String && right == BytecodeType::String)
        || (op == BinaryOp::Multiply
            && matches!(
                (left, right),
                (BytecodeType::String, BytecodeType::Integer)
                    | (BytecodeType::Integer, BytecodeType::String)
            ))
    {
        return Ok(BytecodeType::String);
    }
    if left == BytecodeType::Integer && right == BytecodeType::Integer {
        return Ok(BytecodeType::Integer);
    }
    Err(bad_type("STRFORM binary operands must be integers"))
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
                result: result.expect("method result was checked"),
                fallback: arguments.get(1).cloned().flatten(),
                arguments: arguments.get(2..).unwrap_or_default().to_vec(),
            });
        }
        self.work.push(RuntimeFormTask::Evaluate(target.clone()));
        Ok(true)
    }

    pub(super) fn resolve_method(
        &mut self,
        vm: &Vm,
        result: MethodResult,
        fallback: Option<Expr>,
        arguments: Vec<Option<Expr>>,
    ) -> Result<(), StepError> {
        let VmValue::String(name) = self.pop_value("STRFORM method name is missing")? else {
            return Err(bad_type("STRFORM method name must be a string"));
        };
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(
                VmFaultCode::MissingSymbol,
                "STRFORM method generation is missing",
            )
        })?;
        if let Some(fallback) = &fallback
            && expression_type(program, self.function, fallback, 0)? != result.bytecode_type()
        {
            return Err(bad_type("dynamic method fallback has an incompatible type"));
        }
        let specs = arguments
            .iter()
            .map(|argument| argument_spec(program, self.function, argument.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let method = resolve_method_call(program, self.generation, &name, &specs, Some(result))
            .map_err(map_vm_error)?;
        if let Some(method) = method {
            self.work
                .push(RuntimeFormTask::MethodArgument(RuntimeMethodCall {
                    method,
                    captured: vec![None; specs.len()],
                    specs,
                    arguments,
                    next_slot: 0,
                }));
        } else if let Some(fallback) = fallback {
            self.work.push(RuntimeFormTask::Evaluate(fallback));
        } else {
            return Err(StepError::new(
                VmFaultCode::MissingSymbol,
                format!("dynamic method {name} is missing"),
            ));
        }
        Ok(())
    }

    pub(super) fn advance_method_arguments(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        mut call: RuntimeMethodCall,
    ) -> Result<(), StepError> {
        self.validate_method_call(vm, fiber, &call, false)?;
        if call.next_slot == 0 {
            vm.validate_method_references(fiber, self.frame, &call.method, &call.specs)
                .map_err(map_vm_error)?;
        }
        while call.next_slot < call.specs.len() {
            let slot = call.next_slot;
            if matches!(call.specs[slot], MethodArgumentSpec::Omitted) {
                call.next_slot += 1;
                continue;
            }
            if matches!(
                call.method.bindings.get(slot),
                Some(MethodBinding::ArrayReference)
            ) {
                let MethodArgumentSpec::Variable(variable) = call.specs[slot] else {
                    return Err(bad_type("REF argument has no variable identity"));
                };
                let place = vm
                    .method_variable_place(fiber, self.generation, self.frame, variable)
                    .map_err(map_vm_error)?;
                call.captured[slot] = Some(
                    vm.capture_method_argument(
                        fiber,
                        self.frame,
                        &call.method,
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
        vm.invoke_method(fiber, self.frame, &call.method, &call.specs, &call.captured)
            .map_err(map_vm_error)?;
        self.awaiting_user_result = Some(call.method.result.bytecode_type());
        Ok(())
    }

    pub(super) fn validate_method_call(
        &self,
        vm: &Vm,
        fiber: &Fiber,
        call: &RuntimeMethodCall,
        awaiting_argument: bool,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| bad_type("STRFORM generation is missing"))?;
        let target = program
            .function(call.method.function)
            .ok_or_else(|| bad_type("stored STRFORM method is missing"))?;
        if call.method.generation != self.generation
            || call.specs.len() != call.arguments.len()
            || call.specs.len() != call.captured.len()
            || call.next_slot > call.specs.len()
            || (awaiting_argument && call.next_slot == call.specs.len())
            || resolve_method_call(
                program,
                self.generation,
                &target.name,
                &call.specs,
                Some(call.method.result),
            )
            .map_err(map_vm_error)?
            .as_ref()
                != Some(&call.method)
        {
            return Err(bad_type(
                "stored STRFORM method signature or slot state is invalid",
            ));
        }
        for (slot, spec) in call.specs.iter().enumerate() {
            let omitted = matches!(spec, MethodArgumentSpec::Omitted);
            let captured = call.captured[slot].as_ref();
            let argument = call.arguments[slot].as_ref();
            if (captured.is_some() != (slot < call.next_slot && !omitted))
                || (slot < call.next_slot && argument.is_some())
                || (slot == call.next_slot && awaiting_argument && argument.is_some())
            {
                return Err(bad_type("stored STRFORM method has misplaced captures"));
            }
            if let MethodArgumentSpec::Variable(key) = spec {
                let definition = program
                    .global(*key)
                    .ok_or_else(|| bad_type("stored method variable is missing"))?;
                if program
                    .scoped_variable(self.function, &definition.name)
                    .map(|value| value.key)
                    != Some(*key)
                {
                    return Err(bad_type("stored method variable is outside caller scope"));
                }
            }
            if slot >= call.next_slot
                && !(slot == call.next_slot && awaiting_argument)
                && argument_spec(program, self.function, argument)? != *spec
            {
                return Err(bad_type(
                    "stored method expression differs from its argument shape",
                ));
            }
            if let Some(value) = captured {
                if value.value_type() != target.parameters[slot].value_type {
                    return Err(bad_type("stored method capture type differs"));
                }
                if matches!(call.method.bindings[slot], MethodBinding::ArrayReference) {
                    vm.capture_method_argument(
                        fiber,
                        self.frame,
                        &call.method,
                        &call.specs,
                        slot,
                        value.clone(),
                    )
                    .map_err(map_vm_error)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn valid_method_state(&self, vm: &Vm, fiber: &Fiber) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        if let Some(result) = self.awaiting_user_result {
            if !matches!(result, BytecodeType::Integer | BytecodeType::String) {
                return false;
            }
            let Some(owner) = fiber.frames.iter().position(|frame| frame.id == self.frame) else {
                return false;
            };
            if let Some(callee) = fiber.frames.get(owner + 1) {
                if callee.generation != self.generation
                    || program
                        .function(callee.function)
                        .and_then(|function| function.result)
                        != Some(result)
                    || !callee.return_value_to_caller
                {
                    return false;
                }
            } else if fiber.frames[owner].stack.last().map(VmValue::value_type) != Some(result) {
                return false;
            }
        }
        self.work.iter().all(|task| match task {
            RuntimeFormTask::MethodArgument(call) => {
                self.validate_method_call(vm, fiber, call, false).is_ok()
            }
            RuntimeFormTask::CaptureMethodArgument(call) => {
                self.validate_method_call(vm, fiber, call, true).is_ok()
            }
            RuntimeFormTask::ResolveMethod {
                result,
                fallback,
                arguments,
            } => {
                fallback.as_ref().is_none_or(|value| {
                    expression_type(program, self.function, value, 0).ok()
                        == Some(result.bytecode_type())
                }) && arguments
                    .iter()
                    .all(|value| argument_spec(program, self.function, value.as_ref()).is_ok())
            }
            _ => true,
        }) && self.check_resources(vm).is_ok()
    }

    pub(super) fn method_resources(&self) -> Option<(usize, usize)> {
        let mut slots = 0usize;
        let mut bytes = 0usize;
        let mut expressions = Vec::new();
        for task in &self.work {
            match task {
                RuntimeFormTask::MethodArgument(call)
                | RuntimeFormTask::CaptureMethodArgument(call) => {
                    slots = slots
                        .checked_add(call.specs.len())?
                        .checked_add(call.captured.len())?
                        .checked_add(call.arguments.len())?
                        .checked_add(call.method.bindings.len())?;
                    expressions.extend(call.arguments.iter().flatten());
                    for value in call.captured.iter().flatten() {
                        if let VmValue::String(value) = value {
                            bytes = bytes.checked_add(value.len())?;
                        }
                    }
                    for binding in &call.method.bindings {
                        if let MethodBinding::Default(
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
        let (nodes, text_bytes) = retained_expression_resources(expressions)?;
        slots = slots.checked_add(nodes)?;
        bytes = bytes.checked_add(text_bytes)?;
        Some((slots, bytes))
    }
}

fn retained_expression_resources(expressions: Vec<&Expr>) -> Option<(usize, usize)> {
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
