//! Ordered MATCH lowering: token -> begin -> length -> end -> needle -> live scan.
use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, HirCallArgument,
    HirExprKind, Opcode, SourceLocation, bytecode_type, opcode,
};
use super::Builder;
use erabasic_bytecode::{MatchCallSpec, MatchInput};

impl Builder<'_> {
    pub(super) fn lower_match(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        if !matches!(name, "MATCHALL" | "MATCHALLEX") {
            return None;
        }
        if let Err(message) = self.emit_match(name, arguments, location) {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                message,
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid MATCH operands".to_vec()),
                location,
            );
        }
        Some(BytecodeType::Integer)
    }
    fn emit_match(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Result<(), &'static str> {
        if !(2..=5).contains(&arguments.len()) {
            return Err("MATCH needs two to five arguments");
        }
        let key = |place: &erabasic_hir::HirPlace| {
            self.context
                .variable_keys
                .get(place.variable.0)
                .copied()
                .ok_or("MATCH variable key missing")
        };
        let (input, input_restructured_to_scalar) = match (&arguments[0], name) {
            (HirCallArgument::Place(place), "MATCHALL") => {
                let variable = self
                    .context
                    .program
                    .variables
                    .get(place.variable.0 as usize)
                    .ok_or("MATCH variable metadata missing")?;
                (
                    MatchInput::Variable(key(place)?),
                    variable.reference_semantics.can_restructure
                        && !place.indices.is_empty()
                        && place.indices.iter().all(|index| index.constant.is_some()),
                )
            }
            (HirCallArgument::Value(value), "MATCHALLEX") => match &value.kind {
                HirExprKind::String { value } => (MatchInput::Name(value.clone()), false),
                _ => return Err("MATCHALLEX requires a source string literal"),
            },
            _ => return Err("MATCH input token missing"),
        };
        let output = match arguments.get(4) {
            None => None,
            Some(HirCallArgument::Place(place)) => Some(key(place)?),
            _ => return Err("MATCH output token missing"),
        };
        let HirCallArgument::Value(needle) = &arguments[1] else {
            return Err("MATCH needle missing");
        };
        let needle_type = bytecode_type(needle.value_type)
            .filter(|value| matches!(value, BytecodeType::Integer | BytecodeType::String))
            .ok_or("MATCH needle must be scalar")?;
        let range = |index: usize| match arguments.get(index) {
            None | Some(HirCallArgument::Omitted) => Ok(None),
            Some(HirCallArgument::Value(value)) => bytecode_type(value.value_type)
                .filter(|value| matches!(value, BytecodeType::Integer | BytecodeType::String))
                .map(|kind| Some((value, kind)))
                .ok_or("MATCH range must be scalar"),
            _ => Err("MATCH range cannot be a place"),
        };
        let begin_value = range(2)?;
        let end_value = range(3)?;
        let spec = MatchCallSpec {
            input,
            input_restructured_to_scalar,
            output,
            needle: needle_type,
            begin_type: begin_value.map_or(BytecodeType::Integer, |(_, kind)| kind),
            end_type: end_value.map(|(_, kind)| kind),
        };
        let begin = self.code.len();
        self.emit(
            EncodedInstruction::new(Opcode::BeginMatchCall, spec.encode()),
            location,
        );
        if let Some((value, _)) = begin_value {
            self.lower_expression(value, location);
        } else {
            self.emit(opcode::push_integer(0), location);
        }
        self.emit(
            match_phase(Opcode::MatchCallRange, begin, Some(0)),
            location,
        );
        if let Some((value, _)) = end_value {
            self.lower_expression(value, location);
        }
        self.emit(
            match_phase(Opcode::MatchCallRange, begin, Some(1)),
            location,
        );
        self.lower_expression(needle, location);
        self.emit(match_phase(Opcode::FinishMatchCall, begin, None), location);
        Ok(())
    }
}
fn match_phase(op: Opcode, begin: usize, phase: Option<u8>) -> EncodedInstruction {
    let mut payload = u32::try_from(begin)
        .unwrap_or(u32::MAX)
        .to_le_bytes()
        .to_vec();
    if let Some(phase) = phase {
        payload.push(phase);
    }
    EncodedInstruction::new(op, payload)
}
