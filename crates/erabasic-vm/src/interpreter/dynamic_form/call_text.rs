//! CALLSTR stages share the form work machine but do not create a general script catcher.
use super::{
    Fiber, NativeServiceRegistry, RuntimeFormContinuation, RuntimeFormRoot, StepError, Vm,
    VmFaultCode, frontend, map_vm_error, resource_limit,
};
use crate::state::user_calls::{bind_user_call_signature, validate_user_call_target_kind};
use erabasic_ast::Argument;
use erabasic_bytecode::{CallTextSpec, UserCallSpec};
use erabasic_parser::{CallTextParseStage, parse_call_text_at};

impl RuntimeFormContinuation {
    pub(super) fn parse_call_text(
        &mut self,
        vm: &mut Vm,
        _fiber: &Fiber,
        natives: &NativeServiceRegistry,
        source: &str,
        spec: CallTextSpec,
    ) -> Result<(), StepError> {
        if source.len() > self.remaining_source_bytes {
            return Err(resource_limit(
                "CALLSTR source exceeds the runtime parser limit",
            ));
        }
        self.remaining_source_bytes -= source.len();
        frontend::preflight_nesting(source)?;
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "CALLSTR generation is missing")
        })?;
        let context = frontend::parser_context(program);
        let parsed = match parse_call_text_at(source, 0, &context) {
            Ok(parsed) => parsed,
            Err(error) => {
                let failure = StepError::script(
                    crate::ScriptFaultKind::Parse,
                    VmFaultCode::Native,
                    error
                        .diagnostics
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                );
                // The C# lexer is outside the protected ReduceArguments pass.
                if error.stage == CallTextParseStage::Arguments && spec.mode.allows_missing() {
                    return self.fail_call_text(spec);
                }
                return Err(failure);
            }
        };
        let Some(parsed) = parsed.call else {
            return Ok(());
        }; // Blank JUMP is a no-op.
        let mut arguments = parsed
            .arguments
            .into_iter()
            .map(call_text_argument)
            .collect::<Result<Vec<_>, _>>()?;
        // ReduceArguments resolves variable/method names and expression types before
        // CallFunction, inside TRY. This pass only inspects source/schema: actual
        // storage reads and argument execution remain outside the protected stage.
        let graph = match self.prepare_call_text_arguments(vm, natives, &mut arguments) {
            Ok(graph) => graph,
            Err(failure)
                if spec.mode.allows_missing()
                    && matches!(
                        failure.category,
                        crate::FaultCategory::Script(
                            crate::ScriptFaultKind::Resolve | crate::ScriptFaultKind::Argument
                        )
                    ) =>
            {
                return self.fail_call_text(spec);
            }
            Err(failure) => return Err(failure),
        };
        let Some(target) = program.function_by_name(&parsed.target) else {
            if spec.mode.allows_missing() {
                return self.fail_call_text(spec);
            }
            return Err(StepError::script(
                crate::ScriptFaultKind::Resolve,
                VmFaultCode::MissingSymbol,
                format!("CALLSTR target {} is missing", parsed.target),
            ));
        };
        // Target kind errors originate after argument construction and are outside TRY.
        validate_user_call_target_kind(program, target, spec.mode.user_call_mode())
            .map_err(map_vm_error)?;
        // CALLSTR restructures ALL outer terms before ConvertArg, including
        // excess ones. The bounded pump is not enclosed in TRY's binder catch.
        self.reference_arguments =
            Some(super::reference_arguments::PendingReferenceArguments::new(
                graph,
                target.parameters.len(),
            ));
        self.work
            .push(super::RuntimeFormTask::FinishCallTextArguments {
                target: target.key,
                spec,
            });
        self.work
            .push(super::RuntimeFormTask::ReferenceArgumentsPump);
        Ok(())
    }

    pub(super) fn finish_call_text_arguments(
        &mut self,
        vm: &mut Vm,
        fiber: &Fiber,
        target: erabasic_bytecode::SymbolKey,
        spec: CallTextSpec,
    ) -> Result<(), StepError> {
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "CALLSTR generation disappeared")
        })?;
        let target = program.function(target).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "CALLSTR target disappeared")
        })?;
        let pending = self
            .reference_arguments
            .as_ref()
            .filter(|pending| !pending.preparing)
            .ok_or_else(|| {
                StepError::new(
                    VmFaultCode::InvalidInstruction,
                    "CALLSTR binding preceded restructuring",
                )
            })?;
        let arguments = pending
            .graph
            .roots
            .iter()
            .map(|root| {
                root.as_ref()
                    .map(|root| pending.graph.expression(program, root))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let specs = pending
            .graph
            .roots
            .iter()
            .zip(&arguments)
            .map(|(root, expression)| {
                let Some(root) = root else {
                    return erabasic_bytecode::UserArgumentSpec::Omitted;
                };
                let kind = match root {
                    super::reference_arguments::graph::TermRef::Single(value) => value.value_type(),
                    super::reference_arguments::graph::TermRef::Original(id) => {
                        pending.graph.template.nodes[*id as usize].value_type
                    }
                };
                super::typing::shape_spec(
                    program,
                    self.function,
                    expression.as_ref().expect("present root"),
                    kind,
                )
            })
            .collect::<Vec<_>>();
        let resolved = bind_user_call_signature(
            program,
            self.generation,
            target,
            &UserCallSpec {
                mode: spec.mode.user_call_mode(),
                allow_missing: false,
                missing_target: 0,
                arguments: specs.clone(),
            },
        )
        .map_err(map_vm_error);
        let call = match resolved {
            Ok(call) => call,
            Err(failure)
                if spec.mode.allows_missing()
                    && matches!(
                        failure.category,
                        crate::FaultCategory::Script(crate::ScriptFaultKind::Argument)
                    ) =>
            {
                self.reference_arguments = None;
                return self.fail_call_text(spec);
            }
            Err(failure) => return Err(failure),
        };
        // All Restructure effects have already happened outside TRY. ConvertArg
        // remains protected; subsequent ordinary actual/callee execution is not.
        self.work
            .push(super::RuntimeFormTask::ReleaseReferenceArguments);
        self.reference_bindings = true;
        let checkpoint = self.begin_call_text_argument_checkpoint(fiber, spec)?;
        self.queue_resolved_call(vm, call, specs, arguments)?;
        let Some(super::RuntimeFormTask::MethodArgument(call)) = self.work.last_mut() else {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "CALLSTR argument checkpoint lacks its user-call state",
            ));
        };
        call.argument_checkpoint = checkpoint;
        Ok(())
    }

    fn prepare_call_text_arguments(
        &mut self,
        vm: &Vm,
        natives: &NativeServiceRegistry,
        arguments: &mut [Option<erabasic_ast::Expr>],
    ) -> Result<erabasic_bytecode::ReferenceTermGraph, StepError> {
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(VmFaultCode::MissingSymbol, "CALLSTR generation is missing")
        })?;
        // Resolve in source order first. This source-only pass includes excess
        // nested arguments, as ReduceArguments/Create precede ConvertArg.
        for expression in arguments.iter_mut().flatten() {
            frontend::resolve_expression_named_indices(program, self.function, expression, 0)?;
        }
        let mut analysis = super::typing::TypeAnalysis::new(
            program,
            self.function,
            self.generation,
            false,
            self.remaining_nodes,
            Some(natives),
        );
        analysis.reference_terms = true;
        for expression in arguments.iter().flatten() {
            analysis.expression(expression, 0)?;
        }
        self.remaining_nodes = self.remaining_nodes.saturating_sub(analysis.nodes());
        let graph = super::reference_arguments::GraphBuilder::new(
            program,
            self.function,
            &analysis.expression_types,
        )
        .build(arguments)?;
        let plan = super::call_plan::RuntimeCallPlan::from_analysis(
            super::call_plan::RuntimePlanSource::Arguments(arguments.to_vec()),
            analysis,
        )?;
        self.install_call_plan(plan)?;
        Ok(graph)
    }

    fn fail_call_text(&mut self, spec: CallTextSpec) -> Result<(), StepError> {
        if !matches!(self.completion, RuntimeFormRoot::Call { spec: root, .. } if root == spec) {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "CALLSTR failure root differs",
            ));
        }
        self.completion = RuntimeFormRoot::Call { spec, failed: true };
        Ok(())
    }
}

fn call_text_argument(argument: Argument) -> Result<Option<erabasic_ast::Expr>, StepError> {
    match argument {
        Argument::Expression(expression) => Ok(Some(expression)),
        Argument::Omitted(_) => Ok(None),
        _ => Err(StepError::new(
            VmFaultCode::InvalidInstruction,
            "CALLSTR parser produced an invalid actual shape",
        )),
    }
}
