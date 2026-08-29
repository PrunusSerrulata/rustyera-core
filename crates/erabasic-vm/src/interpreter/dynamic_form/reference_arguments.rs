//! Shared bounded Argument.Restructure scheduling. Evaluation is exclusively the
//! existing `RuntimeForm` work machine, including user waits and Native transactions.
use super::{
    BytecodeFunctionKind, BytecodeType, Deserialize, Expr, ExprKind, Fiber, FormPart,
    FormattedString, MAX_RUNTIME_FORM_NESTING, RuntimeFormContinuation, RuntimeFormRoot,
    RuntimeFormTask, Serialize, StepError, SymbolKey, Vm, VmFaultCode, VmValue, resource_limit,
};
use erabasic_bytecode::{ReferenceTermCall, ReferenceTermGraph, ReferenceTermKind};
use graph::{PreparedReferenceArguments, TermRef};
use restructure::{Children, RestructureTask};
mod can_restructure;
mod from_ast;
pub(super) mod graph;
mod native_arguments;
mod restructure;
mod snapshot;
pub(super) use from_ast::GraphBuilder;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct PendingReferenceArguments {
    pub(super) graph: PreparedReferenceArguments,
    tasks: Vec<RestructureTask>,
    results: Vec<TermRef>,
    pub(super) preparing: bool,
}
fn invalid(message: impl Into<String>) -> StepError {
    StepError::new(VmFaultCode::InvalidInstruction, message)
}
impl PendingReferenceArguments {
    pub(super) fn new(template: ReferenceTermGraph, retained_roots: usize) -> Self {
        let graph = PreparedReferenceArguments::new(template);
        let mut tasks = vec![RestructureTask::Publish];
        for (index, root) in graph.roots.iter().enumerate().rev() {
            if let Some(root) = root {
                tasks.push(RestructureTask::CaptureRoot(index));
                tasks.push(RestructureTask::Visit {
                    term: root.clone(),
                    reject_constant_index: index >= retained_roots,
                });
            }
        }
        Self {
            graph,
            tasks,
            results: Vec::new(),
            preparing: true,
        }
    }
    fn valid(&self) -> bool {
        self.graph.template.validate_structure().is_ok()
            && self.graph.valid_for_template(&self.graph.template)
            && self.tasks.len() <= erabasic_bytecode::MAX_REFERENCE_TERM_NODES * 4 + 1
            && self.results.len() <= self.graph.template.nodes.len()
            && (self.preparing || self.tasks.is_empty() && self.results.is_empty())
            && self.results.iter().all(|term| self.valid_term(term))
            && self.tasks.iter().all(|task| self.valid_task(task))
    }
}
impl RuntimeFormContinuation {
    pub(super) fn reference_arguments_valid(&self) -> bool {
        self.reference_arguments.as_ref().is_none_or(|pending| {
            pending.valid()
                && matches!(self.completion, RuntimeFormRoot::Call { .. })
                && self
                    .work
                    .iter()
                    .filter(|task| {
                        matches!(
                            task,
                            RuntimeFormTask::FinishCallTextArguments { .. }
                                | RuntimeFormTask::ReleaseReferenceArguments
                        )
                    })
                    .count()
                    == 1
        }) && (!self.reference_bindings || self.reference_arguments.is_some())
            && self.work.iter().all(|task| match task {
                RuntimeFormTask::ReferenceArgumentsPump => self
                    .reference_arguments
                    .as_ref()
                    .is_some_and(|pending| pending.preparing),
                RuntimeFormTask::FinishCallTextArguments { spec, .. } => {
                    self.reference_arguments.is_some() && self.call_text_spec() == Some(*spec)
                }
                RuntimeFormTask::ReleaseReferenceArguments => self.reference_arguments.is_some(),
                _ => true,
            })
    }
    pub(super) fn reference_argument_resources(&self) -> Option<(usize, usize)> {
        let Some(pending) = &self.reference_arguments else {
            return Some((0, 0));
        };
        let mut slots = pending
            .graph
            .template
            .nodes
            .len()
            .checked_add(pending.graph.roots.len())?
            .checked_add(pending.tasks.len())?
            .checked_add(pending.results.len())?;
        let mut bytes = pending.graph.string_bytes()?;
        let literal_bytes = |term: &TermRef| match term {
            TermRef::Single(erabasic_bytecode::ReferenceTermValue::String(value)) => value.len(),
            _ => 0,
        };
        for node in &pending.graph.template.nodes {
            slots = slots.checked_add(node.children().len())?;
            bytes = bytes.checked_add(match &node.kind {
                ReferenceTermKind::Value(erabasic_bytecode::ReferenceTermValue::String(value)) => {
                    value.len()
                }
                ReferenceTermKind::Call {
                    target:
                        ReferenceTermCall::Native { name, .. }
                        | ReferenceTermCall::DynamicNative { name, .. }
                        | ReferenceTermCall::Host { name, .. }
                        | ReferenceTermCall::Intrinsic { name },
                    ..
                } => name.len(),
                ReferenceTermKind::Form { parts } => {
                    parts.iter().try_fold(0usize, |count, part| {
                        count.checked_add(match part {
                            erabasic_bytecode::ReferenceTermPart::Text(value) => value.len(),
                            _ => 0,
                        })
                    })?
                }
                _ => 0,
            })?;
        }
        for result in &pending.results {
            bytes = bytes.checked_add(literal_bytes(result))?;
        }
        for task in &pending.tasks {
            if let RestructureTask::Children(children) | RestructureTask::CaptureChild(children) =
                task
            {
                slots = slots
                    .checked_add(children.visits.len())?
                    .checked_add(children.results.len())?;
                for result in &children.results {
                    bytes = bytes.checked_add(literal_bytes(result))?;
                }
            }
        }
        Some((slots, bytes))
    }
    pub(super) fn advance_reference_arguments(&mut self, vm: &mut Vm) -> Result<(), StepError> {
        let mut pending = self
            .reference_arguments
            .take()
            .ok_or_else(|| invalid("reference argument pending state is missing"))?;
        let result = self.advance_reference_arguments_inner(vm, &mut pending);
        self.reference_arguments = Some(pending);
        result
    }

