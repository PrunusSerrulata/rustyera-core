use std::collections::BTreeMap;

use erabasic_bytecode::{
    BytecodeFunction, BytecodeStorage, BytecodeType, ImportKind, Opcode, SymbolKey, opcode,
};

use crate::ValidationCode;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn apply_instruction(
    function: &BytecodeFunction,
    index: usize,
    stack: &mut Vec<BytecodeType>,
    globals: &BTreeMap<SymbolKey, &erabasic_bytecode::BytecodeGlobal>,
    functions: &BTreeMap<SymbolKey, &BytecodeFunction>,
    native: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
    host: &BTreeMap<SymbolKey, &erabasic_bytecode::RuntimeImport>,
) -> Result<Vec<usize>, (ValidationCode, String)> {
    let instruction = &function.code[index];
    let opcode_value = Opcode::try_from(instruction.opcode).map_err(|unknown| {
        (
            ValidationCode::UnknownOpcode,
            format!("unknown opcode {unknown}"),
        )
    })?;
    let next = || {
        (index + 1 < function.code.len())
            .then_some(index + 1)
            .into_iter()
            .collect()
    };
    match opcode_value {
        Opcode::Nop | Opcode::Yield | Opcode::ForBreak | Opcode::SelectEnd => {
            expect_payload(&instruction.payload, 0)?;
        }
        Opcode::PushInteger => {
            expect_payload(&instruction.payload, 8)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::PushString => {
            let length = read_u32(&instruction.payload, 0)? as usize;
            if instruction.payload.len() != 4 + length
                || std::str::from_utf8(&instruction.payload[4..]).is_err()
            {
                return Err((
                    ValidationCode::InvalidOperand,
                    "invalid UTF-8 string operand".into(),
                ));
            }
            stack.push(BytecodeType::String);
        }
        Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace => {
            expect_payload(&instruction.payload, 19)?;
            let key = read_key(&instruction.payload)?;
            let indices = read_u16(&instruction.payload, 16)? as usize;
            let global = globals.get(&key).copied().ok_or((
                ValidationCode::MissingReference,
                "variable operand does not resolve".into(),
            ))?;
            let maximum_indices =
                global.dimensions.len() + usize::from(global.storage == BytecodeStorage::Character);
            if indices > maximum_indices {
                return Err((
                    ValidationCode::InvalidOperand,
                    "variable index count exceeds its schema".into(),
                ));
            }
            if matches!(opcode_value, Opcode::LoadVariable | Opcode::MakePlace)
                && instruction.payload[18] != 0
            {
                return Err((
                    ValidationCode::InvalidOperand,
                    "load instruction has a store operation tag".into(),
                ));
            }
            if opcode_value == Opcode::StoreVariable {
                if !global.mutable {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "store instruction targets an immutable variable".into(),
                    ));
                }
                if instruction.payload[18] > 10 {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "store instruction has an unknown assignment operation".into(),
                    ));
                }
            }
            if opcode_value == Opcode::StoreVariable {
                pop_type(stack, global.value_type)?;
            }
            for _ in 0..indices {
                pop_type(stack, BytecodeType::Integer)?;
            }
            if opcode_value == Opcode::LoadVariable {
                stack.push(global.value_type);
            } else if opcode_value == Opcode::MakePlace {
                stack.push(match global.value_type {
                    BytecodeType::Integer => BytecodeType::IntegerPlace,
                    BytecodeType::String => BytecodeType::StringPlace,
                    BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                        return Err((
                            ValidationCode::InvalidOperand,
                            "a variable schema cannot contain place values".into(),
                        ));
                    }
                });
            }
        }
        Opcode::Unary => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 7 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "unknown unary operation".into(),
                ));
            }
            pop_type(stack, BytecodeType::Integer)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::Binary => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 20 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "unknown binary operation".into(),
                ));
            }
            let right = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "binary operation underflows the stack".into(),
            ))?;
            let left = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "binary operation underflows the stack".into(),
            ))?;
            let string_repeat = instruction.payload[0] == 0
                && matches!(
                    (left, right),
                    (BytecodeType::String, BytecodeType::Integer)
                        | (BytecodeType::Integer, BytecodeType::String)
                );
            if !string_repeat
                && (left != right || !matches!(left, BytecodeType::Integer | BytecodeType::String))
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "binary operands have incompatible types".into(),
                ));
            }
            if !string_repeat
                && left == BytecodeType::String
                && !matches!(instruction.payload[0], 3 | 7..=12)
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "binary operation is not defined for strings".into(),
                ));
            }
            let result =
                if string_repeat || (left == BytecodeType::String && instruction.payload[0] == 3) {
                    BytecodeType::String
                } else {
                    BytecodeType::Integer
                };
            stack.push(result);
        }
        Opcode::ToString => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "string conversion underflows the stack".into(),
            ))?;
            stack.push(BytecodeType::String);
        }
        Opcode::Concat => {
            expect_payload(&instruction.payload, 2)?;
            for _ in 0..read_u16(&instruction.payload, 0)? {
                pop_type(stack, BytecodeType::String)?;
            }
            stack.push(BytecodeType::String);
        }
        Opcode::Pop => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "pop underflows the stack".into(),
            ))?;
        }
        Opcode::Dup => {
            expect_payload(&instruction.payload, 0)?;
            let value = *stack.last().ok_or((
                ValidationCode::StackMismatch,
                "dup underflows the stack".into(),
            ))?;
            stack.push(value);
        }
        Opcode::StorePlace => {
            expect_payload(&instruction.payload, 0)?;
            let place = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "indirect store underflows the stack".into(),
            ))?;
            let value = stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "indirect store underflows the stack".into(),
            ))?;
            if !matches!(
                (place, value),
                (BytecodeType::IntegerPlace, BytecodeType::Integer)
                    | (BytecodeType::StringPlace, BytecodeType::String)
            ) {
                return Err((
                    ValidationCode::TypeMismatch,
                    "indirect store place and value types differ".into(),
                ));
            }
        }
        Opcode::Jump => {
            expect_payload(&instruction.payload, 4)?;
            return Ok(vec![read_u32(&instruction.payload, 0)? as usize]);
        }
        Opcode::JumpIfFalse => {
            expect_payload(&instruction.payload, 4)?;
            pop_type(stack, BytecodeType::Integer)?;
            return Ok(vec![read_u32(&instruction.payload, 0)? as usize, index + 1]);
        }
        Opcode::ForStart => {
            expect_payload(&instruction.payload, 0)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::Integer)?;
            pop_type(stack, BytecodeType::IntegerPlace)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::ForNext => {
            expect_payload(&instruction.payload, 0)?;
            stack.push(BytecodeType::Integer);
        }
        Opcode::SelectStart => {
            expect_payload(&instruction.payload, 0)?;
            stack.pop().ok_or((
                ValidationCode::StackMismatch,
                "SELECTCASE underflows the stack".into(),
            ))?;
        }
        Opcode::SelectCompare => {
            expect_payload(&instruction.payload, 1)?;
            let operands = if instruction.payload[0] == 6 { 2 } else { 1 };
            if instruction.payload[0] > 7 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "SELECTCASE comparison has an unknown operation".into(),
                ));
            }
            for _ in 0..operands {
                stack.pop().ok_or((
                    ValidationCode::StackMismatch,
                    "CASE comparison underflows the stack".into(),
                ))?;
            }
            stack.push(BytecodeType::Integer);
        }
        Opcode::ResolveFunction => {
            expect_payload(&instruction.payload, 6)?;
            pop_type(stack, BytecodeType::String)?;
            stack.push(BytecodeType::String);
            if instruction.payload[4] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "resolve-function allow-missing flag is invalid".into(),
                ));
            }
            if instruction.payload[5] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "resolve-function method flag is invalid".into(),
                ));
            }
            if instruction.payload[4] == 1 {
                return Ok(vec![read_u32(&instruction.payload, 0)? as usize, index + 1]);
            }
        }
        Opcode::InvokeDynamic => {
            expect_payload(&instruction.payload, 3)?;
            if instruction.payload[2] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "dynamic-invoke tail flag is invalid".into(),
                ));
            }
            for _ in 0..read_u16(&instruction.payload, 0)? {
                stack.pop().ok_or((
                    ValidationCode::StackMismatch,
                    "dynamic call argument underflows the stack".into(),
                ))?;
            }
            pop_type(stack, BytecodeType::String)?;
        }
        Opcode::JumpDynamicLabel => {
            expect_payload(&instruction.payload, 4)?;
            pop_type(stack, BytecodeType::String)?;
            let mut successors = vec![read_u32(&instruction.payload, 0)? as usize];
            successors.extend(
                function
                    .labels
                    .iter()
                    .map(|label| label.instruction as usize),
            );
            successors.sort_unstable();
            successors.dedup();
            return Ok(successors);
        }
        Opcode::InvokeEvent => {
            expect_payload(&instruction.payload, 0)?;
            pop_type(stack, BytecodeType::String)?;
        }
        Opcode::Call | Opcode::CallNative | Opcode::CallHost => {
            expect_payload(&instruction.payload, 7)?;
            let import_index = read_u32(&instruction.payload, 0)? as usize;
            let declared_arguments = read_u16(&instruction.payload, 4)? as usize;
            let import = function.imports.get(import_index).ok_or((
                ValidationCode::MissingReference,
                "call import index is out of bounds".into(),
            ))?;
            let (parameters, result) = match (opcode_value, import.kind) {
                (Opcode::Call, ImportKind::Function) => {
                    let target = functions.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "called function does not resolve".into(),
                    ))?;
                    (
                        target
                            .parameters
                            .iter()
                            .map(|parameter| parameter.value_type)
                            .collect(),
                        target.result,
                    )
                }
                (Opcode::CallNative, ImportKind::Native) => {
                    let target = native.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "native import does not resolve".into(),
                    ))?;
                    (target.parameters.clone(), target.result)
                }
                (Opcode::CallHost, ImportKind::Host) => {
                    let target = host.get(&import.key).ok_or((
                        ValidationCode::MissingReference,
                        "host import does not resolve".into(),
                    ))?;
                    (target.parameters.clone(), target.result)
                }
                _ => {
                    return Err((
                        ValidationCode::InvalidOperand,
                        "call opcode does not match its import kind".into(),
                    ));
                }
            };
            if parameters.len() != declared_arguments {
                return Err((
                    ValidationCode::InvalidOperand,
                    "call argument count does not match its import".into(),
                ));
            }
            for parameter in parameters.iter().rev() {
                pop_type(stack, *parameter)?;
            }
            let encoded_result = (instruction.payload[6] != u8::MAX)
                .then(|| opcode::decode_type(instruction.payload[6]))
                .flatten();
            if encoded_result != result {
                return Err((
                    ValidationCode::TypeMismatch,
                    "call result type does not match its import".into(),
                ));
            }
            if let Some(result) = result {
                stack.push(result);
            }
        }
        Opcode::Return => {
            expect_payload(&instruction.payload, 1)?;
            if instruction.payload[0] > 1 {
                return Err((
                    ValidationCode::InvalidOperand,
                    "return flag must be zero or one".into(),
                ));
            }
            if instruction.payload[0] != 0 {
                let result = function
                    .result
                    .or_else(|| {
                        (function.kind != erabasic_bytecode::BytecodeFunctionKind::Method)
                            .then_some(BytecodeType::Integer)
                    })
                    .ok_or((
                        ValidationCode::TypeMismatch,
                        "void function returns a value".into(),
                    ))?;
                pop_type(stack, result)?;
            } else if function.result.is_some()
                && function.kind != erabasic_bytecode::BytecodeFunctionKind::Event
            {
                return Err((
                    ValidationCode::TypeMismatch,
                    "value-returning function has an empty return".into(),
                ));
            }
            if !stack.is_empty() {
                return Err((
                    ValidationCode::StackMismatch,
                    "return leaves temporary values on the stack".into(),
                ));
            }
            return Ok(Vec::new());
        }
        Opcode::AwaitResume => {
            expect_payload(&instruction.payload, 1)?;
            stack.push(
                opcode::decode_type(instruction.payload[0])
                    .ok_or((ValidationCode::InvalidOperand, "invalid resume type".into()))?,
            );
        }
        Opcode::Trap => {
            if std::str::from_utf8(&instruction.payload).is_err() {
                return Err((
                    ValidationCode::InvalidOperand,
                    "trap message is not UTF-8".into(),
                ));
            }
            return Ok(Vec::new());
        }
    }
    Ok(next())
}

