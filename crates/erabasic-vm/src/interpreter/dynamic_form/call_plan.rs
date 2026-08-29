//! One immutable type/binding plan per parsed source. Execution never retypes a subtree.
use super::{
    BytecodeType, Expr, ExprKind, FormPart, FormattedString, RuntimeFormContinuation,
    RuntimeFormTask, StepError, Vm, VmFaultCode, resource_limit, typing,
};
use erabasic_ast::Span;
use erabasic_bytecode::{BoundRuntimeHost, BoundRuntimeNative, UserArgumentSpec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeCallSite {
    pub plan: u64,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) enum RuntimeBoundCall {
    Native(BoundRuntimeNative),
    Host(BoundRuntimeHost),
    Bit(erabasic_bytecode::BitCallSpec),
    Match(erabasic_bytecode::MatchCallSpec),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) enum RuntimePlanSource {
    Form(FormattedString),
    Arguments(Vec<Option<Expr>>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct RuntimeCallPlan {
    pub id: u64,
    pub source: RuntimePlanSource,
    pub types: Vec<(Span, BytecodeType)>,
    pub calls: Vec<(Span, RuntimeBoundCall)>,
    pub nodes: usize,
}

fn invalid(message: &str) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
fn span_key(span: Span) -> (usize, usize) {
    (span.start, span.end)
}

impl RuntimeCallPlan {
    pub(super) fn from_analysis(
        source: RuntimePlanSource,
        analysis: typing::TypeAnalysis<'_>,
    ) -> Result<Self, StepError> {
        let nodes = analysis.nodes();
        let mut types = analysis.source_types;
        types.sort_by_key(|(span, _)| span_key(*span));
        if types
            .windows(2)
            .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1)
        {
            return Err(invalid("runtime source span has conflicting types"));
        }
        types.dedup_by_key(|(span, _)| span_key(*span));
        let mut calls = analysis.bound_calls;
        calls.sort_by_key(|(span, _)| span_key(*span));
        if calls.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(invalid("runtime call sites have duplicate source spans"));
        }
        Ok(Self {
            id: 0,
            source,
            types,
            calls,
            nodes,
        })
    }
    fn bound(&self, span: Span) -> Option<&RuntimeBoundCall> {
        self.calls
            .binary_search_by_key(&span_key(span), |(span, _)| span_key(*span))
            .ok()
            .map(|index| &self.calls[index].1)
    }
    fn expression_type(&self, expression: &Expr) -> Option<BytecodeType> {
        self.types
            .binary_search_by_key(&span_key(expression.span), |(span, _)| span_key(*span))
            .ok()
            .map(|index| self.types[index].1)
    }
    fn expressions(&self) -> SourceExpressions<'_> {
        SourceExpressions::new(&self.source)
    }
    fn call_arguments(&self, span: Span) -> Option<&[Option<Expr>]> {
        self.expressions().find_map(|expression| {
            if expression.span == span
                && let ExprKind::Call { args, .. } = &expression.kind
            {
                Some(args.as_slice())
            } else {
                None
            }
        })
    }
    fn valid(
        &self,
        program: &crate::ProgramGeneration,
        function: super::SymbolKey,
        generation: super::GenerationId,
        limit: usize,
        graph: Option<&erabasic_bytecode::ReferenceTermGraph>,
    ) -> bool {
        let mut analysis =
            typing::TypeAnalysis::new(program, function, generation, false, limit, None);
        analysis.reference_terms = matches!(self.source, RuntimePlanSource::Arguments(_));
        let analyzed = match &self.source {
            RuntimePlanSource::Form(form) => analysis.form(form, 0),
            RuntimePlanSource::Arguments(arguments) => arguments
                .iter()
                .flatten()
                .try_for_each(|expression| analysis.expression(expression, 0).map(|_| ())),
        };
        if analyzed.is_err() {
            return false;
        }
        if let (RuntimePlanSource::Arguments(arguments), Some(graph)) = (&self.source, graph)
            && !super::reference_arguments::GraphBuilder::new(
                program,
                function,
                &analysis.expression_types,
            )
            .build(arguments)
            .is_ok_and(|actual| actual == *graph)
        {
            return false;
        }
        Self::from_analysis(self.source.clone(), analysis).is_ok_and(|expected| {
            self.nodes == expected.nodes
                && self.types == expected.types
                && self.calls == expected.calls
        })
    }
}

