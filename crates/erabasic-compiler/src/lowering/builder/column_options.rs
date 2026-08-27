//! Keep the selected column identity across separately evaluated DEFAULT values.

use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, HirArgument,
    Opcode, SemanticType, SourceLocation, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(super) fn lower_column_options(
        &mut self,
        arguments: &[HirArgument],
        location: SourceLocation,
    ) {
        if !valid_arguments(arguments) {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                "DT_COLUMN_OPTIONS requires string table/column and DEFAULT/value pairs",
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"DT_COLUMN_OPTIONS.invalid".to_vec()),
                location,
            );
            return;
        }
        let contract = crate::registry::column_options_contract();
        // The reference selects column before evaluating table, and holds that object
        // even if a subsequent value expression deletes or replaces its table.
        self.lower_argument(&arguments[1], location);
        self.lower_argument(&arguments[0], location);
        self.emit_native_call(
            "dt__column_resolve",
            &[BytecodeType::String, BytecodeType::String],
            Some(BytecodeType::String),
            contract,
            location,
        );
        self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
        self.emit(opcode::push_string(""), location);
        self.emit(
            opcode::binary(super::super::binary_tag(erabasic_ast::BinaryOp::Equal)),
            location,
        );
        let present = self.code.len();
        self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        self.emit(
            EncodedInstruction::new(Opcode::Trap, b"DT_COLUMN_OPTIONS.missing".to_vec()),
            location,
        );
        self.patch_jump(present, self.code.len());
        for pair in arguments[2..].chunks_exact(2) {
            let (check, apply, value_type) =
                if argument_type(&pair[1]) == Some(SemanticType::String) {
                    (
                        "dt__column_check_str",
                        "dt__column_apply_str",
                        BytecodeType::String,
                    )
                } else {
                    (
                        "dt__column_check_int",
                        "dt__column_apply_int",
                        BytecodeType::Integer,
                    )
                };
            self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
            self.emit_native_call(check, &[BytecodeType::String], None, contract, location);
            self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
            self.lower_argument(&pair[1], location);
            self.emit_native_call(
                apply,
                &[BytecodeType::String, value_type],
                None,
                contract,
                location,
            );
        }
        self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
    }
}

fn argument_type(argument: &HirArgument) -> Option<SemanticType> {
    match argument {
        HirArgument::Expression(expression) => Some(expression.value_type),
        _ => None,
    }
}

fn valid_arguments(arguments: &[HirArgument]) -> bool {
    arguments.len() >= 4
        && arguments.len().is_multiple_of(2)
        && arguments[..2]
            .iter()
            .all(|argument| argument_type(argument) == Some(SemanticType::String))
        && arguments[2..].chunks_exact(2).all(|pair| {
            matches!(&pair[0], HirArgument::Raw(value) if value == "DEFAULT")
                && matches!(
                    argument_type(&pair[1]),
                    Some(SemanticType::Integer | SemanticType::String)
                )
        })
}
