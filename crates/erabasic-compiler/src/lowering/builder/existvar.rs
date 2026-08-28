//! EXISTVAR's second source evaluation is conditional and script-catchable.
use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, HirCallArgument,
    Opcode, SourceLocation, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(super) fn lower_existvar(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        if name != "EXISTVAR" {
            return None;
        }
        let valid = (1..=2).contains(&arguments.len())
            && matches!(arguments.first(), Some(HirCallArgument::Value(value))
                if value.value_type == erabasic_hir::SemanticType::String)
            && arguments.get(1).is_none_or(|value| match value {
                HirCallArgument::Omitted => true,
                HirCallArgument::Value(value) => {
                    value.value_type == erabasic_hir::SemanticType::Integer
                }
                HirCallArgument::Place(_) => false,
            });
        if !valid {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                "EXISTVAR requires String source and optional Integer mode",
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid EXISTVAR operands".to_vec()),
                location,
            );
            return Some(BytecodeType::Integer);
        }
        let HirCallArgument::Value(source) = &arguments[0] else {
            unreachable!("source checked")
        };
        self.lower_expression(source, location);
        self.emit(
            EncodedInstruction::new(Opcode::ProbeVariableName, Vec::new()),
            location,
        );
        let Some(HirCallArgument::Value(mode)) = arguments.get(1) else {
            return Some(BytecodeType::Integer);
        };
        self.lower_expression(mode, location);
        let zero_mode = self.code.len();
        self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        let begin = self.code.len();
        self.emit(
            EncodedInstruction::new(Opcode::BeginExistVarProbe, vec![0; 4]),
            location,
        );
        // Deliberately lower the original expression again, after mode. Do not
        // memoize its value: it may invoke methods, mutate state, or await a Host.
        self.lower_expression(source, location);
        self.emit(existvar_finish(begin, false), location);
        let success = self.code.len();
        self.emit(opcode::jump(Opcode::Jump, 0), location);
        let failure = self.code.len();
        self.emit(existvar_finish(begin, true), location);
        self.code[begin].payload = u32::try_from(failure)
            .unwrap_or(u32::MAX)
            .to_le_bytes()
            .to_vec()
            .into();
        let end = self.code.len();
        self.patch_jump(zero_mode, end);
        self.patch_jump(success, end);
        Some(BytecodeType::Integer)
    }
}

fn existvar_finish(begin: usize, caught: bool) -> EncodedInstruction {
    let mut payload = u32::try_from(begin)
        .unwrap_or(u32::MAX)
        .to_le_bytes()
        .to_vec();
    payload.push(u8::from(caught));
    EncodedInstruction::new(Opcode::FinishExistVarProbe, payload)
}
