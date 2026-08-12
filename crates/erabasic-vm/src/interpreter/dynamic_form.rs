use std::sync::Arc;

use erabasic_ast::{Alignment, BinaryOp, Expr, ExprKind, FormPart, FormattedString, UnaryOp};
use erabasic_bytecode::{BytecodeFunctionKind, BytecodeStorage, BytecodeType, SymbolKey};
use erabasic_parser::{DefaultParserContext, parse_formatted_at};
use serde::{Deserialize, Serialize};

use super::{StepError, binary_value, map_vm_error, unary_value};
use crate::{
    Fiber, FrameId, GenerationId, HostReady, NativeServiceRegistry, Vm, VmFaultCode, VmValue,
    bind_persistent_arguments, make_frame, prepare_dynamic_arguments,
};

const MAX_RUNTIME_FORM_BYTES: usize = 1024 * 1024;
const MAX_RUNTIME_FORM_NESTING: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct RuntimeFormContinuation {
    generation: GenerationId,
    function: SymbolKey,
    frame: FrameId,
    instruction: usize,
    work: Vec<RuntimeFormTask>,
    values: Vec<VmValue>,
    outputs: Vec<String>,
    awaiting_user_result: bool,
    remaining_nodes: usize,
    remaining_source_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum RuntimeFormTask {
    StartForm(FormattedString),
    RenderForm(FormattedString),
    RenderPart(FormPart),
    FinishFormValue,
    CompleteRoot,
    Evaluate(Expr),
    ReadVariable {
        name: String,
        indices: usize,
    },
    ApplyUnary(UnaryOp),
    EvaluateBinaryRight {
        op: BinaryOp,
        right: Expr,
    },
    ApplyBinary(BinaryOp),
    ChooseTernary {
        then_expr: Expr,
        else_expr: Expr,
    },
    FinishCall {
        name: String,
        arguments: usize,
    },
    FinishInterpolation {
        string: bool,
        width: bool,
        alignment: Option<Alignment>,
    },
    ChooseConditional {
        then_value: FormattedString,
        else_value: Option<FormattedString>,
    },
    PushOmitted,
}

pub(super) enum RuntimeFormStep {
    Pending,
    Complete(String),
}

pub(crate) fn requires_runtime_form_context(source: &str) -> bool {
    source.contains(['%', '{', '}', '\\'])
        || ["***", "+++", "===", "///", "$$$"]
            .iter()
            .any(|symbol| source.contains(symbol))
}

pub(super) fn begin_runtime_form(
    vm: &Vm,
    fiber: &mut Fiber,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    instruction: usize,
    source: &str,
) -> Result<(), StepError> {
    let frame = fiber.frames.last().ok_or_else(|| {
        StepError::new(
            VmFaultCode::InvalidInstruction,
            "STRFORM caller frame is missing",
        )
    })?;
    if frame.runtime_form.is_some() {
        return Err(StepError::new(
            VmFaultCode::InvalidInstruction,
            "STRFORM caller already owns a continuation",
        ));
    }
    let node_limit = vm.config.maximum_operand_stack.max(1);
    let (formatted, nodes) =
        parse_runtime_form(vm, natives, generation, function, source, node_limit)?;
    let continuation = RuntimeFormContinuation {
        generation,
        function,
        frame: frame.id,
        instruction,
        work: vec![
            RuntimeFormTask::CompleteRoot,
            RuntimeFormTask::StartForm(formatted),
        ],
        values: Vec::new(),
        outputs: Vec::new(),
        awaiting_user_result: false,
        remaining_nodes: node_limit.saturating_sub(nodes),
        remaining_source_bytes: MAX_RUNTIME_FORM_BYTES.saturating_sub(source.len()),
    };
    fiber
        .frames
        .last_mut()
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM caller frame is missing",
            )
        })?
        .runtime_form = Some(continuation);
    Ok(())
}

pub(super) fn resume_runtime_form(
    vm: &mut Vm,
    fiber: &mut Fiber,
    natives: &mut NativeServiceRegistry,
) -> Result<RuntimeFormStep, StepError> {
    let owner = fiber.frames.last().ok_or_else(|| {
        StepError::new(
            VmFaultCode::InvalidInstruction,
            "STRFORM continuation frame is missing",
        )
    })?;
    let owner_id = owner.id;
    let mut continuation = fiber
        .frames
        .last_mut()
        .filter(|frame| frame.id == owner_id)
        .and_then(|frame| frame.runtime_form.take())
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM continuation is missing",
            )
        })?;

    let result = continuation.step(vm, fiber, natives);
    match result {
        Ok(RuntimeFormStep::Complete(value)) => Ok(RuntimeFormStep::Complete(value)),
        Ok(RuntimeFormStep::Pending) => {
            let frame = fiber
                .frames
                .iter_mut()
                .find(|frame| frame.id == continuation.frame)
                .ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "STRFORM owner frame disappeared",
                    )
                })?;
            if frame.runtime_form.replace(continuation).is_some() {
                return Err(StepError::new(
                    VmFaultCode::InvalidInstruction,
                    "STRFORM owner acquired a second continuation",
                ));
            }
            Ok(RuntimeFormStep::Pending)
        }
        Err(error) => Err(error),
    }
}

