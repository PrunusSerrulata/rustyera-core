use erabasic_ast::{Alignment, BinaryOp, Expr, ExprKind, FormPart, FormattedString, UnaryOp};
use erabasic_bytecode::{BytecodeFunctionKind, BytecodeType, SymbolKey};
use erabasic_parser::{DefaultParserContext, parse_formatted_at};
use serde::{Deserialize, Serialize};

use super::{StepError, map_vm_error};
use crate::{
    Fiber, FrameId, GenerationId, HostReady, NativeServiceRegistry, Vm, VmFaultCode, VmValue,
};

mod call_plan;
mod call_text;
mod checkpoints;
mod existvar;
mod frontend;
mod host_calls;
mod methods;
mod mutations;
mod native_binding;
mod reference_arguments;
mod source_arguments;
mod support;
mod typing;

use checkpoints::FormatCheckpoint;
pub(crate) use checkpoints::{
    RuntimeFormCatchTarget, finish_runtime_form_catch, select_runtime_form_catch,
};
use frontend::parse_runtime_form;
pub(super) use frontend::probe_runtime_expression;
use support::{binary_tag, owner_frame, owner_frame_mut, resource_limit, unary_tag, unsupported};
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
    awaiting_user_call: Option<methods::RuntimeUserWait>,
    checkpoints: Vec<FormatCheckpoint>,
    next_checkpoint: u64,
    host_calls: Vec<host_calls::RuntimeHostCall>,
    next_host_scope: u64,
    completion: RuntimeFormRoot,
    reference_arguments: Option<reference_arguments::PendingReferenceArguments>,
    reference_bindings: bool,
    call_plans: Vec<call_plan::RuntimeCallPlan>,
    current_call_plan: Option<u64>,
    next_call_plan: u64,
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
    BeginCheckedForm(String),
    FinishCheck(u64),
    FinishExpressionProbe(u64),
    FinishCallTextArgumentCatch(u64),
    ExistVarFirst {
        plan: u64,
        source: Expr,
        mode: Option<Expr>,
    },
    ExistVarMode {
        plan: u64,
        source: Expr,
    },
    ParseCallText {
        source: String,
        spec: erabasic_bytecode::CallTextSpec,
    },
    ReferenceArgumentsPump,
    RestoreReferenceBindings(bool),
    RestoreCallPlan(Option<u64>),
    ReleaseReferenceArguments,
    FinishCallTextArguments {
        target: SymbolKey,
        spec: erabasic_bytecode::CallTextSpec,
    },
    CaptureReferencePlace {
        key: SymbolKey,
        indices: usize,
    },
    Evaluate(Expr),
    ReadVariable {
        name: String,
        indices: usize,
    },
    ApplyUnary(UnaryOp),
    MutateVariable {
        variable: SymbolKey,
        indices: usize,
        mode: u8,
    },
    EvaluateBinaryRight {
        op: BinaryOp,
        right: Expr,
    },
    ApplyBinary(BinaryOp),
    ChooseTernary {
        then_expr: Expr,
        else_expr: Expr,
    },
    FinishNative {
        site: call_plan::RuntimeCallSite,
        bound: erabasic_bytecode::BoundRuntimeNative,
        source: Vec<Option<Expr>>,
    },
    HostAdvance(u64),
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
    ResolveMethod {
        plan: u64,
        result: erabasic_bytecode::MethodResult,
        fallback: Option<Expr>,
        arguments: Vec<Option<Expr>>,
    },
    MethodArgument(methods::RuntimeUserCall),
    CaptureMethodArgument(methods::RuntimeUserCall),
    ExistsMethod,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum RuntimeFormRoot {
    Value(BytecodeType),
    Call {
        spec: erabasic_bytecode::CallTextSpec,
        failed: bool,
    },
}

