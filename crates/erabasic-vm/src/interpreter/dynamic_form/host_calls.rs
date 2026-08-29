//! Direct Host expressions use the ordinary caller-pumped Host boundary.
use super::super::{InstructionPosition, StepOutcome};
use super::call_plan::{RuntimeBoundCall, RuntimeCallSite};
use super::support::{owner_frame, owner_frame_mut};
use super::{
    RuntimeFormContinuation, RuntimeFormStep, RuntimeFormTask, methods, resource_limit, support,
};
use crate::interpreter::StepError;
use crate::{Fiber, Vm, VmFaultCode, VmValue};
use crate::{FiberState, RuntimeHostScope, VmExecutionOrigin, VmHost};
use erabasic_ast::{Expr, ExprKind};
use erabasic_bytecode::SymbolKey;
use erabasic_bytecode::{
    BoundRuntimeHost, HostImport, RuntimeExpressionShape, RuntimeHostAuthorization,
    RuntimeHostLowering, RuntimeHostStage,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeHostCall {
    id: u64,
    site: RuntimeCallSite,
    bound: BoundRuntimeHost,
    source: Vec<Option<Expr>>,
    phase: HostPhase,
    /// A measurement or LINES ticket consumed before the later source expression.
    prefix: Vec<VmValue>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum HostPhase {
    Collect {
        stage: RuntimeHostStage,
        count: usize,
    },
    Ready {
        stage: RuntimeHostStage,
        arguments: Vec<VmValue>,
    },
    Waiting {
        stage: RuntimeHostStage,
        stack_depth: usize,
    },
}

pub(super) fn bind(
    program: &crate::ProgramGeneration,
    name: &str,
    shapes: &[Option<RuntimeExpressionShape>],
) -> Result<BoundRuntimeHost, StepError> {
    let family = program
        .artifact
        .runtime_host_authorizations
        .iter()
        .find(|family| family.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            support::permission_denied(format!(
                "runtime callable {name} lacks a Host authorization"
            ))
        })?;
    family.bind(shapes).ok_or_else(|| {
        StepError::script(
            crate::ScriptFaultKind::Argument,
            VmFaultCode::TypeMismatch,
            format!("Host callable {name} has incompatible source arguments"),
        )
    })
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_host_arguments(
        &mut self,
        vm: &Vm,
        bound: &BoundRuntimeHost,
        source: &[Option<Expr>],
        site: RuntimeCallSite,
    ) -> Result<(), StepError> {
        let program = vm
            .generations
            .get(&self.generation)
            .ok_or_else(|| invalid("Host generation missing"))?;
        if self.lookup_bound_call(site) != Some(&RuntimeBoundCall::Host(bound.clone()))
            || !self.validate_call_arguments(program, site, source)
        {
            return Err(invalid("Host call differs from its immutable source plan"));
        }
        let family = family(program, bound.family_key)?;
        let id = self.next_host_scope;
        self.next_host_scope = id
            .checked_add(1)
            .ok_or_else(|| resource_limit("Host scope identity exhausted"))?;
        let (stage, count) = match family.lowering {
            RuntimeHostLowering::Eager => (RuntimeHostStage::Call, source.len()),
            RuntimeHostLowering::HtmlLength => (RuntimeHostStage::MeasureLength, 1),
            RuntimeHostLowering::HtmlLines => (RuntimeHostStage::LinesBegin, 1),
        };
        self.host_calls.push(RuntimeHostCall {
            id,
            site,
            bound: bound.clone(),
            source: source.to_vec(),
            phase: HostPhase::Collect { stage, count },
            prefix: Vec::new(),
        });
        self.work.push(RuntimeFormTask::HostAdvance(id));
        self.schedule_bound_source_arguments(
            program,
            &source[..count],
            &bound.import.parameters[..count],
        )?;
        Ok(())
    }

    pub(crate) fn next_is_host_call(&self) -> bool {
        matches!(self.work.last(), Some(RuntimeFormTask::HostAdvance(id))
            if self.host_calls.last().is_some_and(|call| call.id == *id && matches!(call.phase, HostPhase::Ready { .. })))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn advance_host_call(
        &mut self,
        vm: &mut Vm,
        fiber: &mut Fiber,
        id: u64,
        position: &InstructionPosition<'_>,
        host: &mut impl VmHost,
        host_count: &mut u32,
    ) -> Result<RuntimeFormStep, StepError> {
        let index = self
            .host_calls
            .len()
            .checked_sub(1)
            .ok_or_else(|| invalid("Host scope missing"))?;
        if self.host_calls[index].id != id {
            return Err(invalid("Host scope is not the active call"));
        }
        let phase = self.host_calls[index].phase.clone();
        match phase {
            HostPhase::Collect { stage, count } => {
                let values = self.take_values(count)?;
                if stage == RuntimeHostStage::LinesBegin
                    && matches!(values.as_slice(), [VmValue::String(source)] if source.is_empty())
                {
                    self.host_calls
                        .pop()
                        .ok_or_else(|| invalid("empty HTML line scope missing"))?;
                    self.values.push(VmValue::Integer(0));
                    self.check_resources(vm)?;
                    return Ok(RuntimeFormStep::Pending);
                }
                let call = &mut self.host_calls[index];
                let mut arguments = call.prefix.clone();
                arguments.extend(values);
                call.phase = HostPhase::Ready { stage, arguments };
                self.work.push(RuntimeFormTask::HostAdvance(id));
            }
            HostPhase::Ready { stage, arguments } => {
                let program = vm
                    .generations
                    .get(&self.generation)
                    .ok_or_else(|| invalid("Host generation missing"))?;
                let call = &self.host_calls[index];
                let target = family(program, call.bound.family_key)?
                    .stage_import(&call.bound, stage)
                    .ok_or_else(|| invalid("Host stage is not authorized"))?;
                if arguments.len() != target.import.parameters.len()
                    || arguments
                        .iter()
                        .zip(&target.import.parameters)
                        .any(|(value, kind)| value.value_type() != *kind)
                {
                    return Err(invalid("Host evaluated argument signature differs"));
                }
                let omitted = if stage == RuntimeHostStage::Call {
                    call.bound.omitted_arguments.clone()
                } else {
                    Vec::new()
                };
                let scope = self.host_scope(fiber.id, id);
                let stack_depth = owner_frame(fiber, self.frame)?.stack.len();
                self.host_calls[index].phase = HostPhase::Waiting { stage, stack_depth };
                self.work.push(RuntimeFormTask::HostAdvance(id));
                let origin = vm.execution_origin(position, &target.import.name);
                let outcome = vm.dispatch_host_call(
                    fiber,
                    target,
                    arguments,
                    omitted,
                    origin,
                    Some(scope),
                    host,
                    host_count,
                )?;
                if matches!(outcome, StepOutcome::Blocked) {
                    return Ok(RuntimeFormStep::Blocked);
                }
            }
            HostPhase::Waiting { stage, stack_depth } => {
                // Ready, including immediate Ready, is validated and committed exactly once
                // by the ordinary VM Host boundary before this continuation consumes it.
                if !matches!(fiber.state, FiberState::Runnable) {
                    return Err(invalid("Host result resumed while waiting"));
                }
                let owner = owner_frame_mut(fiber, self.frame)?;
                if owner.stack.len()
                    != stack_depth
                        .checked_add(1)
                        .ok_or_else(|| invalid("Host stack depth overflow"))?
                {
                    return Err(invalid("Host result stack depth differs"));
                }
                let value = owner
                    .stack
                    .pop()
                    .ok_or_else(|| invalid("Host result missing"))?;
                self.accept_host_result(id, stage, value)?;
            }
        }
        self.check_resources(vm)?;
        Ok(RuntimeFormStep::Pending)
    }

    fn accept_host_result(
        &mut self,
        id: u64,
        stage: RuntimeHostStage,
        value: VmValue,
    ) -> Result<(), StepError> {
        use RuntimeHostStage as S;
        match stage {
            S::Call | S::LengthUnit | S::LinesEnd => {
                self.host_calls
                    .pop()
                    .ok_or_else(|| invalid("Host result scope missing"))?;
                self.values.push(value);
            }
            S::MeasureLength => self.accept_html_length(id, value)?,
            S::LinesBegin => {
                if !matches!(value, VmValue::String(_)) {
                    return Err(invalid("HTML line ticket type differs"));
                }
                let call = self
                    .host_calls
                    .last_mut()
                    .ok_or_else(|| invalid("HTML line scope missing"))?;
                call.prefix = vec![value.clone()];
                call.phase = HostPhase::Ready {
                    stage: S::LinesMore,
                    arguments: vec![value],
                };
                self.work.push(RuntimeFormTask::HostAdvance(id));
            }
            S::LinesMore => self.accept_html_more(id, &value)?,
            S::LinesStep => {
                if !matches!(value, VmValue::Integer(_)) {
                    return Err(invalid("HTML line step result type differs"));
                }
                let call = self
                    .host_calls
                    .last_mut()
                    .ok_or_else(|| invalid("HTML line scope missing"))?;
                // The ticket remains owned across every width evaluation and step.
                let ticket = call
                    .ticket()
                    .ok_or_else(|| invalid("HTML line ticket missing"))?
                    .clone();
                call.prefix = vec![ticket.clone()];
                call.phase = HostPhase::Ready {
                    stage: S::LinesMore,
                    arguments: vec![ticket],
                };
                self.work.push(RuntimeFormTask::HostAdvance(id));
            }
        }
        Ok(())
    }

    fn accept_html_length(&mut self, id: u64, value: VmValue) -> Result<(), StepError> {
        if !matches!(value, VmValue::Integer(_)) {
            return Err(invalid("HTML length result type differs"));
        }
        let call = self
            .host_calls
            .last_mut()
            .ok_or_else(|| invalid("HTML length scope missing"))?;
        call.prefix = vec![value];
        call.phase = HostPhase::Collect {
            stage: RuntimeHostStage::LengthUnit,
            count: 1,
        };
        if call.source.len() > 1 && call.source[1].is_none() {
            return Err(StepError::script(
                crate::ScriptFaultKind::Operation,
                VmFaultCode::Host,
                "HTML_STRINGLEN dereferenced an omitted unit after measurement",
            ));
        }
        let unit = call.source.get(1).cloned().flatten();
        self.work.push(RuntimeFormTask::HostAdvance(id));
        self.work.push(unit.map_or_else(
            || {
                RuntimeFormTask::Evaluate(Expr {
                    kind: ExprKind::Integer(0),
                    span: erabasic_ast::Span::default(),
                })
            },
            RuntimeFormTask::Evaluate,
        ));
        Ok(())
    }

    fn accept_html_more(&mut self, id: u64, value: &VmValue) -> Result<(), StepError> {
        let VmValue::Integer(more) = value else {
            return Err(invalid("HTML line predicate type differs"));
        };
        let call = self
            .host_calls
            .last_mut()
            .ok_or_else(|| invalid("HTML line scope missing"))?;
        if *more == 0 {
            call.phase = HostPhase::Ready {
                stage: RuntimeHostStage::LinesEnd,
                arguments: call.prefix.clone(),
            };
            self.work.push(RuntimeFormTask::HostAdvance(id));
        } else {
            call.phase = HostPhase::Collect {
                stage: RuntimeHostStage::LinesStep,
                count: 1,
            };
            let width = call
                .source
                .get(1)
                .cloned()
                .flatten()
                .ok_or_else(|| invalid("HTML width source missing"))?;
            self.work.push(RuntimeFormTask::HostAdvance(id));
            self.work.push(RuntimeFormTask::Evaluate(width));
        }
        Ok(())
    }

    fn host_scope(&self, fiber: crate::FiberId, occurrence: u64) -> RuntimeHostScope {
        RuntimeHostScope {
            fiber,
            frame: self.frame,
            generation: self.generation,
            function: self.function,
            instruction: u32::try_from(self.instruction).expect("validated instruction fits u32"),
            occurrence,
        }
    }
    pub(crate) fn contains_host_scope(&self, scope: RuntimeHostScope) -> bool {
        scope.frame == self.frame
            && scope.generation == self.generation
            && scope.function == self.function
            && scope.instruction as usize == self.instruction
            && scope.occurrence != 0
            && self
                .host_calls
                .iter()
                .any(|call| call.id == scope.occurrence)
    }
    pub(crate) fn waiting_host_import(
        &self,
        scope: RuntimeHostScope,
        origin: &VmExecutionOrigin,
        program: &crate::ProgramGeneration,
    ) -> Option<HostImport> {
        if !self.contains_host_scope(scope)
            || origin.generation != self.generation
            || origin.function != self.function
            || origin.instruction as usize != self.instruction
        {
            return None;
        }
        let call = self
            .host_calls
            .last()
            .filter(|call| call.id == scope.occurrence)?;
        let HostPhase::Waiting { stage, .. } = call.phase else {
            return None;
        };
        let import = family(program, call.bound.family_key)
            .ok()?
            .stage_import(&call.bound, stage)?;
        (origin.command.eq_ignore_ascii_case(&import.import.name)).then_some(import)
    }
    pub(super) fn host_resources(&self) -> Option<(usize, usize)> {
        let expressions = self
            .host_calls
            .iter()
            .flat_map(|call| call.source.iter().flatten())
            .collect();
        let (nodes, mut bytes) = methods::retained_expression_resources(expressions)?;
        let mut slots = nodes.checked_add(self.host_calls.len())?;
        for call in &self.host_calls {
            slots = slots
                .checked_add(call.bound.import.parameters.len())?
                .checked_add(call.bound.omitted_arguments.len())?;
            bytes = bytes
                .checked_add(call.bound.import.name.len())?
                .checked_add(call.bound.import.namespace.len())?;
            let arguments = match &call.phase {
                HostPhase::Ready { arguments, .. } => arguments.as_slice(),
                _ => &[],
            };
            for value in arguments.iter().chain(&call.prefix) {
                slots = slots.checked_add(1)?;
                if let VmValue::String(value) = value {
                    bytes = bytes.checked_add(value.len())?;
                }
            }
        }
        Some((slots, bytes))
    }
}
impl RuntimeHostCall {
    fn ticket(&self) -> Option<&VmValue> {
        self.prefix.first()
    }
}
fn family(
    program: &crate::ProgramGeneration,
    key: SymbolKey,
) -> Result<&RuntimeHostAuthorization, StepError> {
    program
        .artifact
        .runtime_host_authorizations
        .iter()
        .find(|family| family.key == key)
        .ok_or_else(|| invalid("Host family is missing from its owning generation"))
}
fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}

impl RuntimeFormContinuation {
    pub(super) fn abandon_host_scopes(&mut self, frontier: u64) {
        self.host_calls.retain(|call| call.id < frontier);
    }
    pub(super) fn host_scopes_valid(&self) -> bool {
        if self.next_host_scope == 0 {
            return false;
        }
        let mut previous = 0;
        for call in &self.host_calls {
            if call.id <= previous
                || call.id >= self.next_host_scope
                || self
                    .work
                    .iter()
                    .filter(
                        |task| matches!(task, RuntimeFormTask::HostAdvance(id) if *id == call.id),
                    )
                    .count()
                    != 1
            {
                return false;
            }
            previous = call.id;
        }
        self.work
            .iter()
            .filter(|task| matches!(task, RuntimeFormTask::HostAdvance(_)))
            .count()
            == self.host_calls.len()
    }
    pub(crate) fn valid_host_symbols(
        &self,
        program: &crate::ProgramGeneration,
        limit: usize,
    ) -> bool {
        if !self.host_scopes_valid() {
            return false;
        }
        self.host_calls.iter().all(|call| {
            if self.lookup_bound_call(call.site)
                != Some(&RuntimeBoundCall::Host(call.bound.clone()))
                || !self.validate_call_arguments(program, call.site, &call.source)
            {
                return false;
            }
            let Ok(family) = family(program, call.bound.family_key) else {
                return false;
            };
            let (stage, values) = match &call.phase {
                HostPhase::Collect { stage, count } => {
                    let expected = match stage {
                        RuntimeHostStage::Call => call.source.len(),
                        RuntimeHostStage::MeasureLength
                        | RuntimeHostStage::LinesBegin
                        | RuntimeHostStage::LengthUnit
                        | RuntimeHostStage::LinesStep => 1,
                        _ => return false,
                    };
                    if *count != expected {
                        return false;
                    }
                    (*stage, None)
                }
                HostPhase::Ready { stage, arguments } => (*stage, Some(arguments)),
                HostPhase::Waiting { stage, stack_depth } => {
                    if *stack_depth > limit {
                        return false;
                    }
                    (*stage, None)
                }
            };
            let Some(target) = family.stage_import(&call.bound, stage) else {
                return false;
            };
            if values.is_some_and(|values| {
                values.len() != target.import.parameters.len()
                    || values
                        .iter()
                        .zip(&target.import.parameters)
                        .any(|(value, kind)| value.value_type() != *kind)
            }) {
                return false;
            }
            match stage {
                RuntimeHostStage::Call
                | RuntimeHostStage::MeasureLength
                | RuntimeHostStage::LinesBegin => call.prefix.is_empty(),
                RuntimeHostStage::LengthUnit => {
                    matches!(call.prefix.as_slice(), [VmValue::Integer(_)])
                }
                RuntimeHostStage::LinesMore
                | RuntimeHostStage::LinesStep
                | RuntimeHostStage::LinesEnd => {
                    matches!(call.prefix.as_slice(), [VmValue::String(_)])
                }
            }
        })
    }
    pub(crate) fn scope_has_html_ticket(&self, scope: RuntimeHostScope, ticket: &str) -> bool {
        self.contains_host_scope(scope)
            && self
                .host_calls
                .iter()
                .find(|call| call.id == scope.occurrence)
                .is_some_and(|call| {
                    matches!(call.prefix.as_slice(), [VmValue::String(actual)] if actual == ticket)
                        && matches!(
                            call.phase,
                            HostPhase::Collect {
                                stage: RuntimeHostStage::LinesStep,
                                ..
                            } | HostPhase::Ready {
                                stage: RuntimeHostStage::LinesMore
                                    | RuntimeHostStage::LinesStep
                                    | RuntimeHostStage::LinesEnd,
                                ..
                            } | HostPhase::Waiting {
                                stage: RuntimeHostStage::LinesMore
                                    | RuntimeHostStage::LinesStep
                                    | RuntimeHostStage::LinesEnd,
                                ..
                            }
                        )
                })
    }
}

/// Consumes definitions already selected by the one type visitor; it never reads
/// storage or recursively invokes type analysis for an argument subtree.
pub(super) fn validate_source_tokens(
    program: &crate::ProgramGeneration,
    function: SymbolKey,
    name: &str,
    source: &[Option<Expr>],
) -> Result<(), StepError> {
    if !program
        .artifact
        .runtime_host_authorizations
        .iter()
        .any(|family| family.name.eq_ignore_ascii_case(name))
    {
        return Ok(());
    }
    for (slot, expression) in source.iter().enumerate() {
        let Some((ranks, kind)) = erabasic_bytecode::host_source_place_ranks(name, slot) else {
            continue;
        };
        let Some(mut expression) = expression.as_ref() else {
            continue;
        };
        while let ExprKind::Group(inner) = &expression.kind {
            expression = inner;
        }
        let (ExprKind::Identifier(name) | ExprKind::Variable { name, .. }) = &expression.kind
        else {
            return Err(StepError::script(
                crate::ScriptFaultKind::Argument,
                VmFaultCode::TypeMismatch,
                "Host array argument requires a variable token",
            ));
        };
        let variable = program
            .scoped_variable(function, name)
            .ok_or_else(|| invalid("analyzed Host variable disappeared"))?;
        if variable.value_type != kind || !ranks.contains(&variable.dimensions.len()) {
            return Err(StepError::script(
                crate::ScriptFaultKind::Argument,
                VmFaultCode::TypeMismatch,
                "Host array argument rank or scalar type differs",
            ));
        }
    }
    Ok(())
}

impl RuntimeFormContinuation {
    /// Shared plan retirement must retain these roots while argument/user waits run.
    pub(super) fn host_call_sites(&self) -> impl Iterator<Item = RuntimeCallSite> + '_ {
        self.host_calls.iter().map(|call| call.site)
    }
    pub(crate) fn html_scope_tickets(
        &self,
        fiber: crate::FiberId,
    ) -> Vec<(RuntimeHostScope, String)> {
        self.host_calls
            .iter()
            .filter_map(|call| {
                let scope = self.host_scope(fiber, call.id);
                let VmValue::String(ticket) = call.prefix.first()? else {
                    return None;
                };
                self.scope_has_html_ticket(scope, ticket)
                    .then(|| (scope, ticket.clone()))
            })
            .collect()
    }
}

impl RuntimeFormContinuation {
    /// The source plan was independently rebuilt once by `valid_method_state`.
    /// A stable argument wait can only suspend collection, never an issued Host stage.
    pub(crate) fn valid_host_snapshot(&self, vm: &Vm, fiber: &Fiber) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        self.valid_host_symbols(program, vm.config.maximum_operand_stack)
            && (!matches!(fiber.state, FiberState::WaitingHost(_))
                || self
                    .host_calls
                    .iter()
                    .all(|call| matches!(call.phase, HostPhase::Collect { .. })))
    }
}
