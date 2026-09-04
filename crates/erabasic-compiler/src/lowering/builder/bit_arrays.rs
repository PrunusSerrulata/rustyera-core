//! Preserve the reference's capture-before-tail order without evaluating array indices.

use erabasic_bytecode::{BitCallSpec, BitOperation};

use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, EncodedInstruction, HirCallArgument,
    Opcode, SourceLocation,
};
use super::Builder;

impl Builder<'_> {
    pub(super) fn lower_bit_call(
        &mut self,
        name: &str,
        arguments: &[HirCallArgument],
        location: SourceLocation,
    ) -> Option<BytecodeType> {
        let operation = BitOperation::from_name(name)?;
        let spec = self.bit_call_spec(operation, arguments);
        let Some(spec) = spec else {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                "bit-array call has invalid token, argument type or omission metadata",
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid bit-array call".to_vec()),
                location,
            );
            return Some(BytecodeType::Integer);
        };
        let begin = self.code.len();
        self.emit(
            EncodedInstruction::new(Opcode::BeginBitCall, spec.encode().to_vec()),
            location,
        );
        for argument in &arguments[1..] {
            if let HirCallArgument::Value(expression) = argument {
                self.lower_expression(expression, location);
            }
        }
        self.emit(
            EncodedInstruction::new(
                Opcode::FinishBitCall,
                u32::try_from(begin)
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec(),
            ),
            location,
        );
        Some(BytecodeType::Integer)
    }

    fn bit_call_spec(
        &self,
        operation: BitOperation,
        arguments: &[HirCallArgument],
    ) -> Option<BitCallSpec> {
        let HirCallArgument::Place(input) = arguments.first()? else {
            return None;
        };
        let input = *self.context.variable_keys.get(input.variable.0)?;
        let mut present = 0_u8;
        for (index, argument) in arguments.iter().skip(1).enumerate() {
            match argument {
                HirCallArgument::Value(expression)
                    if expression.value_type == erabasic_hir::SemanticType::Integer =>
                {
                    present |= 1_u8.checked_shl(u32::try_from(index).ok()?)?;
                }
                HirCallArgument::Omitted => {}
                _ => return None,
            }
        }
        let spec = BitCallSpec {
            operation,
            input,
            tail_count: u8::try_from(arguments.len() - 1).ok()?,
            present,
        };
        BitCallSpec::decode(&spec.encode()).ok()
    }
}