pub(super) enum RuntimeFormStep {
    Pending,
    Blocked,
    Complete(VmValue),
    CompleteCall,
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
    let (formatted, mut plan) =
        parse_runtime_form(vm, natives, generation, function, source, node_limit)?;
    let nodes = plan.nodes;
    plan.id = 1;
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
        awaiting_user_call: None,
        checkpoints: Vec::new(),
        next_checkpoint: 1,
        host_calls: Vec::new(),
        next_host_scope: 1,
        completion: RuntimeFormRoot::Value(BytecodeType::String),
        reference_arguments: None,
        reference_bindings: false,
        call_plans: vec![plan],
        current_call_plan: Some(1),
        next_call_plan: 2,
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

/// The caller evaluates and type-checks the outer String before entering this API.
pub(super) fn begin_runtime_form_check(
    vm: &Vm,
    fiber: &mut Fiber,
    generation: GenerationId,
    function: SymbolKey,
    instruction: usize,
    source: String,
) -> Result<(), StepError> {
    begin_work(
        vm,
        fiber,
        generation,
        function,
        instruction,
        RuntimeFormRoot::Value(BytecodeType::Integer),
        RuntimeFormTask::BeginCheckedForm(source),
    )
}

pub(super) fn begin_runtime_call_text(
    vm: &Vm,
    fiber: &mut Fiber,
    generation: GenerationId,
    function: SymbolKey,
    instruction: usize,
    source: String,
    spec: erabasic_bytecode::CallTextSpec,
) -> Result<(), StepError> {
    let program = vm.generations.get(&generation).ok_or_else(|| {
        StepError::new(VmFaultCode::MissingSymbol, "CALLSTR generation is missing")
    })?;
    if !program.artifact.manifest.compatibility.supports_call_text() {
        return Err(support::permission_denied(
            "CALLSTR is unavailable in this compatibility identity",
        ));
    }
    begin_work(
        vm,
        fiber,
        generation,
        function,
        instruction,
        RuntimeFormRoot::Call {
            spec,
            failed: false,
        },
        RuntimeFormTask::ParseCallText { source, spec },
    )
}

fn begin_work(
    vm: &Vm,
    fiber: &mut Fiber,
    generation: GenerationId,
    function: SymbolKey,
    instruction: usize,
    completion: RuntimeFormRoot,
    task: RuntimeFormTask,
) -> Result<(), StepError> {
    let owner = fiber.frames.last_mut().ok_or_else(|| {
        StepError::new(
            VmFaultCode::InvalidInstruction,
            "runtime-form caller frame is missing",
        )
    })?;
    if owner.runtime_form.is_some() || owner.generation != generation || owner.function != function
    {
        return Err(StepError::new(
            VmFaultCode::InvalidInstruction,
            "runtime-form caller identity or continuation differs",
        ));
    }
    owner.runtime_form = Some(RuntimeFormContinuation {
        generation,
        function,
        frame: owner.id,
        instruction,
        work: vec![RuntimeFormTask::CompleteRoot, task],
        values: Vec::new(),
        outputs: Vec::new(),
        awaiting_user_call: None,
        checkpoints: Vec::new(),
        next_checkpoint: 1,
        host_calls: Vec::new(),
        next_host_scope: 1,
        completion,
        reference_arguments: None,
        reference_bindings: false,
        call_plans: Vec::new(),
        current_call_plan: None,
        next_call_plan: 1,
        remaining_nodes: vm.config.maximum_operand_stack.max(1),
        remaining_source_bytes: MAX_RUNTIME_FORM_BYTES,
    });
    Ok(())
}

pub(super) fn resume_runtime_form(
    vm: &mut Vm,
    fiber: &mut Fiber,
    natives: &mut NativeServiceRegistry,
    position: &super::InstructionPosition<'_>,
    host: &mut impl crate::VmHost,
    host_count: &mut u32,
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

    let result = continuation.step(vm, fiber, natives, position, host, host_count);
    match result {
        Ok(RuntimeFormStep::Complete(value)) => Ok(RuntimeFormStep::Complete(value)),
        Ok(RuntimeFormStep::CompleteCall) => Ok(RuntimeFormStep::CompleteCall),
        Ok(step @ (RuntimeFormStep::Pending | RuntimeFormStep::Blocked)) => {
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
            Ok(step)
        }
        Err(error) => {
            let frame = fiber
                .frames
                .iter_mut()
                .find(|frame| frame.id == continuation.frame)
                .ok_or_else(|| {
                    StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "STRFORM error owner disappeared",
                    )
                })?;
            if frame.runtime_form.replace(continuation).is_some() {
                return Err(StepError::new(
                    VmFaultCode::InvalidInstruction,
                    "STRFORM error owner already has a continuation",
                ));
            }
            Err(error)
        }
    }
}

