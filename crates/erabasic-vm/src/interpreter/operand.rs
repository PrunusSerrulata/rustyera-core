//! Bytecode operand decoding and stack arithmetic helpers.

use super::{StepError, SymbolKey, VmError, VmFaultCode, VmValue};

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

pub(super) fn read_key(payload: &[u8]) -> Result<SymbolKey, StepError> {
    Ok(SymbolKey(exact_slice(payload, 0)?))
}

pub(super) fn pop(stack: &mut Vec<VmValue>) -> Result<VmValue, StepError> {
    stack
        .pop()
        .ok_or_else(|| StepError::new(VmFaultCode::StackUnderflow, "operand stack underflow"))
}

pub(super) fn pop_indices(stack: &mut Vec<VmValue>, count: usize) -> Result<Vec<u64>, StepError> {
    let mut indices = Vec::with_capacity(count);
    for _ in 0..count {
        let VmValue::Integer(value) = pop(stack)? else {
            return Err(StepError::new(
                VmFaultCode::TypeMismatch,
                "variable indices must be integers",
            ));
        };
        indices.push(u64::try_from(value).map_err(|_| {
            StepError::new(VmFaultCode::Bounds, "variable index cannot be negative")
        })?);
    }
    indices.reverse();
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
                    StepError::new(
                        if right == 0 {
                            VmFaultCode::DivideByZero
                        } else {
                            VmFaultCode::InvalidInstruction
                        },
                        "integer division failed",
                    )
                })?,
                2 => left.checked_rem(right).ok_or_else(|| {
                    StepError::new(VmFaultCode::DivideByZero, "integer remainder failed")
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
                return Err(StepError::new(
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