impl RuntimeFormContinuation {
    pub(crate) const fn origin(&self) -> (GenerationId, SymbolKey, usize) {
        (self.generation, self.function, self.instruction)
    }

    pub(crate) fn valid_for_frame(
        &self,
        generation: GenerationId,
        function: SymbolKey,
        frame: FrameId,
        maximum_stack: usize,
    ) -> bool {
        self.generation == generation
            && self.function == function
            && self.frame == frame
            && self.work.len() <= maximum_stack
            && self.values.len() <= maximum_stack
            && self.outputs.len() <= MAX_RUNTIME_FORM_NESTING
            && self.remaining_nodes <= maximum_stack
            && self.remaining_source_bytes <= MAX_RUNTIME_FORM_BYTES
    }

    // Keeping the continuation transition table together makes every resumable state auditable.
    #[allow(clippy::too_many_lines)]
    fn step(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
    ) -> Result<RuntimeFormStep, StepError> {
        if self.awaiting_user_result {
            let owner = owner_frame_mut(fiber, self.frame)?;
            let value = owner.stack.pop().ok_or_else(|| {
                StepError::new(
                    VmFaultCode::InvalidInstruction,
                    "STRFORM user function result is missing",
                )
            })?;
            self.values.push(value);
            self.awaiting_user_result = false;
            self.check_resources(vm)?;
            return Ok(RuntimeFormStep::Pending);
        }

        let task = self.work.pop().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM continuation ended without a result",
            )
        })?;
        match task {
            RuntimeFormTask::StartForm(formatted) => {
                if self.outputs.len() >= MAX_RUNTIME_FORM_NESTING {
                    return Err(resource_limit("STRFORM nesting limit exceeded"));
                }
                self.outputs.push(String::new());
                self.work.push(RuntimeFormTask::FinishFormValue);
                self.work.push(RuntimeFormTask::RenderForm(formatted));
            }
            RuntimeFormTask::RenderForm(formatted) => {
                self.work.extend(
                    formatted
                        .parts
                        .into_iter()
                        .rev()
                        .map(RuntimeFormTask::RenderPart),
                );
            }
            RuntimeFormTask::RenderPart(part) => self.render_part(vm, fiber, part)?,
            RuntimeFormTask::FinishFormValue => {
                let value = self.outputs.pop().ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "STRFORM output stack is empty",
                    )
                })?;
                self.values.push(VmValue::String(value));
            }
            RuntimeFormTask::CompleteRoot => {
                if !self.outputs.is_empty() || self.values.len() != 1 {
                    return Err(StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "STRFORM root produced an invalid value stack",
                    ));
                }
                let Some(VmValue::String(value)) = self.values.pop() else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "STRFORM root did not produce a string",
                    ));
                };
                return Ok(RuntimeFormStep::Complete(value));
            }
            RuntimeFormTask::Evaluate(expression) => {
                self.evaluate_expression(expression)?;
            }
            RuntimeFormTask::ReadVariable { name, indices } => {
                let indices = self.take_indices(indices)?;
                self.values
                    .push(self.read_variable(vm, fiber, &name, &indices)?);
            }
            RuntimeFormTask::ApplyUnary(op) => {
                let value = self.pop_value("STRFORM unary operand is missing")?;
                self.values.push(unary_value(unary_tag(op), value)?);
            }
            RuntimeFormTask::EvaluateBinaryRight { op, right } => {
                let left = self.pop_value("STRFORM binary left operand is missing")?;
                if let VmValue::Integer(left_value) = left
                    && matches!(
                        op,
                        BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::Nand | BinaryOp::Nor
                    )
                {
                    let short_circuit = matches!(op, BinaryOp::LogicalAnd | BinaryOp::Nand)
                        && left_value == 0
                        || matches!(op, BinaryOp::LogicalOr | BinaryOp::Nor) && left_value != 0;
                    if short_circuit {
                        let value = match op {
                            BinaryOp::Nand => i64::from(left_value == 0),
                            BinaryOp::LogicalOr => i64::from(left_value != 0),
                            BinaryOp::LogicalAnd | BinaryOp::Nor => 0,
                            _ => {
                                return Err(StepError::new(
                                    VmFaultCode::InvalidInstruction,
                                    "STRFORM logical short-circuit state is invalid",
                                ));
                            }
                        };
                        self.values.push(VmValue::Integer(value));
                    } else {
                        self.values.push(VmValue::Integer(left_value));
                        self.work.push(RuntimeFormTask::ApplyBinary(op));
                        self.work.push(RuntimeFormTask::Evaluate(right));
                    }
                } else {
                    self.values.push(left);
                    self.work.push(RuntimeFormTask::ApplyBinary(op));
                    self.work.push(RuntimeFormTask::Evaluate(right));
                }
            }
            RuntimeFormTask::ApplyBinary(op) => {
                let right = self.pop_value("STRFORM binary right operand is missing")?;
                let left = self.pop_value("STRFORM binary left operand is missing")?;
                self.values.push(binary_value(binary_tag(op), left, right)?);
            }
            RuntimeFormTask::ChooseTernary {
                then_expr,
                else_expr,
            } => {
                let condition = self.pop_integer("STRFORM ternary condition is not an integer")?;
                self.work.push(RuntimeFormTask::Evaluate(if condition != 0 {
                    then_expr
                } else {
                    else_expr
                }));
            }
            RuntimeFormTask::FinishCall { name, arguments } => {
                let arguments = self.take_values(arguments)?;
                self.finish_call(vm, fiber, natives, &name, arguments)?;
            }
            RuntimeFormTask::FinishInterpolation {
                string,
                width,
                alignment,
            } => {
                let width = width
                    .then(|| self.pop_integer("STRFORM width expects an integer"))
                    .transpose()?;
                let value = self.pop_value("STRFORM interpolation value is missing")?;
                let value = match (string, value) {
                    (true, VmValue::String(value)) => value,
                    (false, VmValue::Integer(value)) => value.to_string(),
                    (true, _) => {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "STRFORM string interpolation expects a string",
                        ));
                    }
                    (false, _) => {
                        return Err(StepError::new(
                            VmFaultCode::TypeMismatch,
                            "STRFORM integer interpolation expects an integer",
                        ));
                    }
                };
                let width_value = width.map(VmValue::Integer);
                let alignment_value = width_value
                    .as_ref()
                    .map(|_| VmValue::Integer(i64::from(alignment == Some(Alignment::Left))));
                let value = crate::host::apply_width_with_mode(
                    &value,
                    width_value.as_ref(),
                    alignment_value.as_ref(),
                    natives.character_width_mode(),
                )
                .map_err(|message| StepError::new(VmFaultCode::Native, message))?;
                self.append_output(&value)?;
            }
            RuntimeFormTask::ChooseConditional {
                then_value,
                else_value,
            } => {
                let condition = self.pop_integer("STRFORM conditional expects an integer")?;
                if condition != 0 {
                    self.work.push(RuntimeFormTask::RenderForm(then_value));
                } else if let Some(else_value) = else_value {
                    self.work.push(RuntimeFormTask::RenderForm(else_value));
                }
            }
            RuntimeFormTask::PushOmitted => self.values.push(VmValue::Integer(i64::MIN)),
        }
        self.check_resources(vm)?;
        Ok(RuntimeFormStep::Pending)
    }

    fn render_part(&mut self, vm: &Vm, fiber: &Fiber, part: FormPart) -> Result<(), StepError> {
        match part {
            FormPart::Text(value) => self.append_output(&value)?,
            FormPart::IntegerInterpolation {
                expression,
                width,
                alignment,
                ..
            } => {
                let has_width = width.is_some();
                self.work.push(RuntimeFormTask::FinishInterpolation {
                    string: false,
                    width: has_width,
                    alignment,
                });
                if let Some(width) = width {
                    self.work.push(RuntimeFormTask::Evaluate(*width));
                }
                self.work.push(RuntimeFormTask::Evaluate(*expression));
            }
            FormPart::StringInterpolation {
                expression,
                width,
                alignment,
                ..
            } => {
                let has_width = width.is_some();
                self.work.push(RuntimeFormTask::FinishInterpolation {
                    string: true,
                    width: has_width,
                    alignment,
                });
                if let Some(width) = width {
                    self.work.push(RuntimeFormTask::Evaluate(*width));
                }
                self.work.push(RuntimeFormTask::Evaluate(*expression));
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                self.work.push(RuntimeFormTask::ChooseConditional {
                    then_value: *then_value,
                    else_value: else_value.map(|value| *value),
                });
                self.work.push(RuntimeFormTask::Evaluate(*condition));
            }
            FormPart::Triple { symbol, .. } => {
                let (value, index) = match symbol {
                    '*' => ("NAME", "TARGET"),
                    '+' => ("CALLNAME", "MASTER"),
                    '=' => ("CALLNAME", "PLAYER"),
                    '/' => ("NAME", "ASSI"),
                    '$' => ("CALLNAME", "TARGET"),
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::Native,
                            format!("STRFORM triple symbol {symbol:?} is unsupported"),
                        ));
                    }
                };
                let index = self.read_variable(vm, fiber, index, &[])?;
                let VmValue::Integer(index) = index else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "STRFORM triple index is not an integer",
                    ));
                };
                let index = u64::try_from(index).map_err(|_| {
                    StepError::new(VmFaultCode::Bounds, "STRFORM triple index is negative")
                })?;
                let value = self.read_variable(vm, fiber, value, &[index])?;
                let VmValue::String(value) = value else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "STRFORM triple value is not a string",
                    ));
                };
                self.append_output(&value)?;
            }
        }
        Ok(())
    }

    fn evaluate_expression(&mut self, expression: Expr) -> Result<(), StepError> {
        match expression.kind {
            ExprKind::Integer(value) => self.values.push(VmValue::Integer(value)),
            ExprKind::String(value) => self.values.push(VmValue::String(value)),
            ExprKind::Identifier(name) => {
                self.work
                    .push(RuntimeFormTask::ReadVariable { name, indices: 0 });
            }
            ExprKind::Variable { name, indices } => {
                let count = indices.len();
                self.work.push(RuntimeFormTask::ReadVariable {
                    name,
                    indices: count,
                });
                self.work
                    .extend(indices.into_iter().rev().map(RuntimeFormTask::Evaluate));
            }
            ExprKind::Group(inner) => self.work.push(RuntimeFormTask::Evaluate(*inner)),
            ExprKind::Unary { op, operand } => {
                if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
                    return Err(unsupported("STRFORM increment expressions are unsupported"));
                }
                self.work.push(RuntimeFormTask::ApplyUnary(op));
                self.work.push(RuntimeFormTask::Evaluate(*operand));
            }
            ExprKind::Postfix { .. } => {
                return Err(unsupported("STRFORM increment expressions are unsupported"));
            }
            ExprKind::Binary { op, left, right } => {
                self.work
                    .push(RuntimeFormTask::EvaluateBinaryRight { op, right: *right });
                self.work.push(RuntimeFormTask::Evaluate(*left));
            }
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.work.push(RuntimeFormTask::ChooseTernary {
                    then_expr: *then_expr,
                    else_expr: *else_expr,
                });
                self.work.push(RuntimeFormTask::Evaluate(*condition));
            }
            ExprKind::Call { name, args } => {
                let count = args.len();
                self.work.push(RuntimeFormTask::FinishCall {
                    name,
                    arguments: count,
                });
                self.work.extend(args.into_iter().rev().map(|argument| {
                    argument.map_or(RuntimeFormTask::PushOmitted, RuntimeFormTask::Evaluate)
                }));
            }
            ExprKind::Formatted(formatted) => {
                self.work.push(RuntimeFormTask::StartForm(formatted));
            }
            ExprKind::Error => {
                return Err(unsupported("STRFORM contains an invalid expression"));
            }
        }
        Ok(())
    }

    // Native and user-call completion share one exactly-once transition boundary.
    #[allow(clippy::too_many_lines)]
    fn finish_call(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
        name: &str,
        arguments: Vec<VmValue>,
    ) -> Result<(), StepError> {
        let generation = Arc::clone(vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
        })?);
        if let Some(target) = generation.function_by_name(name).cloned() {
            if target.kind != BytecodeFunctionKind::Method || target.result.is_none() {
                return Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    format!("STRFORM target {name} is not a value-returning function"),
                ));
            }
            if target
                .parameters
                .iter()
                .any(|parameter| parameter.by_reference)
            {
                return Err(unsupported(format!(
                    "STRFORM target {name} requires a reference argument"
                )));
            }
            if fiber.frames.len() >= vm.config.maximum_call_depth {
                return Err(resource_limit(
                    "maximum call depth exceeded during STRFORM expansion",
                ));
            }
            let arguments = prepare_dynamic_arguments(
                &target,
                arguments,
                generation.artifact.call_compatibility,
            )
            .map_err(map_vm_error)?;
            vm.memory.ensure_function_statics(
                self.generation,
                target.key,
                generation.function_statics(target.key),
            );
            bind_persistent_arguments(
                &mut vm.memory,
                self.generation,
                &target,
                &generation,
                &arguments,
            )
            .map_err(map_vm_error)?;
            let event_context = owner_frame(fiber, self.frame)?.event_context;
            let frame_id = vm.allocate_frame_id();
            fiber.frames.push(make_frame(
                frame_id,
                self.generation,
                &target,
                generation.function_locals(target.key),
                arguments,
                true,
                event_context,
            ));
            self.awaiting_user_result = true;
            return Ok(());
        }

        if name.eq_ignore_ascii_case("STRFORM") {
            let [VmValue::String(source)] = arguments.as_slice() else {
                return Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "STRFORM expects one string argument",
                ));
            };
            if source.len() > self.remaining_source_bytes {
                return Err(resource_limit(
                    "nested STRFORM sources exceed the runtime parser limit",
                ));
            }
            let (formatted, nodes) = parse_runtime_form(
                vm,
                natives,
                self.generation,
                self.function,
                source,
                self.remaining_nodes,
            )?;
            self.remaining_nodes = self.remaining_nodes.saturating_sub(nodes);
            self.remaining_source_bytes = self.remaining_source_bytes.saturating_sub(source.len());
            self.work.push(RuntimeFormTask::StartForm(formatted));
            return Ok(());
        }

        let native = generation
            .artifact
            .native_imports
            .iter()
            .map(|native| &native.import)
            .find(|native| {
                native.name.eq_ignore_ascii_case(name)
                    && native.result.is_some()
                    && native.parameters.len() == arguments.len()
                    && native
                        .parameters
                        .iter()
                        .zip(&arguments)
                        .all(|(expected, value)| *expected == value.value_type())
                    && natives.contains(native.key)
            })
            .cloned()
            .ok_or_else(|| {
                StepError::new(
                    VmFaultCode::MissingSymbol,
                    format!("STRFORM callable {name} has no compatible runtime import"),
                )
            })?;
        if matches!(
            native.result,
            Some(BytecodeType::IntegerPlace | BytecodeType::StringPlace)
        ) {
            return Err(unsupported(format!(
                "STRFORM callable {name} returns a reference"
            )));
        }
        let expected = native.result;
        let owner_stack = owner_frame(fiber, self.frame)?.stack.len();
        let (ready, rollback) =
            vm.call_registered_native(fiber, native.key, native.clone(), arguments, natives)?;
        let commit = super::validate_native_ready(vm, fiber, expected, &ready).and_then(|()| {
            vm.apply_host_ready(
                fiber,
                expected,
                HostReady {
                    value: ready.value,
                    writes: ready.writes,
                },
            )
        });
        if let Err(error) = commit {
            if let Some(checkpoint) = rollback {
                let _ = natives.rollback(native.key, &checkpoint);
            }
            return Err(map_vm_error(error));
        }
        let owner = owner_frame_mut(fiber, self.frame)?;
        if owner.stack.len() != owner_stack.saturating_add(1) {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM native did not return exactly one value",
            ));
        }
        self.values.push(owner.stack.pop().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM native result is missing",
            )
        })?);
        Ok(())
    }

    fn read_variable(
        &self,
        vm: &Vm,
        fiber: &Fiber,
        name: &str,
        indices: &[u64],
    ) -> Result<VmValue, StepError> {
        let generation = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
        })?;
        let definition = generation
            .scoped_variable(self.function, name)
            .ok_or_else(|| {
                StepError::new(
                    VmFaultCode::MissingSymbol,
                    format!("STRFORM variable {name} is missing"),
                )
            })?;
        let character = if definition.storage == BytecodeStorage::Character {
            Some(if indices.len() > definition.dimensions.len() {
                indices[0]
            } else {
                vm.target_character_for_generation(self.generation) as u64
            })
        } else {
            None
        };
        let value_indices = if character.is_some() && indices.len() > definition.dimensions.len() {
            &indices[1..]
        } else {
            indices
        };
        vm.read_variable_resolved(
            fiber,
            self.generation,
            definition,
            value_indices,
            character,
            (definition.storage == BytecodeStorage::FunctionLocal).then_some(self.frame),
        )
        .map_err(map_vm_error)
    }

    fn take_indices(&mut self, count: usize) -> Result<Vec<u64>, StepError> {
        self.take_values(count)?
            .into_iter()
            .map(|value| match value {
                VmValue::Integer(value) => u64::try_from(value).map_err(|_| {
                    StepError::new(VmFaultCode::Bounds, "STRFORM variable index is negative")
                }),
                _ => Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "STRFORM variable index is not an integer",
                )),
            })
            .collect()
    }

    fn take_values(&mut self, count: usize) -> Result<Vec<VmValue>, StepError> {
        let start = self.values.len().checked_sub(count).ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM value stack underflow",
            )
        })?;
        Ok(self.values.drain(start..).collect())
    }

    fn pop_value(&mut self, message: &str) -> Result<VmValue, StepError> {
        self.values
            .pop()
            .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, message))
    }

    fn pop_integer(&mut self, message: &str) -> Result<i64, StepError> {
        let VmValue::Integer(value) = self.pop_value(message)? else {
            return Err(StepError::new(VmFaultCode::TypeMismatch, message));
        };
        Ok(value)
    }

    fn output_mut(&mut self) -> Result<&mut String, StepError> {
        self.outputs.last_mut().ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM output stack is empty",
            )
        })
    }

    fn append_output(&mut self, value: &str) -> Result<(), StepError> {
        let retained = self.retained_string_bytes()?;
        if retained
            .checked_add(value.len())
            .is_none_or(|bytes| bytes > MAX_RUNTIME_FORM_BYTES)
        {
            return Err(resource_limit("STRFORM output exceeds the runtime limit"));
        }
        self.output_mut()?.push_str(value);
        Ok(())
    }

    fn retained_string_bytes(&self) -> Result<usize, StepError> {
        self.outputs
            .iter()
            .map(String::len)
            .chain(self.values.iter().filter_map(|value| match value {
                VmValue::String(value) => Some(value.len()),
                _ => None,
            }))
            .try_fold(0usize, usize::checked_add)
            .ok_or_else(|| resource_limit("STRFORM retained string size overflowed"))
    }

    fn check_resources(&self, vm: &Vm) -> Result<(), StepError> {
        if self.work.len() > vm.config.maximum_operand_stack
            || self.values.len() > vm.config.maximum_operand_stack
            || self.outputs.len() > MAX_RUNTIME_FORM_NESTING
        {
            return Err(resource_limit("STRFORM continuation exceeds VM limits"));
        }
        if self.retained_string_bytes()? > MAX_RUNTIME_FORM_BYTES {
            return Err(resource_limit(
                "STRFORM retained strings exceed the runtime limit",
            ));
        }
        Ok(())
    }
}