impl RuntimeFormContinuation {
    pub(super) fn install_call_plan(
        &mut self,
        mut plan: RuntimeCallPlan,
    ) -> Result<u64, StepError> {
        let id = self.next_call_plan;
        self.next_call_plan = id
            .checked_add(1)
            .ok_or_else(|| resource_limit("runtime call plan identity exhausted"))?;
        plan.id = id;
        self.call_plans.push(plan);
        self.current_call_plan = Some(id);
        Ok(id)
    }
    fn call_plan(&self, id: u64) -> Option<&RuntimeCallPlan> {
        self.call_plans
            .binary_search_by_key(&id, |plan| plan.id)
            .ok()
            .map(|index| &self.call_plans[index])
    }
    pub(super) fn current_call_site(&self, span: Span) -> Result<RuntimeCallSite, StepError> {
        let plan = self
            .current_call_plan
            .ok_or_else(|| invalid("runtime expression lacks its source plan"))?;
        Ok(RuntimeCallSite { plan, span })
    }
    pub(super) fn lookup_bound_call(&self, site: RuntimeCallSite) -> Option<&RuntimeBoundCall> {
        self.call_plan(site.plan)?.bound(site.span)
    }
    pub(super) fn planned_expression_type(
        &self,
        plan: u64,
        expression: &Expr,
    ) -> Result<BytecodeType, StepError> {
        // Restructure replacements are scalar literals and carry no new callable syntax.
        match &expression.kind {
            ExprKind::Integer(_) => Ok(BytecodeType::Integer),
            ExprKind::String(_) => Ok(BytecodeType::String),
            _ => self
                .call_plan(plan)
                .and_then(|plan| plan.expression_type(expression))
                .ok_or_else(|| invalid("runtime expression lost its analyzed type")),
        }
    }
    pub(super) fn planned_argument_spec(
        &self,
        program: &crate::ProgramGeneration,
        plan: u64,
        argument: Option<&Expr>,
    ) -> Result<UserArgumentSpec, StepError> {
        let Some(expression) = argument else {
            return Ok(UserArgumentSpec::Omitted);
        };
        Ok(typing::shape_spec(
            program,
            self.function,
            expression,
            self.planned_expression_type(plan, expression)?,
        ))
    }
    pub(super) fn validate_call_arguments(
        &self,
        program: &crate::ProgramGeneration,
        site: RuntimeCallSite,
        source: &[Option<Expr>],
    ) -> bool {
        let Some(plan) = self.call_plan(site.plan) else {
            return false;
        };
        if matches!(plan.source, RuntimePlanSource::Arguments(_)) {
            let Some(pending) = &self.reference_arguments else {
                return false;
            };
            let Some(index) = pending.graph.template.nodes.iter().position(|node| {
                node.span == site.span
                    && matches!(node.kind, erabasic_bytecode::ReferenceTermKind::Call { .. })
            }) else {
                return false;
            };
            return pending.graph.expression(program, &super::reference_arguments::graph::TermRef::Original(match u32::try_from(index) { Ok(id) => id, Err(_) => return false }))
                .is_ok_and(|expression| matches!(expression.kind, ExprKind::Call { args, .. } if args == source));
        }
        plan.call_arguments(site.span) == Some(source)
    }
    pub(super) fn validate_planned_expression(
        &self,
        program: &crate::ProgramGeneration,
        plan_id: u64,
        expression: &Expr,
    ) -> bool {
        let Some(plan) = self.call_plan(plan_id) else {
            return false;
        };
        if matches!(plan.source, RuntimePlanSource::Form(_)) {
            return plan.expressions().any(|original| original == expression);
        }
        let Some(pending) = &self.reference_arguments else {
            return false;
        };
        let graph = &pending.graph;
        for (index, node) in graph.template.nodes.iter().enumerate() {
            if node.span == expression.span {
                let Ok(index) = u32::try_from(index) else {
                    return false;
                };
                if graph
                    .expression(
                        program,
                        &super::reference_arguments::graph::TermRef::Original(index),
                    )
                    .is_ok_and(|actual| actual == *expression)
                {
                    return true;
                }
            }
        }
        // Literal roots have no original source span after Restructure. They must
        // be a replacement already present in the separately validated graph.
        graph
            .edges
            .iter()
            .flatten()
            .chain(graph.roots.iter().flatten())
            .any(|term| {
                matches!(term, super::reference_arguments::graph::TermRef::Single(_))
                    && graph
                        .expression(program, term)
                        .is_ok_and(|actual| actual == *expression)
            })
    }
    pub(super) fn restore_call_plan(&mut self, previous: Option<u64>) -> Result<(), StepError> {
        if previous.is_some_and(|id| self.call_plan(id).is_none()) {
            return Err(invalid("runtime source plan restoration target is missing"));
        }
        let keep = previous
            .and_then(|id| self.call_plans.iter().position(|plan| plan.id == id))
            .map_or(0, |index| index + 1);
        if self
            .host_call_sites()
            .chain(self.staged_call_sites())
            .any(|site| {
                self.call_plans[keep..]
                    .iter()
                    .any(|plan| plan.id == site.plan)
            })
        {
            return Err(invalid(
                "runtime source plan would retire a live operation scope",
            ));
        }
        self.call_plans.truncate(keep);
        self.current_call_plan = previous;
        Ok(())
    }
    fn staged_call_sites(&self) -> impl Iterator<Item = RuntimeCallSite> + '_ {
        self.work.iter().filter_map(|task| match task {
            RuntimeFormTask::MapCapture { site, .. } | RuntimeFormTask::BitCapture { site, .. } => {
                Some(*site)
            }
            RuntimeFormTask::MapFinish(call) | RuntimeFormTask::MapValuesEnabled { call, .. } => {
                Some(call.site)
            }
            RuntimeFormTask::BitFinish(call) => Some(call.site),
            RuntimeFormTask::MatchBegin(call)
            | RuntimeFormTask::MatchEnd(call)
            | RuntimeFormTask::MatchNeedle(call)
            | RuntimeFormTask::MatchScan(call) => Some(call.site),
            _ => None,
        })
    }
    pub(super) fn valid_call_plans(&self, vm: &Vm) -> bool {
        let Some(program) = vm.generations.get(&self.generation) else {
            return false;
        };
        self.call_plans.len() <= super::MAX_RUNTIME_FORM_NESTING
            && self.next_call_plan > 0
            && self
                .call_plans
                .windows(2)
                .all(|pair| pair[0].id < pair[1].id)
            && self.call_plans.iter().all(|plan| {
                plan.id > 0
                    && plan.id < self.next_call_plan
                    && plan.valid(
                        program,
                        self.function,
                        self.generation,
                        vm.config.maximum_operand_stack.max(1),
                        self.reference_arguments
                            .as_ref()
                            .map(|pending| &pending.graph.template),
                    )
            })
            && self
                .current_call_plan
                .is_none_or(|id| self.call_plan(id).is_some())
            && self
                .host_call_sites()
                .all(|site| self.call_plan(site.plan).is_some())
            && self.work.iter().all(|task| match task {
                RuntimeFormTask::RestoreCallPlan(previous) => {
                    previous.is_none_or(|id| self.call_plan(id).is_some())
                }
                _ => true,
            })
    }
    pub(super) fn call_plan_resources(&self) -> Option<(usize, usize)> {
        let mut slots = 0usize;
        let mut bytes = 0usize;
        for plan in &self.call_plans {
            slots = slots
                .checked_add(1)?
                .checked_add(plan.types.len())?
                .checked_add(plan.calls.len())?;
            for expression in plan.expressions() {
                slots = slots.checked_add(1)?;
                let text = match &expression.kind {
                    ExprKind::String(text)
                    | ExprKind::Identifier(text)
                    | ExprKind::Variable { name: text, .. }
                    | ExprKind::Call { name: text, .. } => text.len(),
                    _ => 0,
                };
                bytes = bytes.checked_add(text)?;
            }
            // Form text fragments are not expressions; include them separately.
            let (form_slots, form_bytes) = source_form_resources(&plan.source)?;
            slots = slots.checked_add(form_slots)?;
            bytes = bytes.checked_add(form_bytes)?;
            for (_, bound) in &plan.calls {
                let (import, omitted) = match bound {
                    RuntimeBoundCall::Native(bound) => (&bound.import, &bound.omitted_arguments),
                    RuntimeBoundCall::Host(bound) => (&bound.import, &bound.omitted_arguments),
                    RuntimeBoundCall::Bit(_) => {
                        slots = slots.checked_add(1)?;
                        continue;
                    }
                    RuntimeBoundCall::Match(spec) => {
                        slots = slots.checked_add(1)?;
                        if let erabasic_bytecode::MatchInput::Name(name) = &spec.input {
                            bytes = bytes.checked_add(name.len())?;
                        }
                        continue;
                    }
                };
                slots = slots
                    .checked_add(import.parameters.len())?
                    .checked_add(omitted.len())?;
                bytes = bytes
                    .checked_add(import.name.len())?
                    .checked_add(import.namespace.len())?;
            }
        }
        Some((slots, bytes))
    }
}

