//! Opaque BIT stack provenance uses the existing CFG, never a second simulator.
use super::{InstructionError, StackValue, expect_payload, invalid, pop_type, read_u32};
use erabasic_bytecode::{
    BitCallSpec, BytecodeFunction, BytecodeGlobal, BytecodeStorage, BytecodeType, Opcode,
    RuntimeExpressionShape, RuntimeStagedAuthorization, RuntimeStagedKind, SymbolKey,
};
use std::collections::BTreeMap;

fn authorize(
    spec: BitCallSpec,
    input: &BytecodeGlobal,
    staged: &BTreeMap<SymbolKey, &RuntimeStagedAuthorization>,
    trusted_staged: &BTreeMap<SymbolKey, RuntimeStagedAuthorization>,
) -> Result<(), InstructionError> {
    let mut shapes = Vec::with_capacity(usize::from(spec.tail_count) + 1);
    shapes.push(Some(RuntimeExpressionShape {
        value_type: input.value_type,
        variable: true,
        mutable: input.mutable,
    }));
    for index in 0..spec.tail_count {
        shapes.push(
            (spec.present & (1 << index) != 0).then_some(RuntimeExpressionShape {
                value_type: BytecodeType::Integer,
                variable: false,
                mutable: false,
            }),
        );
    }
    let name = match spec.operation {
        erabasic_bytecode::BitOperation::Set => "BITSET",
        erabasic_bytecode::BitOperation::Get => "BITGET",
        erabasic_bytecode::BitOperation::Toggle => "BITTOGGLE",
        erabasic_bytecode::BitOperation::IndexOfFirst => "BITINDEXOFFIRST",
    };
    super::super::staged_authorization::require(
        staged,
        trusted_staged,
        name,
        RuntimeStagedKind::Bit(spec.operation),
        &shapes,
    )
}

pub(super) fn spec(
    function: &BytecodeFunction,
    begin: usize,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
    staged: &BTreeMap<SymbolKey, &RuntimeStagedAuthorization>,
    trusted_staged: &BTreeMap<SymbolKey, RuntimeStagedAuthorization>,
) -> Result<BitCallSpec, InstructionError> {
    let opening = function
        .code
        .get(begin)
        .ok_or_else(|| invalid("BIT capture origin is missing"))?;
    if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginBitCall) {
        return Err(invalid("BIT origin is not an opener"));
    }
    let spec = BitCallSpec::decode(&opening.payload).map_err(invalid)?;
    let input = globals
        .get(&spec.input)
        .copied()
        .ok_or_else(|| invalid("BIT input schema is missing"))?;
    if input.value_type != BytecodeType::Integer
        || input.dimensions.len() != 1
        || !input.mutable
        || matches!(
            input.storage,
            BytecodeStorage::Constant | BytecodeStorage::Calculated
        )
        || input.owner.is_some_and(|owner| owner != function.key)
            && input.storage != BytecodeStorage::FunctionPersistent
    {
        return Err(invalid(
            "BIT input must be a mutable Integer array of rank one in the caller scope",
        ));
    }
    authorize(spec, input, staged, trusted_staged)?;
    Ok(spec)
}

pub(super) fn apply(
    function: &BytecodeFunction,
    index: usize,
    opcode: Opcode,
    stack: &mut Vec<StackValue>,
    globals: &BTreeMap<SymbolKey, &BytecodeGlobal>,
    staged: &BTreeMap<SymbolKey, &RuntimeStagedAuthorization>,
    trusted_staged: &BTreeMap<SymbolKey, RuntimeStagedAuthorization>,
) -> Result<(), InstructionError> {
    if opcode == Opcode::BeginBitCall {
        spec(function, index, globals, staged, trusted_staged)?;
        let begin = u32::try_from(index)
            .map_err(|_| invalid("BIT capture offset exceeds the bytecode format"))?;
        stack.push(StackValue::BitCallToken { begin });
    } else {
        expect_payload(&function.code[index].payload, 4)?;
        let begin = read_u32(&function.code[index].payload, 0)?;
        if begin as usize >= index {
            return Err(invalid("BIT finish precedes its capture"));
        }
        let spec = spec(function, begin as usize, globals, staged, trusted_staged)?;
        for _ in 0..spec.evaluated_arguments() {
            pop_type(stack, BytecodeType::Integer)?;
        }
        if stack.pop() != Some(StackValue::BitCallToken { begin }) {
            return Err(invalid("BIT completion does not own its opaque token"));
        }
        stack.push(BytecodeType::Integer.into());
    }
    Ok(())
}
