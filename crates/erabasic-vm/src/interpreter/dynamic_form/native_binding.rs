//! Dynamic source binding uses a finite grant and the normal Native execution boundary.
use super::call_plan::{RuntimeBoundCall, RuntimeCallSite};
use super::{
    BytecodeType, Expr, ExprKind, NativeServiceRegistry, RuntimeFormContinuation, RuntimeFormTask,
    StepError, Vm, VmFaultCode, support,
};
use erabasic_bytecode::{BoundRuntimeNative, RuntimeExpressionShape, RuntimeNativeAuthorization};

pub(super) fn authorization<'a>(
    program: &'a crate::ProgramGeneration,
    name: &str,
) -> Result<&'a RuntimeNativeAuthorization, StepError> {
    if name.eq_ignore_ascii_case("STRFORMCHECK")
        && !program
            .artifact
            .manifest
            .compatibility
            .supports_checked_runtime_forms()
    {
        return Err(support::permission_denied(
            "STRFORMCHECK is unavailable in this compatibility identity",
        ));
    }
    program
        .artifact
        .runtime_native_authorizations
        .iter()
        .find(|family| family.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            support::permission_denied(format!(
                "runtime callable {name} lacks a Native authorization"
            ))
        })
}
pub(super) fn bind(
    program: &crate::ProgramGeneration,
    name: &str,
    shapes: &[Option<RuntimeExpressionShape>],
    natives: Option<&NativeServiceRegistry>,
) -> Result<BoundRuntimeNative, StepError> {
    if crate::structured::is_internal_column_native(name) {
        return Err(support::permission_denied(
            "runtime text cannot invoke an internal column operation",
        ));
    }
    let symbol = program
        .artifact
        .runtime_builtins
        .iter()
        .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            StepError::script(
                crate::ScriptFaultKind::Resolve,
                VmFaultCode::MissingSymbol,
                format!("runtime callable {name} is missing"),
            )
        })?;
    let family = authorization(program, name)?;
    let bound = family.bind(shapes).ok_or_else(|| {
        StepError::script(
            crate::ScriptFaultKind::Argument,
            VmFaultCode::TypeMismatch,
            format!(
                "runtime callable {} has incompatible source arguments",
                symbol.name
            ),
        )
    })?;
    if let Some(natives) = natives {
        require_provider(natives, family)?;
    }
    Ok(bound)
}
pub(super) fn require_provider(
    natives: &NativeServiceRegistry,
    family: &RuntimeNativeAuthorization,
) -> Result<(), StepError> {
    // This exact contract is verified at load as well; provider names cannot forge a weaker state policy.
    if family.contract != erabasic_bytecode::canonical_native_contract(&family.name) {
        return Err(support::permission_denied(
            "runtime Native provider contract differs from canonical contract",
        ));
    }
    if !natives.contains(family.key)
        && !crate::interpreter::special_native::owns_native(&family.name)
        && !matches!(
            family.name.as_str(),
            "strform" | "strformcheck" | "existmeth" | "existvar"
        )
    {
        return Err(StepError::classified(
            crate::FaultCategory::HostContract,
            VmFaultCode::Native,
            format!("runtime native provider {} is missing", family.name),
        ));
    }
    Ok(())
}

impl RuntimeFormContinuation {
    pub(super) fn schedule_native_arguments(
        &mut self,
        vm: &Vm,
        bound: &BoundRuntimeNative,
        args: &[Option<Expr>],
        site: RuntimeCallSite,
    ) -> Result<(), StepError> {
        let program = vm.generations.get(&self.generation).ok_or_else(|| {
            StepError::new(
                VmFaultCode::MissingSymbol,
                "runtime Native generation is missing",
            )
        })?;
        self.work.push(RuntimeFormTask::FinishNative {
            site,
            bound: bound.clone(),
            source: args.to_vec(),
        });
        for (argument, parameter) in args.iter().zip(&bound.import.parameters).rev() {
            if matches!(
                parameter,
                BytecodeType::IntegerPlace | BytecodeType::StringPlace
            ) {
                let expression = argument.as_ref().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "omitted Native place")
                })?;
                let mut expression = expression;
                while let ExprKind::Group(inner) = &expression.kind {
                    expression = inner;
                }
                let (name, indices) = match &expression.kind {
                    ExprKind::Variable { name, indices } => (name, indices.as_slice()),
                    ExprKind::Identifier(name) => (name, &[][..]),
                    _ => {
                        return Err(StepError::new(
                            VmFaultCode::InvalidInstruction,
                            "bound Native place lost its variable",
                        ));
                    }
                };
                let variable = program
                    .scoped_variable(self.function, name)
                    .ok_or_else(|| {
                        StepError::new(VmFaultCode::MissingSymbol, "Native place variable missing")
                    })?;
                self.work.push(RuntimeFormTask::CaptureReferencePlace {
                    key: variable.key,
                    indices: indices.len(),
                });
                self.work
                    .extend(indices.iter().rev().cloned().map(RuntimeFormTask::Evaluate));
            } else {
                self.work.push(
                    argument
                        .clone()
                        .map_or(RuntimeFormTask::PushOmitted, RuntimeFormTask::Evaluate),
                );
            }
        }
        Ok(())
    }
    pub(super) fn schedule_planned_call(
        &mut self,
        vm: &Vm,
        span: erabasic_ast::Span,
        args: &[Option<Expr>],
    ) -> Result<(), StepError> {
        let site = self.current_call_site(span)?;
        match self.lookup_bound_call(site).cloned() {
            Some(RuntimeBoundCall::Native(bound)) => {
                self.schedule_native_arguments(vm, &bound, args, site)
            }
            Some(RuntimeBoundCall::Host(bound)) => {
                self.schedule_host_arguments(vm, &bound, args, site)
            }
            None => Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "runtime call lost its analyzed binding",
            )),
        }
    }
    pub(super) fn valid_native_task(
        &self,
        program: &crate::ProgramGeneration,
        site: RuntimeCallSite,
        bound: &BoundRuntimeNative,
        source: &[Option<Expr>],
    ) -> bool {
        self.lookup_bound_call(site) == Some(&RuntimeBoundCall::Native(bound.clone()))
            && self.validate_call_arguments(program, site, source)
    }
}
