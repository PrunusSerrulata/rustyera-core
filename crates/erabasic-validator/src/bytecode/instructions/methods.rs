//! Validator state tracks only an opaque lease and its next syntactic slot.
//! Captures live in bounded VM pending state, never in this operand stack.
use std::collections::BTreeMap;

use erabasic_bytecode::{
    BytecodeFunction, BytecodeGlobal, BytecodeStorage, BytecodeType, CallTextSpec, Opcode,
    SymbolKey, UserArgumentAdvance, UserArgumentSpec, UserCallSpec,
};

use super::{StackValue, expect_payload, pop_type, read_u16, read_u32};
use crate::ValidationCode;

type InstructionError = (ValidationCode, String);

pub(super) fn apply(
    function: &BytecodeFunction,
    index: usize,
    operation: Opcode,
    stack: &mut Vec<StackValue>,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<Vec<usize>, InstructionError> {
    let payload = &function.code[index].payload;
    match operation {
        Opcode::ResolveUserCall => {
            let resolve =
                u32::try_from(index).map_err(|_| invalid("user-call origin exceeds u32"))?;
            let spec = decode_spec(function, resolve, payload, globals)?;
            pop_type(stack, BytecodeType::String)?;
            stack.push(StackValue::UserCallToken {
                resolve,
                next_slot: 0,
            });
            if spec.allow_missing {
                Ok(vec![spec.missing_target as usize, index + 1])
            } else {
                Ok(vec![index + 1])
            }
        }
        Opcode::GuardUserArgument | Opcode::SelectUserArgument => {
            expect_payload(payload, 10)?;
            let resolve = read_u32(payload, 0)?;
            let slot = read_u16(payload, 4)?;
            let spec = referenced_spec(function, index, resolve, globals)?;
            let argument = spec.arguments.get(usize::from(slot));
            let valid = if operation == Opcode::SelectUserArgument {
                matches!(argument, Some(UserArgumentSpec::Variable(_)))
            } else {
                matches!(
                    argument,
                    Some(UserArgumentSpec::Variable(_) | UserArgumentSpec::Value(_))
                )
            };
            if !valid {
                return Err(invalid(
                    "user-call guard/selection has an incompatible slot",
                ));
            }
            expect_token(stack, resolve, slot)?;
            Ok(vec![read_u32(payload, 6)? as usize, index + 1])
        }
        Opcode::CaptureUserArgument => {
            capture_argument(function, index, payload, stack, globals)?;
            Ok(vec![index + 1])
        }
        Opcode::AdvanceUserArgument => {
            advance_argument(function, index, payload, stack, globals)?;
            Ok(vec![index + 1])
        }
        Opcode::InvokeUserCall => {
            expect_payload(payload, 4)?;
            let resolve = read_u32(payload, 0)?;
            let spec = referenced_spec(function, index, resolve, globals)?;
            let count = u16::try_from(spec.arguments.len()).expect("decoded count is u16");
            expect_token(stack, resolve, count)?;
            stack.pop();
            if let Some(result) = spec.mode.expected_result() {
                stack.push(StackValue::Value(result));
            }
            if spec.mode.unwinds_caller() {
                if !stack.is_empty() {
                    return Err((
                        ValidationCode::StackMismatch,
                        "jump user-call leaves an active operand or pending call".into(),
                    ));
                }
                // The caller frame is kept by the VM until successful callee return.
                Ok(Vec::new())
            } else {
                Ok(vec![index + 1])
            }
        }
        Opcode::AbandonUserCall => {
            expect_payload(payload, 4)?;
            let resolve = read_u32(payload, 0)?;
            let spec = referenced_spec(function, index, resolve, globals)?;
            if !spec.allow_missing || spec.missing_target as usize != index {
                return Err(invalid(
                    "user-call abandon is not its declared missing branch",
                ));
            }
            expect_token(stack, resolve, 0)?;
            stack.pop();
            Ok(vec![index + 1])
        }
        Opcode::InvokeCallText => {
            let spec = CallTextSpec::decode(payload).map_err(operand_error)?;
            pop_type(stack, BytecodeType::String)?;
            // CALLSTR is a statement. A jump's blank source still falls through.
            if !stack.is_empty() {
                return Err((
                    ValidationCode::StackMismatch,
                    "call-text statement leaves an active operand or pending call".into(),
                ));
            }
            if spec.mode.has_catch() {
                Ok(vec![spec.catch_target as usize, index + 1])
            } else {
                Ok(vec![index + 1])
            }
        }
        _ => unreachable!("only user-call opcodes are dispatched here"),
    }
}

fn advance_argument(
    function: &BytecodeFunction,
    index: usize,
    payload: &[u8],
    stack: &mut [StackValue],
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<(), InstructionError> {
    expect_payload(payload, 7)?;
    let resolve = read_u32(payload, 0)?;
    let slot = read_u16(payload, 4)?;
    let spec = referenced_spec(function, index, resolve, globals)?;
    let reason = UserArgumentAdvance::decode(payload[6]).map_err(operand_error)?;
    let argument = spec.arguments.get(usize::from(slot));
    let valid = match reason {
        UserArgumentAdvance::Omitted => matches!(argument, Some(UserArgumentSpec::Omitted)),
        UserArgumentAdvance::Discarded => matches!(
            argument,
            Some(UserArgumentSpec::Value(_) | UserArgumentSpec::Variable(_))
        ),
    };
    if !valid {
        return Err(invalid("user-call advance reason does not match its slot"));
    }
    advance_token(stack, resolve, slot)?;
    // The VM additionally verifies default/retained-prefix membership.
    Ok(())
}

fn capture_argument(
    function: &BytecodeFunction,
    index: usize,
    payload: &[u8],
    stack: &mut Vec<StackValue>,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<(), InstructionError> {
    expect_payload(payload, 7)?;
    let resolve = read_u32(payload, 0)?;
    let slot = read_u16(payload, 4)?;
    let spec = referenced_spec(function, index, resolve, globals)?;
    let reference = match payload[6] {
        0 => false,
        1 => true,
        _ => return Err(invalid("user-call capture reference flag is invalid")),
    };
    let value_type = match spec.arguments.get(usize::from(slot)) {
        Some(UserArgumentSpec::Value(value_type)) if !reference => *value_type,
        Some(UserArgumentSpec::Variable(key)) => match (globals[key].value_type, reference) {
            (BytecodeType::Integer, true) => BytecodeType::IntegerPlace,
            (BytecodeType::String, true) => BytecodeType::StringPlace,
            (scalar, false) => scalar,
            _ => return Err(invalid("user-call variable has a non-scalar schema")),
        },
        _ => return Err(invalid("user-call capture does not match its slot")),
    };
    pop_type(stack, value_type)?;
    // Value and REF captures and the discard branch advance identically at joins.
    // VM validates REF backing identity and whether this actual was retained.
    advance_token(stack, resolve, slot)
}

fn decode_spec(
    function: &BytecodeFunction,
    resolve: u32,
    payload: &[u8],
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<UserCallSpec, InstructionError> {
    let spec = UserCallSpec::decode(payload).map_err(operand_error)?;
    if spec.allow_missing
        && !function
            .code
            .get(spec.missing_target as usize)
            .is_some_and(|instruction| {
                instruction.opcode == Opcode::AbandonUserCall as u16
                    && &instruction.payload[..] == resolve.to_le_bytes().as_slice()
            })
    {
        return Err((
            ValidationCode::InvalidControlFlow,
            "user-call missing branch must abandon its own resolve token".into(),
        ));
    }
    for argument in &spec.arguments {
        if let UserArgumentSpec::Variable(key) = argument {
            let variable = globals.get(key).ok_or((
                ValidationCode::MissingReference,
                "dynamic user-call variable does not resolve".into(),
            ))?;
            if !matches!(
                variable.value_type,
                BytecodeType::Integer | BytecodeType::String
            ) {
                return Err(invalid("user-call variable has a non-scalar schema"));
            }
            if variable.storage == BytecodeStorage::FunctionLocal
                && variable.owner != Some(function.key)
            {
                return Err((
                    ValidationCode::MissingReference,
                    "user-call argument references another function's local variable".into(),
                ));
            }
        }
    }
    Ok(spec)
}

fn referenced_spec(
    function: &BytecodeFunction,
    index: usize,
    resolve: u32,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<UserCallSpec, InstructionError> {
    if resolve as usize >= index {
        return Err(invalid(
            "user-call resolve origin must precede its consumer",
        ));
    }
    let instruction = function
        .code
        .get(resolve as usize)
        .ok_or_else(|| invalid("user-call resolve origin is outside the function"))?;
    if instruction.opcode != Opcode::ResolveUserCall as u16 {
        return Err(invalid("user-call resolve origin is not ResolveUserCall"));
    }
    decode_spec(function, resolve, &instruction.payload, globals)
}

fn expect_token(
    stack: &[StackValue],
    resolve: u32,
    next_slot: u16,
) -> Result<(), InstructionError> {
    if stack.last() != Some(&StackValue::UserCallToken { resolve, next_slot }) {
        return Err((
            ValidationCode::StackMismatch,
            "user-call token origin or next syntactic slot does not match".into(),
        ));
    }
    Ok(())
}

fn advance_token(
    stack: &mut [StackValue],
    resolve: u32,
    slot: u16,
) -> Result<(), InstructionError> {
    expect_token(stack, resolve, slot)?;
    let next_slot = slot
        .checked_add(1)
        .ok_or_else(|| invalid("user-call slot exceeds u16"))?;
    *stack.last_mut().expect("token was checked") =
        StackValue::UserCallToken { resolve, next_slot };
    Ok(())
}

fn operand_error(message: String) -> InstructionError {
    (ValidationCode::InvalidOperand, message)
}

fn invalid(message: &str) -> InstructionError {
    operand_error(message.into())
}