fn parse_runtime_form(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    source: &str,
    node_limit: usize,
) -> Result<(FormattedString, usize), StepError> {
    if source.len() > MAX_RUNTIME_FORM_BYTES {
        return Err(resource_limit(
            "STRFORM source exceeds the runtime parser limit",
        ));
    }
    preflight_nesting(source)?;
    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
    })?;
    let compatibility = program.artifact.call_compatibility;
    let mut context = DefaultParserContext::default();
    context.set_lexer_compatibility(
        compatibility.allow_full_width_space,
        compatibility.debug_semicolon,
        compatibility.ignore_triple_symbols,
    );
    let parsed = parse_formatted_at(source, 0, &context);
    if parsed.has_errors() {
        let message = parsed
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(StepError::new(
            VmFaultCode::Native,
            format!("STRFORM expansion failed: {message}"),
        ));
    }
    let mut formatted = parsed.value.ok_or_else(|| {
        StepError::new(
            VmFaultCode::Native,
            "STRFORM expansion produced no formatted string",
        )
    })?;
    resolve_named_indices(program, function, &mut formatted, 0)?;
    let nodes = validate_form(vm, natives, generation, function, &formatted, node_limit)?;
    Ok((formatted, nodes))
}

fn resolve_named_indices(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    formatted: &mut FormattedString,
    depth: usize,
) -> Result<(), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit(
            "STRFORM named-index resolution exceeds the nesting limit",
        ));
    }
    for part in &mut formatted.parts {
        match part {
            FormPart::Text(_) | FormPart::Triple { .. } => {}
            FormPart::StringInterpolation {
                expression, width, ..
            }
            | FormPart::IntegerInterpolation {
                expression, width, ..
            } => {
                resolve_expression_named_indices(program, function, expression, depth + 1)?;
                if let Some(width) = width {
                    resolve_expression_named_indices(program, function, width, depth + 1)?;
                }
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                resolve_expression_named_indices(program, function, condition, depth + 1)?;
                resolve_named_indices(program, function, then_value, depth + 1)?;
                if let Some(else_value) = else_value {
                    resolve_named_indices(program, function, else_value, depth + 1)?;
                }
            }
        }
    }
    Ok(())
}

