//! Profile-aware integer execution. Diagnostics are drained at the instruction boundary.

use erabasic_compat::{
    IntegerArithmeticError, IntegerArithmeticPolicy, IntegerArithmeticWarning, IntegerOperation,
};

use super::{InstructionPosition, StepError, Vm, VmEvent, VmFaultCode, VmValue, operand};
use crate::{FiberId, GenerationId, VmDiagnosticNotification};

impl Vm {
    pub(super) fn integer_policy(&self, generation: GenerationId) -> IntegerArithmeticPolicy {
        self.generations[&generation]
            .artifact
            .manifest
            .compatibility
            .integer_arithmetic_policy()
    }

    pub(super) fn integer_arithmetic(
        &mut self,
        generation: GenerationId,
        operation: IntegerOperation,
        left: i64,
        right: Option<i64>,
    ) -> Result<i64, StepError> {
        let outcome = self
            .integer_policy(generation)
            .evaluate(operation, left, right)
            .map_err(|error| {
                let message = format!("integer {operation:?} failed: {error:?}");
                match error {
                    IntegerArithmeticError::InvalidOperands => {
                        StepError::new(VmFaultCode::InvalidInstruction, message)
                    }
                    IntegerArithmeticError::DivideByZero | IntegerArithmeticError::Overflow => {
                        StepError::script(
                            crate::ScriptFaultKind::Arithmetic,
                            if error == IntegerArithmeticError::DivideByZero {
                                VmFaultCode::DivideByZero
                            } else {
                                VmFaultCode::InvalidInstruction
                            },
                            message,
                        )
                    }
                }
            })?;
        if let Some(warning) = outcome.warning
            && !self.pending_arithmetic_warnings.contains(&warning)
        {
            // There are only two warning kinds; this queue cannot grow with expression size.
            self.pending_arithmetic_warnings.push(warning);
        }
        Ok(outcome.value)
    }

    pub(super) fn unary_value(
        &mut self,
        generation: GenerationId,
        operation: u8,
        value: VmValue,
    ) -> Result<VmValue, StepError> {
        if self.integer_policy(generation) == IntegerArithmeticPolicy::ReferenceWrappingV1 {
            return operand::unary_value(operation, value);
        }
        let (arithmetic, right) = match operation {
            1 => (IntegerOperation::Negate, None),
            4 | 6 => (IntegerOperation::Add, Some(1)),
            5 | 7 => (IntegerOperation::Subtract, Some(1)),
            _ => return operand::unary_value(operation, value),
        };
        let VmValue::Integer(value) = value else {
            return operand::unary_value(operation, value);
        };
        self.integer_arithmetic(generation, arithmetic, value, right)
            .map(VmValue::Integer)
    }

    pub(super) fn binary_value(
        &mut self,
        generation: GenerationId,
        operation: u8,
        left: VmValue,
        right: VmValue,
    ) -> Result<VmValue, StepError> {
        if self.integer_policy(generation) == IntegerArithmeticPolicy::ReferenceWrappingV1 {
            return operand::binary_value(operation, left, right);
        }
        let arithmetic = match operation {
            0 => IntegerOperation::Multiply,
            1 => IntegerOperation::Divide,
            2 => IntegerOperation::Modulo,
            3 => IntegerOperation::Add,
            4 => IntegerOperation::Subtract,
            _ => return operand::binary_value(operation, left, right),
        };
        match (left, right) {
            (VmValue::Integer(left), VmValue::Integer(right)) => self
                .integer_arithmetic(generation, arithmetic, left, Some(right))
                .map(VmValue::Integer),
            (left, right) => operand::binary_value(operation, left, right),
        }
    }

    pub(super) fn drain_arithmetic_diagnostics(
        &mut self,
        fiber: FiberId,
        position: &InstructionPosition<'_>,
        events: &mut Vec<VmEvent>,
    ) {
        if self.pending_arithmetic_warnings.is_empty() {
            return;
        }
        // Memo entries do not store diagnostics. Never publish a trace that observed one,
        // including a duplicate already reported at this source position.
        self.invalidate_path_memo(fiber);
        self.active_function_memos.clear();
        let command = self.command_for_position(position);
        let origin = self.execution_origin(position, &command);
        for warning in self.pending_arithmetic_warnings.drain(..) {
            let (tag, code, message) = match warning {
                IntegerArithmeticWarning::Overflow => (
                    0,
                    "compat.arithmetic.overflow",
                    "integer arithmetic overflowed; snake saturation policy applied",
                ),
                IntegerArithmeticWarning::DivideByZero => (
                    1,
                    "compat.arithmetic.divide_by_zero",
                    "integer division or remainder by zero returned zero under snake policy",
                ),
            };
            let site = (
                position.generation,
                position.function,
                position.instruction,
                tag,
            );
            if self.arithmetic_warning_sites.insert(site) {
                events.push(VmEvent::Diagnostic {
                    fiber,
                    code: code.into(),
                    message: message.into(),
                    origin: origin.clone(),
                    notification: VmDiagnosticNotification::LogOnly,
                });
            }
        }
    }
}