    fn advance_reference_arguments_inner(
        &mut self,
        vm: &mut Vm,
        pending: &mut PendingReferenceArguments,
    ) -> Result<(), StepError> {
        // The warm-cache entry leaves no pump task to execute.
        let task = pending
            .tasks
            .pop()
            .ok_or_else(|| invalid("reference argument reduction task is missing"))?;
        let program = std::sync::Arc::clone(
            vm.generations
                .get(&self.generation)
                .ok_or_else(|| invalid("reference argument generation is missing"))?,
        );
        let mut evaluate = None;
        let mut published = false;
        match task {
            RestructureTask::Visit {
                term,
                reject_constant_index,
            } => self.visit_reference_term(&program, pending, term, reject_constant_index)?,
            RestructureTask::Children(children) if children.next < children.visits.len() => {
                let edge = children.visits[children.next].0;
                let term = pending.graph.edges[children.node as usize][edge].clone();
                let reject_constant_index = children.reject_constant_index;
                pending.tasks.push(RestructureTask::CaptureChild(children));
                pending.tasks.push(RestructureTask::Visit {
                    term,
                    reject_constant_index,
                });
            }
            RestructureTask::CaptureChild(mut children) => {
                let term = pending
                    .results
                    .pop()
                    .ok_or_else(|| invalid("reference argument child result missing"))?;
                let (edge, assign) = children.visits[children.next];
                if assign {
                    pending.graph.edges[children.node as usize][edge] = term.clone();
                }
                children.results.push(term);
                children.next += 1;
                pending.tasks.push(RestructureTask::Children(children));
            }
            RestructureTask::Children(children) => {
                evaluate = self.finish_reference_children(&program, pending, &children)?;
            }
            RestructureTask::CaptureFold => {
                pending.results.push(PreparedReferenceArguments::from_value(
                    self.pop_value("reference argument folded result missing")?,
                )?);
            }
            RestructureTask::DiscardUniqueValue(node) => {
                self.pop_integer("reference argument REPLACE unique mode is not Integer")?;
                pending.results.push(TermRef::Original(node));
            }
            RestructureTask::CheckFormPredicate(node) => {
                evaluate = self.finish_reference_form_predicate(&program, pending, node)?;
            }
            RestructureTask::CaptureRoot(index) => {
                pending.graph.roots[index] = Some(
                    pending
                        .results
                        .pop()
                        .ok_or_else(|| invalid("reference argument root result missing"))?,
                );
            }
            RestructureTask::Publish => {
                if !pending.results.is_empty() || !pending.tasks.is_empty() {
                    return Err(invalid(
                        "reference argument publication has unfinished reduction",
                    ));
                }
                pending.preparing = false;
                published = true;
            }
        }
        if !published {
            self.work.push(RuntimeFormTask::ReferenceArgumentsPump);
            if let Some(expression) = evaluate {
                self.work.push(RuntimeFormTask::RestoreReferenceBindings(
                    self.reference_bindings,
                ));
                self.reference_bindings = true;
                self.work.push(RuntimeFormTask::Evaluate(expression));
            }
        }
        Ok(())
    }