fn resolve_expression_named_indices(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    expression: &mut Expr,
    depth: usize,
) -> Result<(), StepError> {
    if depth > MAX_RUNTIME_FORM_NESTING {
        return Err(resource_limit(
            "STRFORM named-index resolution exceeds the nesting limit",
        ));
    }
    match &mut expression.kind {
        ExprKind::Integer(_) | ExprKind::String(_) | ExprKind::Identifier(_) | ExprKind::Error => {}
        ExprKind::Variable { name, indices } => {
            for index in indices.iter_mut() {
                resolve_expression_named_indices(program, function, index, depth + 1)?;
            }
            let Some(definition) = program.scoped_variable(function, name) else {
                return Ok(());
            };
            let dimensions = definition.dimensions.len();
            let explicit_character =
                definition.storage == BytecodeStorage::Character && indices.len() > dimensions;
            for (position, index) in indices.iter_mut().enumerate() {
                if explicit_character && position == 0 {
                    continue;
                }
                let ExprKind::Identifier(candidate) = &index.kind else {
                    continue;
                };
                // Emuera gives an in-scope variable or zero-argument function precedence over
                // the symbolic CSV key used by an array subscript.
                if program.scoped_variable(function, candidate).is_some() {
                    continue;
                }
                if program.function_by_name(candidate).is_some() {
                    index.kind = ExprKind::Call {
                        name: candidate.clone(),
                        args: Vec::new(),
                    };
                    continue;
                }
                if let Some(value) =
                    super::native_ops::resolve_named_index_value(program, name, candidate)
                {
                    index.kind = ExprKind::Integer(value);
                }
            }
        }
        ExprKind::Call { args, .. } => {
            for argument in args.iter_mut().flatten() {
                resolve_expression_named_indices(program, function, argument, depth + 1)?;
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => {
            resolve_expression_named_indices(program, function, operand, depth + 1)?;
        }
        ExprKind::Binary { left, right, .. } => {
            resolve_expression_named_indices(program, function, left, depth + 1)?;
            resolve_expression_named_indices(program, function, right, depth + 1)?;
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            resolve_expression_named_indices(program, function, condition, depth + 1)?;
            resolve_expression_named_indices(program, function, then_expr, depth + 1)?;
            resolve_expression_named_indices(program, function, else_expr, depth + 1)?;
        }
        ExprKind::Formatted(formatted) => {
            resolve_named_indices(program, function, formatted, depth + 1)?;
        }
    }
    Ok(())
}

// Validation is an exhaustive, iterative walk over every supported FORM AST node.
#[allow(clippy::too_many_lines)]
fn validate_form(
    vm: &Vm,
    natives: &NativeServiceRegistry,
    generation: GenerationId,
    function: SymbolKey,
    formatted: &FormattedString,
    node_limit: usize,
) -> Result<usize, StepError> {
    enum Node<'a> {
        Form(&'a FormattedString),
        Expr(&'a Expr),
    }

    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
    })?;
    let mut pending = vec![Node::Form(formatted)];
    let mut nodes = 0usize;
    while let Some(node) = pending.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > node_limit {
            return Err(resource_limit("STRFORM AST exceeds the VM operand limit"));
        }
        match node {
            Node::Form(formatted) => {
                for part in &formatted.parts {
                    match part {
                        FormPart::Text(_) => {}
                        FormPart::Triple { symbol, .. } => {
                            let names = match symbol {
                                '*' => Some(("NAME", "TARGET")),
                                '+' => Some(("CALLNAME", "MASTER")),
                                '=' => Some(("CALLNAME", "PLAYER")),
                                '/' => Some(("NAME", "ASSI")),
                                '$' => Some(("CALLNAME", "TARGET")),
                                _ => None,
                            }
                            .ok_or_else(|| {
                                unsupported(format!(
                                    "STRFORM triple symbol {symbol:?} is unsupported"
                                ))
                            })?;
                            for name in [names.0, names.1] {
                                if program.scoped_variable(function, name).is_none() {
                                    return Err(StepError::new(
                                        VmFaultCode::MissingSymbol,
                                        format!("STRFORM variable {name} is missing"),
                                    ));
                                }
                            }
                        }
                        FormPart::StringInterpolation {
                            expression, width, ..
                        }
                        | FormPart::IntegerInterpolation {
                            expression, width, ..
                        } => {
                            pending.push(Node::Expr(expression));
                            if let Some(width) = width {
                                pending.push(Node::Expr(width));
                            }
                        }
                        FormPart::Conditional {
                            condition,
                            then_value,
                            else_value,
                            ..
                        } => {
                            pending.push(Node::Expr(condition));
                            pending.push(Node::Form(then_value));
                            if let Some(else_value) = else_value {
                                pending.push(Node::Form(else_value));
                            }
                        }
                    }
                }
            }
            Node::Expr(expression) => match &expression.kind {
                ExprKind::Integer(_) | ExprKind::String(_) => {}
                ExprKind::Identifier(name) => {
                    ensure_variable(program, function, name)?;
                }
                ExprKind::Variable { name, indices } => {
                    ensure_variable(program, function, name)?;
                    pending.extend(indices.iter().map(Node::Expr));
                }
                ExprKind::Call { name, args } => {
                    if let Some(target) = program.function_by_name(name) {
                        if target.kind != BytecodeFunctionKind::Method || target.result.is_none() {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!("STRFORM target {name} is not a value-returning function"),
                            ));
                        }
                        if target
                            .parameters
                            .iter()
                            .any(|parameter| parameter.by_reference)
                        {
                            return Err(unsupported(format!(
                                "STRFORM target {name} requires a reference argument"
                            )));
                        }
                        if args.len() > target.parameters.len()
                            || target
                                .parameters
                                .iter()
                                .enumerate()
                                .any(|(index, parameter)| {
                                    args.get(index).is_none_or(Option::is_none)
                                        && parameter.default.is_none()
                                        && !program
                                            .artifact
                                            .call_compatibility
                                            .allow_omitted_arguments
                                })
                        {
                            return Err(StepError::new(
                                VmFaultCode::TypeMismatch,
                                format!("STRFORM target {name} has incompatible arguments"),
                            ));
                        }
                    } else {
                        let supported_native =
                            program.artifact.native_imports.iter().any(|native| {
                                native.import.name.eq_ignore_ascii_case(name)
                                    && native.import.result.is_some()
                                    && !matches!(
                                        native.import.result,
                                        Some(
                                            BytecodeType::IntegerPlace | BytecodeType::StringPlace
                                        )
                                    )
                                    && native.import.parameters.len() == args.len()
                                    && (name.eq_ignore_ascii_case("STRFORM")
                                        || natives.contains(native.import.key))
                            });
                        if !supported_native {
                            let host_only = program
                                .artifact
                                .host_imports
                                .iter()
                                .any(|host| host.import.name.eq_ignore_ascii_case(name));
                            return Err(if host_only {
                                unsupported(format!(
                                    "STRFORM host callable {name} is unsupported in a template"
                                ))
                            } else {
                                StepError::new(
                                    VmFaultCode::MissingSymbol,
                                    format!("STRFORM callable {name} is missing"),
                                )
                            });
                        }
                    }
                    pending.extend(args.iter().flatten().map(Node::Expr));
                }
                ExprKind::Unary { op, operand } => {
                    if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
                        return Err(unsupported("STRFORM increment expressions are unsupported"));
                    }
                    pending.push(Node::Expr(operand));
                }
                ExprKind::Postfix { .. } => {
                    return Err(unsupported("STRFORM increment expressions are unsupported"));
                }
                ExprKind::Binary { left, right, .. } => {
                    pending.push(Node::Expr(left));
                    pending.push(Node::Expr(right));
                }
                ExprKind::Ternary {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    pending.push(Node::Expr(condition));
                    pending.push(Node::Expr(then_expr));
                    pending.push(Node::Expr(else_expr));
                }
                ExprKind::Formatted(formatted) => pending.push(Node::Form(formatted)),
                ExprKind::Group(inner) => pending.push(Node::Expr(inner)),
                ExprKind::Error => {
                    return Err(unsupported("STRFORM contains an invalid expression"));
                }
            },
        }
    }
    Ok(nodes)
}

