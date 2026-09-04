//! A probe catch landing has exactly one exceptional predecessor.
use super::{StackValue, expect_payload, pop_type, read_u32};
use crate::ValidationCode;
use erabasic_bytecode::{BytecodeFunction, BytecodeType, Opcode};
use std::collections::BTreeMap;
type Error = (ValidationCode, String);
fn invalid(message: &str) -> Error {
    (ValidationCode::InvalidOperand, message.into())
}

/// Constructed once per function, before the existing CFG stack traversal.
/// None records duplicate success landings without retaining an unbounded list.
#[derive(Default)]
pub(in crate::bytecode) struct ProbeIndex {
    successes: BTreeMap<u32, Option<usize>>,
    #[cfg(test)]
    scanned: usize,
}

impl ProbeIndex {
    pub(in crate::bytecode) fn new(function: &BytecodeFunction) -> Result<Self, (usize, Error)> {
        let mut result = Self::default();
        for index in 0..function.code.len() {
            #[cfg(test)]
            {
                result.scanned += 1;
            }
            if let Some((begin, false)) =
                finish_origin(function, index).map_err(|error| (index, error))?
            {
                result
                    .successes
                    .entry(begin)
                    .and_modify(|success| *success = None)
                    .or_insert(Some(index));
            }
        }
        Ok(result)
    }
}

pub(super) fn apply(
    function: &BytecodeFunction,
    index: usize,
    opcode: Opcode,
    stack: &mut Vec<StackValue>,
    probes: &ProbeIndex,
) -> Result<Vec<usize>, Error> {
    let payload = &function.code[index].payload;
    match opcode {
        Opcode::ProbeVariableName => {
            expect_payload(payload, 0)?;
            pop_type(stack, BytecodeType::String)?;
            stack.push(BytecodeType::Integer.into());
        }
        Opcode::BeginExistVarProbe => {
            expect_payload(payload, 4)?;
            let begin = u32::try_from(index).map_err(|_| invalid("probe origin exceeds u32"))?;
            let failure = read_u32(payload, 0)? as usize;
            if failure <= index || finish_origin(function, failure)? != Some((begin, true)) {
                return Err(invalid(
                    "EXISTVAR catch does not point to its own failure landing",
                ));
            }
            let success = probes.successes.get(&begin).copied().flatten();
            if success.is_none_or(|success| !(index < success && success < failure)) {
                return Err(invalid(
                    "EXISTVAR probe needs exactly one success completion before its failure landing",
                ));
            }
            stack.push(StackValue::ExistVarProbeToken { begin });
            return Ok(vec![index + 1, failure]);
        }
        Opcode::FinishExistVarProbe => {
            let Some((begin, caught)) = finish_origin(function, index)? else {
                unreachable!()
            };
            let opening = function
                .code
                .get(begin as usize)
                .ok_or_else(|| invalid("missing probe opener"))?;
            if Opcode::try_from(opening.opcode) != Ok(Opcode::BeginExistVarProbe) {
                return Err(invalid(
                    "probe completion does not reference a probe opener",
                ));
            }
            expect_payload(&opening.payload, 4)?;
            if begin as usize >= index
                || (caught && read_u32(&opening.payload, 0)? as usize != index)
            {
                return Err(invalid("probe completion/failure position differs"));
            }
            if !caught {
                pop_type(stack, BytecodeType::String)?;
            }
            if stack.pop() != Some(StackValue::ExistVarProbeToken { begin }) {
                return Err(invalid(
                    "probe completion does not own the active opaque token",
                ));
            }
            stack.push(BytecodeType::Integer.into());
        }
        _ => unreachable!("probe opcodes only"),
    }
    Ok(vec![index + 1])
}

pub(super) fn validate_edge(
    function: &BytecodeFunction,
    from: usize,
    to: usize,
) -> Result<(), Error> {
    if let Some((begin, true)) = finish_origin(function, to)?
        && (from != begin as usize
            || Opcode::try_from(function.code[from].opcode) != Ok(Opcode::BeginExistVarProbe)
            || read_u32(&function.code[from].payload, 0)? as usize != to)
    {
        return Err(invalid(
            "ordinary control flow cannot enter an EXISTVAR catch landing",
        ));
    }
    Ok(())
}

