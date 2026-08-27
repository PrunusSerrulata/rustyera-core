//! HTML observation operands have reference-defined, non-eager evaluation order.

use erabasic_hir::{HirCallArgument, SemanticType};

use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, Opcode,
    SourceLocation, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(super) fn lower_html_query(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        if !matches!(name, "HTML_STRINGLEN" | "HTML_STRINGLINES") {
            return None;
        }
        if !valid_arguments(name, arguments) {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                format!("{name} requires a string source and integer width/unit argument"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid HTML query operands".to_vec()),
                location,
            );
            return Some(BytecodeType::Integer);
        }
        let HirCallArgument::Value(source) = &arguments[0] else {
            unreachable!("validated source value")
        };
        self.lower_expression(source, location);
        if name == "HTML_STRINGLEN" {
            // A malformed later HTML part must fail before the unit flag runs.
            self.emit_html_step(
                "HTML__MEASURE_LENGTH",
                &[BytecodeType::String],
                BytecodeType::Integer,
                location,
            );
            if let Some(HirCallArgument::Value(flag)) = arguments.get(1) {
                self.lower_expression(flag, location);
            } else {
                self.emit(opcode::push_integer(0), location);
            }
            self.emit_html_step(
                "HTML__LENGTH_UNIT",
                &[BytecodeType::Integer, BytecodeType::Integer],
                BytecodeType::Integer,
                location,
            );
        } else {
            self.lower_html_lines(arguments, location);
        }
        Some(BytecodeType::Integer)
    }

    fn lower_html_lines(&mut self, arguments: &[HirCallArgument], location: SourceLocation) {
        self.emit_html_step(
            "HTML__LINES_BEGIN",
            &[BytecodeType::String],
            BytecodeType::String,
            location,
        );
        // The owned runtime flow carries total budgets and exact caller identity.
        // Keeping its ticket on the operand stack preserves recursion and suspension.
        let next = self.code.len();
        self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
        self.emit_html_step(
            "HTML__LINES_MORE",
            &[BytecodeType::String],
            BytecodeType::Integer,
            location,
        );
        let done = self.code.len();
        self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
        let HirCallArgument::Value(width) = &arguments[1] else {
            unreachable!("validated width value")
        };
        // Empty input skips this expression; nonempty tails re-evaluate it per split.
        self.lower_expression(width, location);
        self.emit_html_step(
            "HTML__LINES_STEP",
            &[BytecodeType::String, BytecodeType::Integer],
            BytecodeType::Integer,
            location,
        );
        self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        self.emit(
            opcode::jump(Opcode::Jump, u32::try_from(next).unwrap_or(u32::MAX)),
            location,
        );
        self.patch_jump(done, self.code.len());
        self.emit_html_step(
            "HTML__LINES_END",
            &[BytecodeType::String],
            BytecodeType::Integer,
            location,
        );
    }

    fn emit_html_step(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: BytecodeType,
        location: SourceLocation,
    ) {
        self.emit_host_call(
            name,
            parameters,
            Some(result),
            &crate::registry::html_query_binding(name),
            location,
        );
    }
}

fn valid_arguments(name: &str, arguments: &[HirCallArgument]) -> bool {
    let valid_count = if name == "HTML_STRINGLEN" {
        (1..=2).contains(&arguments.len())
    } else {
        arguments.len() == 2
    };
    valid_count
        && matches!(arguments.first(), Some(HirCallArgument::Value(value)) if value.value_type == SemanticType::String)
        && arguments.get(1).is_none_or(|argument| {
            matches!(argument, HirCallArgument::Value(value) if value.value_type == SemanticType::Integer)
        })
}
