//! Bytecode operand decoding and stack arithmetic helpers.

use super::{StepError, VmError, VmFaultCode, VmValue};

pub(super) fn exact<const N: usize>(payload: &[u8]) -> Result<[u8; N], StepError> {
    payload.try_into().map_err(|_| {
        StepError::new(
            VmFaultCode::InvalidInstruction,
            format!("expected {N} operand bytes, found {}", payload.len()),
        )
    })
}

pub(super) fn read_u16(payload: &[u8], offset: usize) -> Result<u16, StepError> {
    Ok(u16::from_le_bytes(exact_slice(payload, offset)?))
}

pub(super) fn read_u32(payload: &[u8], offset: usize) -> Result<u32, StepError> {
    Ok(u32::from_le_bytes(exact_slice(payload, offset)?))
}

pub(super) fn exact_slice<const N: usize>(
    payload: &[u8],
    offset: usize,
) -> Result<[u8; N], StepError> {
    payload
        .get(offset..offset + N)
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| StepError::new(VmFaultCode::InvalidInstruction, "truncated operand"))
}

pub(super) fn pop(stack: &mut Vec<VmValue>) -> Result<VmValue, StepError> {
    stack
        .pop()
        .ok_or_else(|| StepError::new(VmFaultCode::StackUnderflow, "operand stack underflow"))
}

pub(super) fn concat_strings(stack: &mut Vec<VmValue>, count: usize) -> Result<String, StepError> {
    let available = stack.len();
    for offset in 0..count.min(available) {
        let index = available - offset - 1;
        if !matches!(stack[index], VmValue::String(_)) {
            stack.truncate(index);
            return Err(StepError::new(
                VmFaultCode::TypeMismatch,
                "concat expects strings",
            ));
        }
    }
    if count > available {
        stack.clear();
        return Err(StepError::new(
            VmFaultCode::StackUnderflow,
            "operand stack underflow",
        ));
    }
    if count == 0 {
        return Ok(String::new());
    }

    let start = available - count;
    let total_bytes = stack[start..]
        .iter()
        .map(|value| match value {
            VmValue::String(value) => value.len(),
            _ => unreachable!("concat parts were validated before measuring"),
        })
        .sum::<usize>();
    let mut parts = stack.drain(start..);
    let VmValue::String(mut result) = parts.next().expect("validated concat part exists") else {
        unreachable!("concat parts were validated before draining")
    };
    result.reserve(total_bytes - result.len());
    for part in parts {
        let VmValue::String(part) = part else {
            unreachable!("concat parts were validated before draining")
        };
        result.push_str(&part);
    }
    Ok(result)
}

pub(super) struct PoppedIndices {
    inline: [u64; 4],
    length: usize,
    overflow: Vec<u64>,
}

impl PoppedIndices {
    pub(super) fn as_slice(&self) -> &[u64] {
        if self.length <= self.inline.len() {
            &self.inline[..self.length]
        } else {
            &self.overflow
        }
    }
}

pub(super) fn pop_indices(
    stack: &mut Vec<VmValue>,
    count: usize,
) -> Result<PoppedIndices, StepError> {
    let mut indices = PoppedIndices {
        inline: [0; 4],
        length: count,
        overflow: if count > 4 {
            vec![0; count]
        } else {
            Vec::new()
        },
    };
    for index in (0..count).rev() {
        let VmValue::Integer(value) = pop(stack)? else {
            return Err(StepError::new(
                VmFaultCode::TypeMismatch,
                "variable indices must be integers",
            ));
        };
        let value = u64::try_from(value).map_err(|_| {
            StepError::script(
                crate::ScriptFaultKind::Bounds,
                VmFaultCode::Bounds,
                "variable index cannot be negative",
            )
        })?;
        if count <= indices.inline.len() {
            indices.inline[index] = value;
        } else {
            indices.overflow[index] = value;
        }
    }
    Ok(indices)
}

