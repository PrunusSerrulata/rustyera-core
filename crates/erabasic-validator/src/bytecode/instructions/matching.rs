//! MATCH phases retain an opaque token below ordinary, potentially waiting, expressions.
use super::{InstructionError, StackValue, expect_payload, invalid, pop_type, read_u32};
use erabasic_bytecode::{
    BytecodeFunction, BytecodeType, MatchCallSpec, MatchInput, Opcode, RuntimeExpressionShape,
    RuntimeStagedKind,
};

fn validate_spec(
    function: &BytecodeFunction,
    index: usize,
    opcode: Opcode,
    begin: u32,
    context: &super::Context<'_>,
) -> Result<MatchCallSpec, InstructionError> {
    let begin_index = usize::try_from(begin)
        .map_err(|_| invalid("MATCH capture offset exceeds the host index range"))?;
    let opening = function
        .code
        .get(begin_index)
        .ok_or_else(|| invalid("MATCH opener missing"))?;
    if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginMatchCall)
        || (opcode != Opcode::BeginMatchCall && begin_index >= index)
    {
        return Err(invalid("MATCH phase does not follow its opener"));
    }
    let spec = MatchCallSpec::decode(&opening.payload).map_err(invalid)?;
    let (name, kind, input_shape) = match &spec.input {
        MatchInput::Variable(key) => {
            let definition = context
                .globals
                .get(key)
                .ok_or_else(|| invalid("MATCH input token metadata missing"))?;
            (
                "MATCHALL",
                RuntimeStagedKind::MatchAll,
                RuntimeExpressionShape {
                    value_type: definition.value_type,
                    variable: true,
                    mutable: definition.mutable,
                },
            )
        }
        MatchInput::Name(_) => (
            "MATCHALLEX",
            RuntimeStagedKind::MatchAllEx,
            RuntimeExpressionShape {
                value_type: BytecodeType::String,
                variable: false,
                mutable: false,
            },
        ),
    };
    let scalar = |value_type| {
        Some(RuntimeExpressionShape {
            value_type,
            variable: false,
            mutable: false,
        })
    };
    let output = match spec.output.map(|key| context.globals.get(&key).copied()) {
        Some(Some(definition)) => Some(RuntimeExpressionShape {
            value_type: definition.value_type,
            variable: true,
            mutable: definition.mutable,
        }),
        Some(None) => return Err(invalid("MATCH output token metadata missing")),
        None => None,
    };
    super::super::staged_authorization::require(
        &context.staged,
        context.trusted_staged,
        name,
        kind,
        &[
            Some(input_shape),
            scalar(spec.needle),
            scalar(spec.begin_type),
            spec.end_type.and_then(scalar),
            output,
        ],
    )?;
    for key in spec.output.iter().copied().chain(match &spec.input {
        MatchInput::Variable(key) => Some(*key),
        MatchInput::Name(_) => None,
    }) {
        let definition = context
            .globals
            .get(&key)
            .ok_or_else(|| invalid("MATCH variable token metadata missing"))?;
        if definition.owner.is_some_and(|owner| {
            owner != function.key
                && !(definition.storage == erabasic_bytecode::BytecodeStorage::FunctionPersistent
                    && context
                        .functions
                        .get(&owner)
                        .is_some_and(|target| target.name.eq_ignore_ascii_case(&function.name)))
        }) {
            return Err(invalid("MATCH token belongs to another caller"));
        }
    }
    Ok(spec)
}

pub(super) fn apply(
    function: &BytecodeFunction,
    index: usize,
    opcode: Opcode,
    stack: &mut Vec<StackValue>,
    context: &super::Context<'_>,
) -> Result<Vec<usize>, InstructionError> {
    let payload = &function.code[index].payload;
    let begin = if opcode == Opcode::BeginMatchCall {
        u32::try_from(index)
            .map_err(|_| invalid("MATCH capture offset exceeds the bytecode format"))?
    } else {
        expect_payload(
            payload,
            if opcode == Opcode::MatchCallRange {
                5
            } else {
                4
            },
        )?;
        read_u32(payload, 0)?
    };
    let spec = validate_spec(function, index, opcode, begin, context)?;
    // Output rank/type/mutability are deliberately NOT checked here: the source
    // only checks them after its first match and even then capacity can skip a write.
    match opcode {
        Opcode::BeginMatchCall => stack.push(StackValue::MatchCallToken { begin, phase: 0 }),
        Opcode::MatchCallRange => {
            let phase = payload[4];
            let kind = match phase {
                0 => Some(spec.begin_type),
                1 => spec.end_type,
                _ => return Err(invalid("MATCH range phase invalid")),
            };
            if let Some(kind) = kind {
                pop_type(stack, kind)?;
            }
            if stack.pop() != Some(StackValue::MatchCallToken { begin, phase }) {
                return Err(invalid("MATCH range does not own the current opaque token"));
            }
            stack.push(StackValue::MatchCallToken {
                begin,
                phase: phase + 1,
            });
        }
        Opcode::FinishMatchCall => {
            pop_type(stack, spec.needle)?;
            if stack.pop() != Some(StackValue::MatchCallToken { begin, phase: 2 }) {
                return Err(invalid("MATCH finish does not own a completed range token"));
            }
            stack.push(BytecodeType::Integer.into());
        }
        _ => unreachable!("MATCH opcodes only"),
    }
    Ok(vec![index + 1])
}
