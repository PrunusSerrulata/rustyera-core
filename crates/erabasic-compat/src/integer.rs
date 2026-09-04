//! Integer operations shared by load-time evaluation and VM execution.

/// The exact arithmetic behavior selected by a validated compatibility identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerArithmeticPolicy {
    ReferenceWrappingV1,
    SnakeSaturatingV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Negate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerArithmeticWarning {
    Overflow,
    DivideByZero,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerArithmeticOutcome {
    pub value: i64,
    pub warning: Option<IntegerArithmeticWarning>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerArithmeticError {
    DivideByZero,
    Overflow,
    InvalidOperands,
}

impl std::fmt::Display for IntegerArithmeticError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::DivideByZero => "integer division or remainder by zero",
            Self::Overflow => "integer division or remainder overflow",
            Self::InvalidOperands => "invalid integer operation operands",
        })
    }
}

impl std::error::Error for IntegerArithmeticError {}

impl IntegerArithmeticPolicy {
    /// Evaluate a language operation without emitting or suppressing its diagnostic.
    ///
    /// # Errors
    /// Returns an error for invalid operands or an arithmetic fault. Negation requires
    /// no right operand; all binary operations require one.
    pub fn evaluate(
        self,
        operation: IntegerOperation,
        left: i64,
        right: Option<i64>,
    ) -> Result<IntegerArithmeticOutcome, IntegerArithmeticError> {
        let right = match (operation, right) {
            (IntegerOperation::Negate, None) => 0,
            (IntegerOperation::Negate, Some(_)) | (_, None) => {
                return Err(IntegerArithmeticError::InvalidOperands);
            }
            (_, Some(right)) => right,
        };
        let snake = self == Self::SnakeSaturatingV1;
        if matches!(
            operation,
            IntegerOperation::Divide | IntegerOperation::Modulo
        ) {
            if right == 0 {
                return if snake {
                    Ok(IntegerArithmeticOutcome {
                        value: 0,
                        warning: Some(IntegerArithmeticWarning::DivideByZero),
                    })
                } else {
                    Err(IntegerArithmeticError::DivideByZero)
                };
            }
            // The fixed .NET oracle faults for both MIN / -1 and MIN % -1.
            let value = match operation {
                IntegerOperation::Divide => left.checked_div(right),
                _ => left.checked_rem(right),
            }
            .ok_or(IntegerArithmeticError::Overflow)?;
            return Ok(IntegerArithmeticOutcome {
                value,
                warning: None,
            });
        }
        let (wrapped, overflow) = match operation {
            IntegerOperation::Add => left.overflowing_add(right),
            IntegerOperation::Subtract => left.overflowing_sub(right),
            IntegerOperation::Multiply => left.overflowing_mul(right),
            IntegerOperation::Negate => left.overflowing_neg(),
            IntegerOperation::Divide | IntegerOperation::Modulo => unreachable!(),
        };
        if !snake || !overflow {
            return Ok(IntegerArithmeticOutcome {
                value: wrapped,
                warning: None,
            });
        }
        // SafeSubtract deliberately selects the endpoint from the left operand,
        // including its unusual 0 - MIN => MIN result. Preserve that observable rule.
        let positive = match operation {
            IntegerOperation::Add | IntegerOperation::Subtract => left > 0,
            IntegerOperation::Multiply => (left > 0) == (right > 0),
            IntegerOperation::Negate => true,
            IntegerOperation::Divide | IntegerOperation::Modulo => unreachable!(),
        };
        Ok(IntegerArithmeticOutcome {
            value: if positive { i64::MAX } else { i64::MIN },
            warning: Some(IntegerArithmeticWarning::Overflow),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_preserves_reference_and_fixed_snake_boundaries() {
        let cases = [
            (IntegerOperation::Add, i64::MAX, Some(1), i64::MIN, i64::MAX),
            (
                IntegerOperation::Subtract,
                0,
                Some(i64::MIN),
                i64::MIN,
                i64::MIN,
            ),
            (
                IntegerOperation::Multiply,
                i64::MIN,
                Some(-1),
                i64::MIN,
                i64::MAX,
            ),
            (IntegerOperation::Negate, i64::MIN, None, i64::MIN, i64::MAX),
        ];
        for (operation, left, right, reference, snake) in cases {
            assert_eq!(
                IntegerArithmeticPolicy::ReferenceWrappingV1.evaluate(operation, left, right),
                Ok(IntegerArithmeticOutcome {
                    value: reference,
                    warning: None
                })
            );
            assert_eq!(
                IntegerArithmeticPolicy::SnakeSaturatingV1.evaluate(operation, left, right),
                Ok(IntegerArithmeticOutcome {
                    value: snake,
                    warning: Some(IntegerArithmeticWarning::Overflow),
                })
            );
        }
        for operation in [IntegerOperation::Divide, IntegerOperation::Modulo] {
            assert_eq!(
                IntegerArithmeticPolicy::SnakeSaturatingV1.evaluate(operation, 7, Some(0)),
                Ok(IntegerArithmeticOutcome {
                    value: 0,
                    warning: Some(IntegerArithmeticWarning::DivideByZero),
                })
            );
            for policy in [
                IntegerArithmeticPolicy::ReferenceWrappingV1,
                IntegerArithmeticPolicy::SnakeSaturatingV1,
            ] {
                assert_eq!(
                    policy.evaluate(operation, i64::MIN, Some(-1)),
                    Err(IntegerArithmeticError::Overflow)
                );
                assert_eq!(
                    policy.evaluate(operation, -7, Some(3)).unwrap().warning,
                    None
                );
            }
        }
        assert_eq!(
            IntegerArithmeticPolicy::ReferenceWrappingV1.evaluate(
                IntegerOperation::Divide,
                1,
                Some(0)
            ),
            Err(IntegerArithmeticError::DivideByZero)
        );
        assert_eq!(
            IntegerArithmeticPolicy::SnakeSaturatingV1.evaluate(
                IntegerOperation::Negate,
                1,
                Some(1)
            ),
            Err(IntegerArithmeticError::InvalidOperands)
        );
    }
}