enum SourceNode<'a> {
    Expression(&'a Expr),
    Form(&'a FormattedString),
}
struct SourceExpressions<'a> {
    pending: Vec<SourceNode<'a>>,
}
impl<'a> SourceExpressions<'a> {
    fn new(source: &'a RuntimePlanSource) -> Self {
        let pending = match source {
            RuntimePlanSource::Form(form) => vec![SourceNode::Form(form)],
            RuntimePlanSource::Arguments(arguments) => arguments
                .iter()
                .flatten()
                .map(SourceNode::Expression)
                .collect(),
        };
        Self { pending }
    }
}
impl<'a> Iterator for SourceExpressions<'a> {
    type Item = &'a Expr;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(node) = self.pending.pop() {
            match node {
                SourceNode::Form(form) => extend_form(&mut self.pending, form),
                SourceNode::Expression(expression) => {
                    extend_expression(&mut self.pending, expression);
                    return Some(expression);
                }
            }
        }
        None
    }
}
fn extend_expression<'a>(pending: &mut Vec<SourceNode<'a>>, expression: &'a Expr) {
    match &expression.kind {
        ExprKind::Variable { indices, .. } => {
            pending.extend(indices.iter().map(SourceNode::Expression));
        }
        ExprKind::Call { args, .. } => {
            pending.extend(args.iter().flatten().map(SourceNode::Expression));
        }
        ExprKind::Group(inner)
        | ExprKind::Unary { operand: inner, .. }
        | ExprKind::Postfix { operand: inner, .. } => pending.push(SourceNode::Expression(inner)),
        ExprKind::Binary { left, right, .. } => {
            pending.extend([SourceNode::Expression(left), SourceNode::Expression(right)]);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => pending
            .extend([condition, then_expr, else_expr].map(|value| SourceNode::Expression(value))),
        ExprKind::Formatted(form) => pending.push(SourceNode::Form(form)),
        _ => {}
    }
}
fn extend_form<'a>(pending: &mut Vec<SourceNode<'a>>, form: &'a FormattedString) {
    for part in &form.parts {
        match part {
            FormPart::StringInterpolation {
                expression, width, ..
            }
            | FormPart::IntegerInterpolation {
                expression, width, ..
            } => {
                pending.push(SourceNode::Expression(expression));
                pending.extend(width.iter().map(|value| SourceNode::Expression(value)));
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                pending.push(SourceNode::Expression(condition));
                pending.push(SourceNode::Form(then_value));
                pending.extend(else_value.iter().map(|value| SourceNode::Form(value)));
            }
            _ => {}
        }
    }
}
fn source_form_resources(source: &RuntimePlanSource) -> Option<(usize, usize)> {
    let mut pending = SourceExpressions::new(source).pending;
    let mut bytes = 0usize;
    let mut slots = 0usize;
    while let Some(node) = pending.pop() {
        match node {
            SourceNode::Expression(expression) => extend_expression(&mut pending, expression),
            SourceNode::Form(form) => {
                slots = slots.checked_add(1)?.checked_add(form.parts.len())?;
                for part in &form.parts {
                    if let FormPart::Text(text) = part {
                        bytes = bytes.checked_add(text.len())?;
                    }
                }
                extend_form(&mut pending, form);
            }
        }
    }
    Some((slots, bytes))
}