    fn visit_reference_term(
        &mut self,
        program: &crate::ProgramGeneration,
        pending: &mut PendingReferenceArguments,
        term: TermRef,
        reject_constant_index: bool,
    ) -> Result<(), StepError> {
        self.remaining_nodes = self
            .remaining_nodes
            .checked_sub(1)
            .ok_or_else(|| resource_limit("reference argument restructuring node limit"))?;
        if pending.graph.single(&term).is_some() {
            pending.results.push(term);
        } else {
            let TermRef::Original(node) = term else {
                return Err(invalid("reference argument nonliteral replacement"));
            };
            if pending.graph.constant_index_out_of_range(program, node)? {
                if reject_constant_index {
                    pending.graph.check_constant_indices(program, node)?;
                }
                // Retained call arguments defer the original variable access to
                // ConvertArg, which is inside TRY. Preserve earlier index effects,
                // but do not evaluate indices after the known failing constant.
                pending.graph.defer_constant_index_failure(program, node)?;
                pending.results.push(TermRef::Original(node));
                return Ok(());
            }
            let visits = pending.graph.child_plan(program, node)?;
            pending.tasks.push(RestructureTask::Children(Children {
                node,
                reject_constant_index,
                visits,
                next: 0,
                results: Vec::new(),
            }));
        }
        Ok(())
    }

    fn finish_reference_children(
        &self,
        program: &crate::ProgramGeneration,
        pending: &mut PendingReferenceArguments,
        children: &Children,
    ) -> Result<Option<Expr>, StepError> {
        let mut evaluate = None;

        pending
            .graph
            .check_constant_indices(program, children.node)?;
        let original = TermRef::Original(children.node);
        let node = &pending.graph.template.nodes[children.node as usize];
        let name = match &node.kind {
            ReferenceTermKind::Call {
                target:
                    ReferenceTermCall::Native { name, .. }
                    | ReferenceTermCall::DynamicNative { name, .. }
                    | ReferenceTermCall::Host { name, .. }
                    | ReferenceTermCall::Intrinsic { name },
                ..
            } => Some(name.as_str()),
            _ => None,
        };
        if let ReferenceTermKind::Call { arguments, .. } = &node.kind {
            let dereferenced_slot = match name {
                Some("REPLACE") if arguments.len() >= 4 => Some(3),
                Some("VARSIZE") if arguments.len() >= 2 => Some(1),
                _ => None,
            };
            if dereferenced_slot.is_some_and(|slot| arguments[slot].node.is_none()) {
                // This particular null dereference is a source-proven script
                // operation failure, not malformed VM bytecode. CALLSTR's
                // binder catch does not enclose it; an outer STRFORMCHECK does.
                // TEXT's sticky-error policy must retain its CLR origin.
                return Err(StepError::script(
                    crate::ScriptFaultKind::Operation,
                    if matches!(
                        node.kind,
                        ReferenceTermKind::Call {
                            target: ReferenceTermCall::Host { .. },
                            ..
                        }
                    ) {
                        VmFaultCode::Host
                    } else {
                        VmFaultCode::Native
                    },
                    "reference unique method dereferenced an omitted argument",
                ));
            }
        }
        if name == Some("STRFORM") {
            let argument = pending.graph.edges[children.node as usize]
                .first()
                .ok_or_else(|| invalid("STRFORM argument missing"))?;
            if pending.graph.single(argument).is_some()
                || pending.graph.variable_const(program, argument)?
            {
                pending
                    .tasks
                    .push(RestructureTask::CheckFormPredicate(children.node));
                evaluate = Some(pending.graph.expression(program, argument)?);
            } else {
                pending.results.push(original);
            }
        } else if name == Some("REPLACE") {
            // UniqueRestructure executes arg3 even though CanRestructure=false.
            if let Some(argument) = pending.graph.edges[children.node as usize].get(3) {
                pending
                    .tasks
                    .push(RestructureTask::DiscardUniqueValue(children.node));
                evaluate = Some(pending.graph.expression(program, argument)?);
            } else {
                pending.results.push(original);
            }
        } else if pending.graph.may_fold(
            program,
            self.function,
            children.node,
            &children.results,
        )? {
            pending.tasks.push(RestructureTask::CaptureFold);
            evaluate = Some(pending.graph.expression(program, &original)?);
        } else {
            pending.results.push(original);
        }
        Ok(evaluate)
    }