fn expect_payload(payload: &[u8], length: usize) -> Result<(), (ValidationCode, String)> {
    if payload.len() == length {
        Ok(())
    } else {
        Err((
            ValidationCode::InvalidOperand,
            format!("expected {length} payload bytes, found {}", payload.len()),
        ))
    }
}

fn read_u16(payload: &[u8], offset: usize) -> Result<u16, (ValidationCode, String)> {
    Ok(u16::from_le_bytes(
        payload
            .get(offset..offset + 2)
            .ok_or((
                ValidationCode::InvalidOperand,
                "truncated u16 operand".into(),
            ))?
            .try_into()
            .expect("two-byte slice"),
    ))
}

fn read_u32(payload: &[u8], offset: usize) -> Result<u32, (ValidationCode, String)> {
    Ok(u32::from_le_bytes(
        payload
            .get(offset..offset + 4)
            .ok_or((
                ValidationCode::InvalidOperand,
                "truncated u32 operand".into(),
            ))?
            .try_into()
            .expect("four-byte slice"),
    ))
}

fn read_key(payload: &[u8]) -> Result<SymbolKey, (ValidationCode, String)> {
    let mut key = [0; 16];
    key.copy_from_slice(payload.get(..16).ok_or((
        ValidationCode::InvalidOperand,
        "truncated symbol key".into(),
    ))?);
    Ok(SymbolKey(key))
}

fn pop_type(
    stack: &mut Vec<BytecodeType>,
    expected: BytecodeType,
) -> Result<(), (ValidationCode, String)> {
    let actual = stack.pop().ok_or((
        ValidationCode::StackMismatch,
        "instruction underflows the stack".into(),
    ))?;
    if actual == expected {
        Ok(())
    } else {
        Err((
            ValidationCode::TypeMismatch,
            format!("expected {expected:?}, found {actual:?}"),
        ))
    }
}
