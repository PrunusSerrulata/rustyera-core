use erabasic_ast::{Alignment, BinaryOp, Expr, ExprKind, FormPart, FormattedString, UnaryOp};
use erabasic_bytecode::{BytecodeFunctionKind, BytecodeType, SymbolKey};
use erabasic_parser::{DefaultParserContext, parse_formatted_at};
use serde::{Deserialize, Serialize};

use super::{StepError, map_vm_error};
use crate::{
    Fiber, FrameId, GenerationId, HostReady, NativeServiceRegistry, Vm, VmFaultCode, VmValue,
};

mod bit_calls;
mod call_plan;
mod call_text;
mod checkpoints;
mod existvar;
mod frontend;
mod host_calls;
mod input_host;
mod map_calls;
mod matching;
mod methods;
mod mutations;
mod native_binding;
mod reference_arguments;
mod source_arguments;
mod staged_binding;
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
    next_map_call: u64,
    next_reference_scope: u64,
    next_bit_call: u64,
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
    GateInputHost {
        plan: u64,
        key: Expr,
        triggered: bool,
    },
    FinishInputHost {
        name: String,
        count: usize,
    },
    ReadInputHost {
        depth: usize,
        gate: Option<(Expr, bool)>,
    },
    BitCapture {
        spec: erabasic_bytecode::BitCallSpec,
        site: call_plan::RuntimeCallSite,
        source: Vec<Option<Expr>>,
    },
    BitFinish(bit_calls::FormBitCall),
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
    MapCapture {
        bound: erabasic_bytecode::BoundRuntimeNative,
        site: call_plan::RuntimeCallSite,
        arguments: Vec<Option<Expr>>,
    },
    MapValuesEnabled {
        call: map_calls::MapWorkCall,
        output: Option<Expr>,
    },
    MapFinish(map_calls::MapWorkCall),
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
    MatchBegin(matching::FormMatch),
    MatchEnd(matching::FormMatch),
    MatchNeedle(matching::FormMatch),
    MatchScan(matching::FormMatch),
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
        next_map_call: 1,
        next_reference_scope: 1,
        next_bit_call: 1,
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
        next_map_call: 1,
        next_reference_scope: 1,
        next_bit_call: 1,
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

mod continuation;
mod metadata;