impl RuntimeFormContinuation {
    pub(crate) const fn origin(&self) -> (GenerationId, SymbolKey, usize) {
        (self.generation, self.function, self.instruction)
    }

    pub(crate) fn call_text_spec(&self) -> Option<erabasic_bytecode::CallTextSpec> {
        match self.completion {
            RuntimeFormRoot::Call { spec, .. } => Some(spec),
            RuntimeFormRoot::Value(_) => None,
        }
    }

    pub(crate) fn root_result_type(&self) -> Option<BytecodeType> {
        match self.completion {
            RuntimeFormRoot::Value(value_type) => Some(value_type),
            RuntimeFormRoot::Call { .. } => None,
        }
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
            && self.host_scopes_valid()
            && self.checkpoints_valid()
            && self.host_resources().is_some_and(|(slots, bytes)| {
                slots <= maximum_stack && bytes <= MAX_RUNTIME_FORM_BYTES
            })
            && self.reference_arguments_valid()
            && self
                .reference_argument_resources()
                .is_some_and(|(slots, bytes)| {
                    slots <= maximum_stack && bytes <= MAX_RUNTIME_FORM_BYTES
                })
    }

    // Keeping the continuation transition table together makes every resumable state auditable.
    #[allow(clippy::too_many_lines)]
    fn step(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
        position: &super::InstructionPosition<'_>,
        host: &mut impl crate::VmHost,
        host_count: &mut u32,
    ) -> Result<RuntimeFormStep, StepError> {
        if self.awaiting_user_call.is_some() {
            self.finish_user_wait(vm, fiber)?;
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
            RuntimeFormTask::HostAdvance(id) => {
                return self.advance_host_call(vm, fiber, id, position, host, host_count);
            }
            RuntimeFormTask::ReferenceArgumentsPump => self.advance_reference_arguments(vm)?,
            RuntimeFormTask::RestoreCallPlan(previous) => {
                self.restore_call_plan(previous)?;
            }
            RuntimeFormTask::RestoreReferenceBindings(previous) => {
                self.reference_bindings = previous;
            }
            RuntimeFormTask::ReleaseReferenceArguments => {
                self.reference_bindings = false;
                self.reference_arguments = None;
            }
            RuntimeFormTask::FinishCallTextArguments { target, spec } => {
                self.finish_call_text_arguments(vm, fiber, target, spec)?;
            }
            RuntimeFormTask::CaptureReferencePlace { key, indices } => {
                self.capture_reference_place(vm, fiber, key, indices)?;
            }
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
                if !self.host_calls.is_empty()
                    || !self.outputs.is_empty()
                    || !self.checkpoints.is_empty()
                    || !self.work.is_empty()
                {
                    return Err(StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "runtime form root has unfinished temporary state",
                    ));
                }
                match self.completion {
                    RuntimeFormRoot::Value(expected) => {
                        if self.values.len() != 1 || self.values[0].value_type() != expected {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "runtime form root result type or count differs",
                            ));
                        }
                        return Ok(RuntimeFormStep::Complete(
                            self.values.pop().expect("one value checked"),
                        ));
                    }
                    RuntimeFormRoot::Call { spec, failed } => {
                        if !self.values.is_empty() {
                            return Err(StepError::new(
                                VmFaultCode::InvalidInstruction,
                                "call text root has an unexpected value",
                            ));
                        }
                        if failed && spec.mode.has_catch() {
                            owner_frame_mut(fiber, self.frame)?.instruction =
                                spec.catch_target as usize;
                        }
                        return Ok(RuntimeFormStep::CompleteCall);
                    }
                }
            }
            RuntimeFormTask::BeginCheckedForm(source) => {
                self.begin_checked_form(vm, fiber, natives, &source)?;
            }
            RuntimeFormTask::FinishCheck(id) => self.finish_checked_form(id)?,
            RuntimeFormTask::ParseCallText { source, spec } => {
                self.parse_call_text(vm, fiber, natives, &source, spec)?;
            }
            RuntimeFormTask::ExistVarFirst { plan, source, mode } => {
                self.existvar_first(vm, plan, source, mode)?;
            }
            RuntimeFormTask::ExistVarMode { source, .. } => {
                self.existvar_mode(vm, fiber, source)?;
            }
            RuntimeFormTask::FinishExpressionProbe(id) => self.finish_expression_probe(vm, id)?,
            RuntimeFormTask::FinishCallTextArgumentCatch(_) => {
                return Err(StepError::new(
                    VmFaultCode::InvalidInstruction,
                    "CALLSTR argument checkpoint escaped its user-call state",
                ));
            }
            RuntimeFormTask::Evaluate(expression) => {
                self.evaluate_expression(vm, expression)?;
            }
            RuntimeFormTask::ReadVariable { name, indices } => {
                let indices = self.take_indices(indices)?;
                self.values
                    .push(self.read_variable(vm, fiber, &name, &indices)?);
            }
            RuntimeFormTask::MutateVariable {
                variable,
                indices,
                mode,
            } => self.mutate_variable(vm, fiber, variable, indices, mode)?,
            RuntimeFormTask::ApplyUnary(op) => {
                let value = self.pop_value("STRFORM unary operand is missing")?;
                self.values
                    .push(vm.unary_value(self.generation, unary_tag(op), value)?);
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
                self.values
                    .push(vm.binary_value(self.generation, binary_tag(op), left, right)?);
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
            RuntimeFormTask::FinishNative { bound, source, .. } => {
                let arguments = self.take_values(source.len())?;
                self.finish_native(vm, fiber, natives, bound, arguments)?;
            }
            RuntimeFormTask::FinishCall { name, arguments } => {
                let arguments = self.take_values(arguments)?;
                self.finish_call(vm, fiber, natives, &name, &arguments)?;
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
                        return Err(StepError::script(
                            crate::ScriptFaultKind::Argument,
                            VmFaultCode::TypeMismatch,
                            "STRFORM string interpolation expects a string",
                        ));
                    }
                    (false, _) => {
                        return Err(StepError::script(
                            crate::ScriptFaultKind::Argument,
                            VmFaultCode::TypeMismatch,
                            "STRFORM integer interpolation expects an integer",
                        ));
                    }
                };
                let width_value = width.map(VmValue::Integer);
                let alignment_value = width_value
                    .as_ref()
                    .map(|_| VmValue::Integer(i64::from(alignment == Some(Alignment::Left))));
                let value = crate::host::apply_owned_width_with_mode(
                    value,
                    width_value.as_ref(),
                    alignment_value.as_ref(),
                    natives.character_width_mode(),
                )?;
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
            RuntimeFormTask::ResolveMethod {
                plan,
                result,
                fallback,
                arguments,
            } => {
                self.resolve_method(vm, plan, result, fallback, arguments)?;
            }
            RuntimeFormTask::MethodArgument(call) => {
                self.advance_method_arguments(vm, fiber, call)?;
            }
            RuntimeFormTask::CaptureMethodArgument(mut call) => {
                self.validate_method_call(vm, fiber, &call, true)?;
                let actual = self.pop_value("STRFORM method argument is missing")?;
                let slot = call.next_slot;
                call.captured[slot] = Some(
                    vm.capture_user_argument(
                        fiber,
                        self.frame,
                        &call.call,
                        &call.specs,
                        slot,
                        actual,
                    )
                    .map_err(map_vm_error)?,
                );
                call.next_slot += 1;
                self.work.push(RuntimeFormTask::MethodArgument(call));
            }
            RuntimeFormTask::ExistsMethod => {
                let VmValue::String(name) = self.pop_value("EXISTMETH name is missing")? else {
                    return Err(StepError::new(
                        VmFaultCode::TypeMismatch,
                        "EXISTMETH expects a string",
                    ));
                };
                let program = vm.generations.get(&self.generation).ok_or_else(|| {
                    StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
                })?;
                self.values
                    .push(VmValue::Integer(crate::state::user_calls::exists_method(
                        program,
                        self.generation,
                        &name,
                    )));
                vm.invalidate_path_memo(fiber.id);
            }
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
                    StepError::script(
                        crate::ScriptFaultKind::Bounds,
                        VmFaultCode::Bounds,
                        "STRFORM triple index is negative",
                    )
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

    fn evaluate_expression(&mut self, vm: &Vm, expression: Expr) -> Result<(), StepError> {
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
                    self.schedule_integer_mutation(
                        vm,
                        &operand,
                        u8::from(op == UnaryOp::PreDecrement),
                    )?;
                    return Ok(());
                }
                self.work.push(RuntimeFormTask::ApplyUnary(op));
                self.work.push(RuntimeFormTask::Evaluate(*operand));
            }
            ExprKind::Postfix { op, operand } => {
                let mode = match op {
                    erabasic_ast::PostfixOp::Increment => 2,
                    erabasic_ast::PostfixOp::Decrement => 3,
                };
                self.schedule_integer_mutation(vm, &operand, mode)?;
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
                let user_defined = vm
                    .generations
                    .get(&self.generation)
                    .is_some_and(|program| program.function_by_name(&name).is_some());
                if user_defined {
                    self.schedule_direct_user_call(vm, &name, args)?;
                    return Ok(());
                }
                if name.eq_ignore_ascii_case("EXISTVAR") {
                    self.schedule_existvar(&args)?;
                    return Ok(());
                }
                if name.eq_ignore_ascii_case("EXISTMETH") {
                    let program = vm.generations.get(&self.generation).ok_or_else(|| {
                        StepError::new(VmFaultCode::MissingSymbol, "EXISTMETH generation missing")
                    })?;
                    native_binding::authorization(program, &name)?;
                }
                if self.schedule_method(&name, &args)? {
                    return Ok(());
                }
                if !name.eq_ignore_ascii_case("STRFORM")
                    && !name.eq_ignore_ascii_case("STRFORMCHECK")
                {
                    self.schedule_planned_call(vm, expression.span, &args)?;
                    return Ok(());
                }
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

    fn schedule_form_source(
        &mut self,
        vm: &Vm,
        natives: &NativeServiceRegistry,
        source: &str,
    ) -> Result<(), StepError> {
        if source.len() > self.remaining_source_bytes {
            return Err(resource_limit(
                "nested form sources exceed the parser limit",
            ));
        }
        self.remaining_source_bytes -= source.len();
        let (formatted, plan) = parse_runtime_form(
            vm,
            natives,
            self.generation,
            self.function,
            source,
            self.remaining_nodes,
        )?;
        self.remaining_nodes = self.remaining_nodes.saturating_sub(plan.nodes);
        let previous = self.current_call_plan;
        self.install_call_plan(plan)?;
        self.work.push(RuntimeFormTask::RestoreCallPlan(previous));
        self.work.push(RuntimeFormTask::RestoreReferenceBindings(
            self.reference_bindings,
        ));
        self.reference_bindings = false;
        self.work.push(RuntimeFormTask::StartForm(formatted));
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
        arguments: &[VmValue],
    ) -> Result<(), StepError> {
        let generation =
            std::sync::Arc::clone(vm.generations.get(&self.generation).ok_or_else(|| {
                StepError::new(VmFaultCode::MissingSymbol, "STRFORM generation is missing")
            })?);
        if generation.function_by_name(name).is_some() {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "user call bypassed lazy runtime-form resolver",
            ));
        }
        if name.eq_ignore_ascii_case("STRFORM") || name.eq_ignore_ascii_case("STRFORMCHECK") {
            let family = native_binding::authorization(&generation, name)?;
            native_binding::require_provider(natives, family)?;
            let [VmValue::String(source)] = arguments else {
                return Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "formatted-string function expects one string argument",
                ));
            };
            if name.eq_ignore_ascii_case("STRFORMCHECK") {
                self.begin_checked_form(vm, fiber, natives, source)?;
            } else {
                self.schedule_form_source(vm, natives, source)?;
            }
            return Ok(());
        }

        Err(StepError::new(
            VmFaultCode::InvalidInstruction,
            "Native call bypassed source authorization binding",
        ))
    }

    fn finish_native(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        natives: &mut NativeServiceRegistry,
        bound: erabasic_bytecode::BoundRuntimeNative,
        arguments: Vec<VmValue>,
    ) -> Result<(), StepError> {
        let generation =
            std::sync::Arc::clone(vm.generations.get(&self.generation).ok_or_else(|| {
                StepError::new(VmFaultCode::MissingSymbol, "Native generation missing")
            })?);
        let family = native_binding::authorization(&generation, &bound.import.name)?;
        if family.bind_physical(
            bound.import.parameters.clone(),
            bound.omitted_arguments.clone(),
        ) != bound
            || bound.import.parameters.len() != arguments.len()
            || bound
                .import
                .parameters
                .iter()
                .zip(&arguments)
                .any(|(expected, value)| *expected != value.value_type())
        {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "Native operands differ from bound source signature",
            ));
        }
        native_binding::require_provider(natives, family)?;
        let service_key = bound.service_key;
        if bound.import.name == "replace" && bound.omitted_arguments.contains(&3) {
            return Err(StepError::script(
                crate::ScriptFaultKind::Operation,
                VmFaultCode::Native,
                "REPLACE omitted mode is read by the reference method",
            ));
        }
        let omitted_arguments = bound.omitted_arguments;
        let native = bound.import;
        let expected = native.result;
        let owner_stack = owner_frame(fiber, self.frame)?.stack.len();
        let (ready, rollback) = if let Some(ready) = vm
            .execute_special_native(
                fiber,
                &native.name.to_ascii_lowercase(),
                &arguments,
                &omitted_arguments,
            )
            .map_err(map_vm_error)?
        {
            (ready, None)
        } else {
            vm.call_registered_native_with_omissions(
                fiber,
                service_key,
                native.clone(),
                arguments,
                omitted_arguments,
                natives,
            )?
        };
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
            if let Some(checkpoint) = rollback
                && let Err(rollback) = natives.rollback(service_key, &checkpoint)
            {
                return Err(StepError::classified(
                    crate::FaultCategory::HostContract,
                    VmFaultCode::Native,
                    format!("runtime-form Native rollback failed: {rollback}"),
                ));
            }
            // Ready values/writes belong to the service contract. A bad returned
            // place is not a script bounds error, even when the storage validator
            // uses that category for a script-originated read of the same place.
            return Err(StepError::classified(
                crate::FaultCategory::HostContract,
                VmFaultCode::Native,
                error.to_string(),
            ));
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
}
