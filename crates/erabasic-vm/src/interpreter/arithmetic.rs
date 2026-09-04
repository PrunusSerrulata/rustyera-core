//! Profile-aware integer execution. Diagnostics are drained at the instruction boundary.

use erabasic_compat::{IntegerArithmeticError, IntegerArithmeticPolicy, IntegerOperation};

use super::compatibility_diagnostics::CompatibilityWarning;
use super::{StepError, Vm, VmFaultCode, VmValue, operand};
use crate::GenerationId;

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
        if let Some(warning) = outcome.warning {
            self.queue_compatibility_warning(CompatibilityWarning::Arithmetic(warning));
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
}