fn ensure_variable(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    name: &str,
) -> Result<(), StepError> {
    if program.scoped_variable(function, name).is_none() {
        return Err(StepError::new(
            VmFaultCode::MissingSymbol,
            format!("STRFORM variable {name} is missing"),
        ));
    }
    Ok(())
}

fn preflight_nesting(source: &str) -> Result<(), StepError> {
    let mut braces = 0usize;
    let mut parentheses = 0usize;
    let mut percent_expression = false;
    let mut quote = None;
    let mut escaped = false;
    let mut unary_run = 0usize;
    let mut ternaries = 0usize;
    for character in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"') && (braces > 0 || percent_expression) {
            quote = Some(character);
            continue;
        }
        match character {
            '%' if braces == 0 => percent_expression = !percent_expression,
            '{' => braces = braces.saturating_add(1),
            '}' => braces = braces.saturating_sub(1),
            '(' => {
                parentheses = parentheses.saturating_add(1);
            }
            ')' => {
                parentheses = parentheses.saturating_sub(1);
            }
            _ => {}
        }
        if matches!(character, '!' | '~' | '+' | '-') {
            unary_run = unary_run.saturating_add(1);
        } else if !character.is_whitespace() {
            unary_run = 0;
        }
        if character == '?' {
            ternaries = ternaries.saturating_add(1);
        }
        if braces.saturating_add(parentheses) > MAX_RUNTIME_FORM_NESTING
            || unary_run > MAX_RUNTIME_FORM_NESTING
            || ternaries > MAX_RUNTIME_FORM_NESTING
        {
            return Err(resource_limit("STRFORM parser nesting limit exceeded"));
        }
    }
    Ok(())
}