pub(super) fn pop_arguments(
    stack: &mut Vec<VmValue>,
    count: usize,
) -> Result<Vec<VmValue>, StepError> {
    let mut arguments = Vec::with_capacity(count);
    for _ in 0..count {
        arguments.push(pop(stack)?);
    }
    arguments.reverse();
    Ok(arguments)
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn unary_value(operation: u8, value: VmValue) -> Result<VmValue, StepError> {
    let VmValue::Integer(value) = value else {
        return Err(StepError::new(
            VmFaultCode::TypeMismatch,
            "unary operation expects an integer",
        ));
    };
    Ok(VmValue::Integer(match operation {
        0 => value,
        1 => value.wrapping_neg(),
        2 => i64::from(value == 0),
        3 => !value,
        4 | 6 => value.wrapping_add(1),
        5 | 7 => value.wrapping_sub(1),
        _ => {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "unknown unary operation",
            ));
        }
    }))
}

#[allow(clippy::too_many_lines)]
pub(super) fn binary_value(
    operation: u8,
    left: VmValue,
    right: VmValue,
) -> Result<VmValue, StepError> {
    match (left, right) {
        (VmValue::Integer(left), VmValue::Integer(right)) => {
            let value = match operation {
                0 => left.wrapping_mul(right),
                1 => left.checked_div(right).ok_or_else(|| {
                    StepError::script(
                        crate::ScriptFaultKind::Arithmetic,
                        if right == 0 {
                            VmFaultCode::DivideByZero
                        } else {
                            VmFaultCode::InvalidInstruction
                        },
                        "integer division failed",
                    )
                })?,
                2 => left.checked_rem(right).ok_or_else(|| {
                    StepError::script(
                        crate::ScriptFaultKind::Arithmetic,
                        VmFaultCode::DivideByZero,
                        "integer remainder failed",
                    )
                })?,
                3 => left.wrapping_add(right),
                4 => left.wrapping_sub(right),
                5 => left.wrapping_shl(u32::try_from(right & 63).unwrap_or(0)),
                6 => left.wrapping_shr(u32::try_from(right & 63).unwrap_or(0)),
                7 => i64::from(left < right),
                8 => i64::from(left <= right),
                9 => i64::from(left > right),
                10 => i64::from(left >= right),
                11 => i64::from(left == right),
                12 => i64::from(left != right),
                13 => left & right,
                14 => left ^ right,
                15 => left | right,
                16 => i64::from(left != 0 && right != 0),
                17 => i64::from((left != 0) ^ (right != 0)),
                18 => i64::from(left != 0 || right != 0),
                19 => i64::from(!(left != 0 && right != 0)),
                20 => i64::from(!(left != 0 || right != 0)),
                _ => {
                    return Err(StepError::new(
                        VmFaultCode::InvalidInstruction,
                        "unknown binary operation",
                    ));
                }
            };
            Ok(VmValue::Integer(value))
        }
        (VmValue::String(left), VmValue::String(right)) => Ok(match operation {
            3 => VmValue::String(left + &right),
            7 => VmValue::Integer(i64::from(left < right)),
            8 => VmValue::Integer(i64::from(left <= right)),
            9 => VmValue::Integer(i64::from(left > right)),
            10 => VmValue::Integer(i64::from(left >= right)),
            11 => VmValue::Integer(i64::from(left == right)),
            12 => VmValue::Integer(i64::from(left != right)),
            _ => {
                return Err(StepError::new(
                    VmFaultCode::TypeMismatch,
                    "binary operation is not defined for strings",
                ));
            }
        }),
        (VmValue::String(value), VmValue::Integer(count))
        | (VmValue::Integer(count), VmValue::String(value))
            if operation == 0 =>
        {
            if !(0..10_000).contains(&count) {
                return Err(StepError::script(
                    crate::ScriptFaultKind::Argument,
                    VmFaultCode::InvalidInstruction,
                    "string repeat count must be between 0 and 9999",
                ));
            }
            Ok(VmValue::String(
                value.repeat(usize::try_from(count).unwrap_or_default()),
            ))
        }
        _ => Err(StepError::new(
            VmFaultCode::TypeMismatch,
            "binary operands have different types",
        )),
    }
}

