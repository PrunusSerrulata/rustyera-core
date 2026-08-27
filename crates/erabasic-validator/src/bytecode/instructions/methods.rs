use std::collections::BTreeMap;

use erabasic_bytecode::{
    BytecodeFunction, BytecodeGlobal, BytecodeStorage, BytecodeType, MethodArgumentSpec,
    MethodCallSpec, Opcode, SymbolKey,
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
        Opcode::ResolveMethod => {
            let spec = decode_spec(function, payload, globals)?;
            pop_type(stack, BytecodeType::String)?;
            let resolve = u32::try_from(index).map_err(|_| {
                (
                    ValidationCode::InvalidOperand,
                    "method origin exceeds u32".into(),
                )
            })?;
            stack.push(StackValue::MethodToken { resolve });
            if spec.allow_missing {
                Ok(vec![spec.missing_target as usize, index + 1])
            } else {
                Ok(vec![index + 1])
            }
        }
        Opcode::SelectMethodArgument => {
            expect_payload(payload, 10)?;
            let resolve = read_u32(payload, 0)?;
            let slot = read_u16(payload, 4)?;
            let spec = referenced_spec(function, index, resolve, globals)?;
            if !matches!(
                spec.arguments.get(usize::from(slot)),
                Some(MethodArgumentSpec::Variable(_))
            ) {
                return Err((
                    ValidationCode::InvalidOperand,
                    "method argument selection requires a variable slot".into(),
                ));
            }
            expect_capture_prefix(stack, resolve, &spec, usize::from(slot))?;
            Ok(vec![read_u32(payload, 6)? as usize, index + 1])
        }
        Opcode::CaptureMethodArgument => {
            capture_argument(function, index, payload, stack, globals)?;
            Ok(vec![index + 1])
        }
        Opcode::InvokeMethod => {
            expect_payload(payload, 4)?;
            let resolve = read_u32(payload, 0)?;
            let spec = referenced_spec(function, index, resolve, globals)?;
            let base = expect_capture_prefix(stack, resolve, &spec, spec.arguments.len())?;
            stack.truncate(base);
            stack.push(StackValue::Value(spec.result.bytecode_type()));
            Ok(vec![index + 1])
        }
        _ => unreachable!("only method opcodes are dispatched here"),
    }
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
        _ => {
            return Err((
                ValidationCode::InvalidOperand,
                "method capture reference flag is invalid".into(),
            ));
        }
    };
    let value_type = match spec.arguments.get(usize::from(slot)) {
        Some(MethodArgumentSpec::Value(value_type)) if !reference => *value_type,
        Some(MethodArgumentSpec::Variable(key)) => {
            let scalar = globals[key].value_type;
            match (scalar, reference) {
                (BytecodeType::Integer, true) => BytecodeType::IntegerPlace,
                (BytecodeType::String, true) => BytecodeType::StringPlace,
                (_, false) => scalar,
                _ => {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "method variable has a non-scalar schema".into(),
                    ));
                }
            }
        }
        _ => {
            return Err((
                ValidationCode::InvalidOperand,
                "method capture does not match an actual argument slot".into(),
            ));
        }
    };
    pop_type(stack, value_type)?;
    expect_capture_prefix(stack, resolve, &spec, usize::from(slot))?;
    // Both value and REF branches converge on the same opaque slot.
    // The VM additionally checks the resolved formal mode and the
    // place's binding identity against the declared variable symbol.
    stack.push(StackValue::MethodArgument { resolve, slot });
    Ok(())
}

fn decode_spec(
    function: &BytecodeFunction,
    payload: &[u8],
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
) -> Result<MethodCallSpec, InstructionError> {
    let spec = MethodCallSpec::decode(payload)
        .map_err(|message| (ValidationCode::InvalidOperand, message))?;
    if spec.allow_missing
        && !function
            .code
            .get(spec.missing_target as usize)
            .is_some_and(|instruction| {
                instruction.opcode == Opcode::Pop as u16 && instruction.payload.is_empty()
            })
    {
        return Err((
            ValidationCode::InvalidControlFlow,
            "method missing branch must begin with an operand-free Pop".into(),
        ));
    }
    for argument in &spec.arguments {
        if let MethodArgumentSpec::Variable(key) = argument {
            let variable = globals.get(key).ok_or((
                ValidationCode::MissingReference,
                "dynamic method variable does not resolve".into(),
            ))?;
            if !matches!(
                variable.value_type,
                BytecodeType::Integer | BytecodeType::String
            ) {
                return Err((
                    ValidationCode::InvalidOperand,
                    "method variable has a non-scalar schema".into(),
                ));
            }
            // Function statics retain their existing access rules. Only frame-local
            // storage requires this caller's owner identity before evaluating actuals.
            if variable.storage == BytecodeStorage::FunctionLocal
                && variable.owner != Some(function.key)
            {
                return Err((
                    ValidationCode::MissingReference,
                    "method argument references another function's local variable".into(),
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
) -> Result<MethodCallSpec, InstructionError> {
    if resolve as usize >= index {
        return Err((
            ValidationCode::InvalidOperand,
            "method resolve origin must precede its consumer".into(),
        ));
    }
    let instruction = function.code.get(resolve as usize).ok_or((
        ValidationCode::InvalidOperand,
        "method resolve origin is outside the function".into(),
    ))?;
    if instruction.opcode != Opcode::ResolveMethod as u16 {
        return Err((
            ValidationCode::InvalidOperand,
            "method resolve origin is not ResolveMethod".into(),
        ));
    }
    decode_spec(function, &instruction.payload, globals)
}

/// Returns the base of this call's opaque suffix. A nested call may have its
/// caller's captures below it, but cannot consume, duplicate, or reorder them.
fn expect_capture_prefix(
    stack: &[StackValue],
    resolve: u32,
    spec: &MethodCallSpec,
    before_slot: usize,
) -> Result<usize, InstructionError> {
    let captured = spec.arguments[..before_slot]
        .iter()
        .filter(|argument| !matches!(argument, MethodArgumentSpec::Omitted))
        .count();
    let base = stack.len().checked_sub(captured + 1).ok_or((
        ValidationCode::StackMismatch,
        "method argument sequence underflows its resolve token".into(),
    ))?;
    if stack[base] != (StackValue::MethodToken { resolve }) {
        return Err((
            ValidationCode::StackMismatch,
            "method resolve token origin does not match".into(),
        ));
    }
    let expected = spec.arguments[..before_slot]
        .iter()
        .enumerate()
        .filter(|(_, argument)| !matches!(argument, MethodArgumentSpec::Omitted));
    for (actual, (slot, _)) in stack[base + 1..].iter().zip(expected) {
        let slot = u16::try_from(slot).map_err(|_| {
            (
                ValidationCode::InvalidOperand,
                "method argument slot exceeds u16".into(),
            )
        })?;
        if *actual != (StackValue::MethodArgument { resolve, slot }) {
            return Err((
                ValidationCode::StackMismatch,
                "method arguments are missing, reordered, or from another resolve".into(),
            ));
        }
    }
    Ok(base)
}
