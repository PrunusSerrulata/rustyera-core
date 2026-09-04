//! Lower already-bound source operands through the existing value/place tasks.
use super::{RuntimeFormContinuation, RuntimeFormTask};
use crate::VmFaultCode;
use crate::interpreter::StepError;
use erabasic_ast::{Expr, ExprKind};
use erabasic_bytecode::BytecodeType;
impl RuntimeFormContinuation {
    pub(super) fn schedule_bound_source_arguments(
        &mut self,
        program: &crate::ProgramGeneration,
        args: &[Option<Expr>],
        parameters: &[BytecodeType],
    ) -> Result<(), StepError> {
        for (argument, parameter) in args.iter().zip(parameters).rev() {
            if matches!(
                parameter,
                BytecodeType::IntegerPlace | BytecodeType::StringPlace
            ) {
                let expression = argument.as_ref().ok_or_else(|| {
                    StepError::new(VmFaultCode::InvalidInstruction, "omitted Host/Native place")
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
                            "bound Host/Native place lost its variable",
                        ));
                    }
                };
                let variable = program
                    .scoped_variable(self.function, name)
                    .ok_or_else(|| {
                        StepError::new(
                            VmFaultCode::MissingSymbol,
                            "Host/Native place variable missing",
                        )
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
}