fn finish_origin(function: &BytecodeFunction, index: usize) -> Result<Option<(u32, bool)>, Error> {
    let Some(instruction) = function.code.get(index) else {
        return Ok(None);
    };
    if Opcode::try_from(instruction.opcode) != Ok(Opcode::FinishExistVarProbe) {
        return Ok(None);
    }
    expect_payload(&instruction.payload, 5)?;
    if instruction.payload[4] > 1 {
        return Err(invalid("unknown probe completion phase"));
    }
    Ok(Some((
        read_u32(&instruction.payload, 0)?,
        instruction.payload[4] == 1,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_bytecode::{BytecodeFunctionKind, EncodedInstruction, SymbolKey, opcode};

    fn finish(begin: u32, caught: bool) -> EncodedInstruction {
        let mut payload = begin.to_le_bytes().to_vec();
        payload.push(u8::from(caught));
        EncodedInstruction::new(Opcode::FinishExistVarProbe, payload)
    }

    fn function(code: Vec<EncodedInstruction>) -> BytecodeFunction {
        BytecodeFunction {
            key: SymbolKey::derive("test.probe", b"index"),
            name: "PROBES".into(),
            kind: BytecodeFunctionKind::Normal,
            parameters: Vec::new(),
            result: None,
            labels: Vec::new(),
            imports: Vec::new(),
            code,
            max_stack: 8,
        }
    }

    #[test]
    fn sequential_probes_build_one_index_and_reuse_it_for_both_paths() {
        let mut code = Vec::new();
        for _ in 0..1024 {
            let begin = u32::try_from(code.len()).unwrap();
            code.extend([
                opcode::jump(Opcode::BeginExistVarProbe, begin + 3),
                opcode::push_string("FLAG"),
                finish(begin, false),
                finish(begin, true),
            ]);
        }
        let function = function(code);
        let probes = ProbeIndex::new(&function).ok().unwrap();
        assert_eq!(probes.scanned, function.code.len());
        assert_eq!(probes.successes.len(), 1024);
        for begin in (0..function.code.len()).step_by(4) {
            let mut stack = Vec::new();
            assert_eq!(
                apply(
                    &function,
                    begin,
                    Opcode::BeginExistVarProbe,
                    &mut stack,
                    &probes
                )
                .unwrap(),
                vec![begin + 1, begin + 3]
            );
            let mut caught = stack.clone();
            stack.push(BytecodeType::String.into());
            apply(
                &function,
                begin + 2,
                Opcode::FinishExistVarProbe,
                &mut stack,
                &probes,
            )
            .unwrap();
            apply(
                &function,
                begin + 3,
                Opcode::FinishExistVarProbe,
                &mut caught,
                &probes,
            )
            .unwrap();
            assert_eq!(stack, caught);
            assert_eq!(stack, vec![StackValue::Value(BytecodeType::Integer)]);
        }
        assert_eq!(probes.scanned, function.code.len());
    }

    #[test]
    fn nested_probe_completion_preserves_the_outer_token() {
        let function = function(vec![
            opcode::jump(Opcode::BeginExistVarProbe, 6),
            opcode::jump(Opcode::BeginExistVarProbe, 4),
            opcode::push_string("FLAG"),
            finish(1, false),
            finish(1, true),
            finish(0, false),
            finish(0, true),
        ]);
        let probes = ProbeIndex::new(&function).ok().unwrap();
        let mut stack = Vec::new();
        apply(
            &function,
            0,
            Opcode::BeginExistVarProbe,
            &mut stack,
            &probes,
        )
        .unwrap();
        apply(
            &function,
            1,
            Opcode::BeginExistVarProbe,
            &mut stack,
            &probes,
        )
        .unwrap();
        stack.push(BytecodeType::String.into());
        apply(
            &function,
            3,
            Opcode::FinishExistVarProbe,
            &mut stack,
            &probes,
        )
        .unwrap();
        assert_eq!(
            stack,
            vec![
                StackValue::ExistVarProbeToken { begin: 0 },
                BytecodeType::Integer.into()
            ]
        );
        stack.pop();
        stack.push(BytecodeType::String.into());
        apply(
            &function,
            5,
            Opcode::FinishExistVarProbe,
            &mut stack,
            &probes,
        )
        .unwrap();
        assert_eq!(stack, vec![BytecodeType::Integer.into()]);
    }

    #[test]
    fn probe_index_keeps_origin_payload_edge_and_token_rejections() {
        let baseline = vec![
            opcode::jump(Opcode::BeginExistVarProbe, 4),
            opcode::push_string("FLAG"),
            finish(0, false),
            EncodedInstruction::new(Opcode::Nop, Vec::new()),
            finish(0, true),
        ];
        let mut duplicate = baseline.clone();
        duplicate[3] = finish(0, false);
        let duplicate = function(duplicate);
        let probes = ProbeIndex::new(&duplicate).ok().unwrap();
        assert!(
            apply(
                &duplicate,
                0,
                Opcode::BeginExistVarProbe,
                &mut Vec::new(),
                &probes
            )
            .is_err()
        );
        let baseline = function(baseline);
        let probes = ProbeIndex::new(&baseline).ok().unwrap();
        assert!(validate_edge(&baseline, 0, 4).is_ok());
        assert!(validate_edge(&baseline, 3, 4).is_err());
        for mut stack in [
            vec![BytecodeType::String.into()],
            vec![
                StackValue::ExistVarProbeToken { begin: 1 },
                BytecodeType::String.into(),
            ],
        ] {
            assert!(
                apply(
                    &baseline,
                    2,
                    Opcode::FinishExistVarProbe,
                    &mut stack,
                    &probes
                )
                .is_err()
            );
        }
        for payload in [vec![], vec![0; 4], vec![0, 0, 0, 0, 2]] {
            let malformed = function(vec![EncodedInstruction::new(
                Opcode::FinishExistVarProbe,
                payload,
            )]);
            assert!(ProbeIndex::new(&malformed).is_err());
        }
        let mut wrong = baseline.clone();
        wrong.code[4] = finish(1, true);
        let probes = ProbeIndex::new(&wrong).ok().unwrap();
        assert!(
            apply(
                &wrong,
                0,
                Opcode::BeginExistVarProbe,
                &mut Vec::new(),
                &probes
            )
            .is_err()
        );
    }
}