pub(super) fn assign_binary_tag(operation: u8) -> Result<u8, StepError> {
    Ok(match operation {
        1 => 3,
        2 => 4,
        3 => 0,
        4 => 1,
        5 => 2,
        6 => 13,
        7 => 15,
        8 => 14,
        9 => 5,
        10 => 6,
        _ => {
            return Err(StepError::new(
                VmFaultCode::InvalidInstruction,
                "unknown assignment operation",
            ));
        }
    })
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_vm_error(error: VmError) -> StepError {
    if let VmError::ScriptFailure(failure) = error {
        return failure;
    }
    let code = match error {
        VmError::InvalidArguments(_) => VmFaultCode::TypeMismatch,
        VmError::ResourceLimit(_) => VmFaultCode::ResourceLimit,
        VmError::MissingFunction(_) => VmFaultCode::MissingSymbol,
        _ => VmFaultCode::Bounds,
    };
    StepError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_strings_preserves_order_and_reuses_the_first_allocation() {
        let mut first = String::with_capacity(32);
        first.push('前');
        let allocation = first.as_ptr();
        let mut stack = vec![
            VmValue::Integer(7),
            VmValue::String(first),
            VmValue::String("缀".to_owned()),
            VmValue::String(String::new()),
        ];

        let Ok(result) = concat_strings(&mut stack, 3) else {
            panic!("validated strings must concatenate");
        };

        assert_eq!(result, "前缀");
        assert_eq!(result.as_ptr(), allocation);
        assert_eq!(stack, [VmValue::Integer(7)]);
    }

    #[test]
    fn concat_strings_preserves_legacy_fault_priority_and_consumption() {
        let mut mismatch = vec![
            VmValue::Integer(1),
            VmValue::Integer(2),
            VmValue::String("top".to_owned()),
        ];
        let error = concat_strings(&mut mismatch, 4).unwrap_err();
        assert_eq!(error.code, VmFaultCode::TypeMismatch);
        assert_eq!(mismatch, [VmValue::Integer(1)]);

        let mut underflow = vec![
            VmValue::String("bottom".to_owned()),
            VmValue::String("top".to_owned()),
        ];
        let error = concat_strings(&mut underflow, 3).unwrap_err();
        assert_eq!(error.code, VmFaultCode::StackUnderflow);
        assert!(underflow.is_empty());

        let mut unchanged = vec![VmValue::Integer(7)];
        let Ok(result) = concat_strings(&mut unchanged, 0) else {
            panic!("zero-part concat must succeed");
        };
        assert_eq!(result, "");
        assert_eq!(unchanged, [VmValue::Integer(7)]);
    }

    #[test]
    fn popped_indices_preserve_source_order_inline_and_on_overflow() {
        for expected in [vec![1_u64, 2, 3], vec![1_u64, 2, 3, 4, 5]] {
            let mut stack = expected
                .iter()
                .copied()
                .map(|value| VmValue::Integer(i64::try_from(value).unwrap_or_default()))
                .collect::<Vec<_>>();
            let Ok(indices) = pop_indices(&mut stack, expected.len()) else {
                panic!("valid integer indices must decode");
            };
            assert_eq!(indices.as_slice(), expected);
            assert!(stack.is_empty());
        }
    }

    #[test]
    fn multiplication_repeats_a_string_in_either_operand_order() {
        assert!(matches!(
            binary_value(0, VmValue::String("x".into()), VmValue::Integer(3)),
            Ok(VmValue::String(value)) if value == "xxx"
        ));
        assert!(matches!(
            binary_value(0, VmValue::Integer(2), VmValue::String("ab".into())),
            Ok(VmValue::String(value)) if value == "abab"
        ));
        assert!(binary_value(0, VmValue::String("x".into()), VmValue::Integer(-1)).is_err());
    }
}