fn owner_frame(fiber: &Fiber, frame: FrameId) -> Result<&crate::state::Frame, StepError> {
    fiber
        .frames
        .iter()
        .find(|candidate| candidate.id == frame)
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM owner frame is missing",
            )
        })
}

fn owner_frame_mut(
    fiber: &mut Fiber,
    frame: FrameId,
) -> Result<&mut crate::state::Frame, StepError> {
    fiber
        .frames
        .iter_mut()
        .find(|candidate| candidate.id == frame)
        .ok_or_else(|| {
            StepError::new(
                VmFaultCode::InvalidInstruction,
                "STRFORM owner frame is missing",
            )
        })
}

fn unsupported(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::Native, message)
}

fn resource_limit(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::ResourceLimit, message)
}

const fn unary_tag(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Plus => 0,
        UnaryOp::Minus => 1,
        UnaryOp::LogicalNot => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::PreIncrement => 4,
        UnaryOp::PreDecrement => 5,
    }
}

const fn binary_tag(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Multiply => 0,
        BinaryOp::Divide => 1,
        BinaryOp::Modulo => 2,
        BinaryOp::Add => 3,
        BinaryOp::Subtract => 4,
        BinaryOp::ShiftLeft => 5,
        BinaryOp::ShiftRight => 6,
        BinaryOp::Less => 7,
        BinaryOp::LessEqual => 8,
        BinaryOp::Greater => 9,
        BinaryOp::GreaterEqual => 10,
        BinaryOp::Equal => 11,
        BinaryOp::NotEqual => 12,
        BinaryOp::BitAnd => 13,
        BinaryOp::BitXor => 14,
        BinaryOp::BitOr => 15,
        BinaryOp::LogicalAnd => 16,
        BinaryOp::LogicalXor => 17,
        BinaryOp::LogicalOr => 18,
        BinaryOp::Nand => 19,
        BinaryOp::Nor => 20,
    }
}
