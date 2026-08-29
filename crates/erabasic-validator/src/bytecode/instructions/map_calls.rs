use super::{StackValue, expect_payload, pop_type, read_u32};
use crate::ValidationCode;
use erabasic_bytecode::{
    BytecodeFunction, BytecodeType, ImportKind, MapCallKind, Opcode, RuntimeImport, SymbolKey,
};
use std::collections::BTreeMap;
type Error = (ValidationCode, String);
fn invalid(message: &str) -> Error {
    (ValidationCode::InvalidOperand, message.into())
}

pub(super) fn signature<'a>(
    function: &BytecodeFunction,
    begin: usize,
    native: &'a BTreeMap<SymbolKey, &RuntimeImport>,
) -> Result<(MapCallKind, &'a RuntimeImport), Error> {
    let opening = function
        .code
        .get(begin)
        .ok_or_else(|| invalid("MAP opener is missing"))?;
    if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginMapCall) {
        return Err(invalid("MAP origin is not its opener"));
    }
    expect_payload(&opening.payload, 4)?;
    let import = function
        .imports
        .get(read_u32(&opening.payload, 0)? as usize)
        .filter(|import| import.kind == ImportKind::Native)
        .ok_or_else(|| invalid("MAP import is not Native"))?;
    let target = *native
        .get(&import.key)
        .ok_or_else(|| invalid("MAP native import is missing"))?;
    let kind = MapCallKind::from_name(&target.name)
        .ok_or_else(|| invalid("MAP import is not a staged operation"))?;
    if target.namespace != "rustyera.vm"
        || !kind.valid_parameters(&target.parameters)
        || target.result != Some(kind.result_type())
    {
        return Err(invalid("MAP staged signature differs"));
    }
    Ok((kind, target))
}
pub(super) fn apply(
    function: &BytecodeFunction,
    index: usize,
    opcode: Opcode,
    stack: &mut Vec<StackValue>,
    native: &BTreeMap<SymbolKey, &RuntimeImport>,
) -> Result<(), Error> {
    expect_payload(&function.code[index].payload, 4)?;
    let begin = if opcode == Opcode::BeginMapCall {
        index
    } else {
        read_u32(&function.code[index].payload, 0)? as usize
    };
    let begin_token = u32::try_from(begin)
        .map_err(|_| invalid("MAP capture offset exceeds the bytecode format"))?;
    if opcode != Opcode::BeginMapCall && begin >= index {
        return Err(invalid("MAP completion precedes capture"));
    }
    let (kind, target) = signature(function, begin, native)?;
    if opcode == Opcode::BeginMapCall {
        pop_type(stack, BytecodeType::String)?;
        stack.push(StackValue::MapCallToken { begin: begin_token });
        stack.push(BytecodeType::Integer.into());
    } else {
        if opcode == Opcode::FinishMapCall {
            for parameter in kind
                .materialized_parameters(&target.parameters)
                .into_iter()
                .rev()
            {
                pop_type(stack, parameter)?;
            }
        }
        if stack.pop() != Some(StackValue::MapCallToken { begin: begin_token }) {
            return Err(invalid("MAP completion does not own its opaque token"));
        }
        stack.push(kind.result_type().into());
    }
    Ok(())
}