    fn finish_reference_form_predicate(
        &mut self,
        program: &crate::ProgramGeneration,
        pending: &mut PendingReferenceArguments,
        node: u32,
    ) -> Result<Option<Expr>, StepError> {
        let mut evaluate = None;

        let VmValue::String(source) = self.pop_value("STRFORM predicate source missing")? else {
            return Err(invalid("STRFORM predicate source is not String"));
        };
        if self.reference_form_predicate(program, &source)? {
            // Evaluate the ORIGINAL call: arg0 is deliberately read a second time.
            pending.tasks.push(RestructureTask::CaptureFold);
            evaluate = Some(
                pending
                    .graph
                    .expression(program, &TermRef::Original(node))?,
            );
        } else {
            pending.results.push(TermRef::Original(node));
        }
        Ok(evaluate)
    }

    fn reference_form_predicate(
        &mut self,
        program: &crate::ProgramGeneration,
        source: &str,
    ) -> Result<bool, StepError> {
        if source.len() > self.remaining_source_bytes {
            return Err(resource_limit("STRFORM predicate source limit"));
        }
        self.remaining_source_bytes -= source.len();
        super::frontend::preflight_nesting(source)?;
        let policy = program.artifact.call_compatibility;
        let config = erabasic_lexer::LexerConfig {
            allow_full_width_space: policy.allow_full_width_space,
            debug_semicolon: policy.debug_semicolon,
            ignore_triple_symbols: policy.ignore_triple_symbols,
            ..Default::default()
        };
        let (form, diagnostics) =
            erabasic_lexer::lex_formatted(source, &config, &erabasic_lexer::MacroTable::new());
        Ok(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == erabasic_ast::Severity::Error)
            && form
                .parts
                .iter()
                .all(|part| matches!(part, erabasic_lexer::FormattedTokenPart::Text(_))))
    }
}

impl PendingReferenceArguments {
    fn valid_term(&self, term: &TermRef) -> bool {
        match term {
            TermRef::Original(id) => (*id as usize) < self.graph.template.nodes.len(),
            TermRef::Single(_) => true,
        }
    }
    fn valid_task(&self, task: &RestructureTask) -> bool {
        let node = |id: &u32| (*id as usize) < self.graph.template.nodes.len();
        match task {
            RestructureTask::Visit {
                term: TermRef::Original(id),
                ..
            }
            | RestructureTask::CheckFormPredicate(id)
            | RestructureTask::DiscardUniqueValue(id) => node(id),
            RestructureTask::Visit {
                term: TermRef::Single(_),
                ..
            }
            | RestructureTask::CaptureFold
            | RestructureTask::Publish => true,
            RestructureTask::CaptureRoot(index) => {
                self.graph.roots.get(*index).is_some_and(Option::is_some)
            }
            RestructureTask::Children(state) | RestructureTask::CaptureChild(state) => {
                node(&state.node)
                    && state.next <= state.visits.len()
                    && (!matches!(task, RestructureTask::CaptureChild(_))
                        || state.next < state.visits.len())
                    && state.results.len() == state.next
                    && state.results.iter().all(|term| self.valid_term(term))
                    && state
                        .visits
                        .iter()
                        .all(|(edge, _)| *edge < self.graph.edges[state.node as usize].len())
            }
        }
    }
}
