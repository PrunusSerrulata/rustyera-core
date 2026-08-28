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
        // C# CallFunction rejects Method/Event here, outside TRY. Use the shared
        // target-kind helper rather than reimplementing profile rules in this module.
        validate_user_call_target_kind(program, target, spec.mode.user_call_mode())
            .map_err(map_vm_error)?;
        let mut arguments = parsed
            .arguments
            .into_iter()
            .map(call_text_argument)
            .collect::<Result<Vec<_>, _>>()?;
        // Restructure/name/type checks happen for every actual before ConvertArg,
        // and their failures are deliberately not caught by this instruction's TRY.
        let mut specs = Vec::with_capacity(arguments.len());
        for argument in &mut arguments {
            let Some(expression) = argument else {
                specs.push(erabasic_bytecode::UserArgumentSpec::Omitted);
                continue;
            };
            frontend::resolve_expression_named_indices(program, self.function, expression, 0)?;
            let (nodes, shape) = frontend::validate_runtime_expression(
                vm,
                natives,
                self.generation,
                self.function,
                expression,
                self.remaining_nodes,
            )?;
            self.remaining_nodes = self.remaining_nodes.saturating_sub(nodes);
            specs.push(shape);
        }
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
                return self.fail_call_text(spec);
            }
            Err(failure) => return Err(failure),
        };
        // TRY is now over: argument execution, service/callee failures propagate normally.
        self.queue_resolved_call(vm, call, specs, arguments);
        Ok(())
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
