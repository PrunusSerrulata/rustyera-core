use super::super::{
    BytecodeType, EncodedInstruction, HirFormPart, HirFormattedString, Opcode, SourceLocation,
    compiler_native_contract, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(in super::super) fn lower_formatted(
        &mut self,
        formatted: &HirFormattedString,
        fallback: SourceLocation,
    ) -> BytecodeType {
        let mut parts = 0u16;
        for part in &formatted.parts {
            match part {
                HirFormPart::Text { value } => {
                    self.emit(opcode::push_string(value), formatted.location);
                }
                HirFormPart::Interpolation {
                    expression,
                    width,
                    alignment,
                    integer,
                    location,
                } => {
                    let value_type = self.lower_expression(expression, fallback);
                    if let Some(width) = width {
                        let parameters = vec![
                            value_type,
                            self.lower_expression(width, fallback),
                            BytecodeType::Integer,
                        ];
                        self.emit(
                            opcode::push_integer(i64::from(matches!(
                                alignment,
                                Some(erabasic_ast::Alignment::Left)
                            ))),
                            *location,
                        );
                        self.emit_native_call(
                            if *integer {
                                "format_integer"
                            } else {
                                "format_string"
                            },
                            &parameters,
                            Some(BytecodeType::String),
                            compiler_native_contract(true),
                            *location,
                        );
                    } else if *integer {
                        // Width-free integer formatting is exactly decimal conversion. Keep it in
                        // bytecode so dynamic function names do not cross a native-call boundary.
                        self.emit(
                            EncodedInstruction::new(Opcode::ToString, Vec::new()),
                            *location,
                        );
                    } else {
                        debug_assert_eq!(value_type, BytecodeType::String);
                    }
                }
                HirFormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    location,
                } => {
                    self.lower_expression(condition, fallback);
                    let false_jump = self.code.len();
                    self.emit(opcode::jump(Opcode::JumpIfFalse, 0), *location);
                    self.lower_formatted(then_value, fallback);
                    let end_jump = self.code.len();
                    self.emit(opcode::jump(Opcode::Jump, 0), *location);
                    self.patch_jump(false_jump, self.code.len());
                    if let Some(else_value) = else_value {
                        self.lower_formatted(else_value, fallback);
                    } else {
                        self.emit(opcode::push_string(""), *location);
                    }
                    self.patch_jump(end_jump, self.code.len());
                }
                HirFormPart::Triple { symbol, location } => {
                    self.lower_form_triple(*symbol, *location);
                }
            }
            parts = parts.saturating_add(1);
        }
        if parts == 0 {
            self.emit(opcode::push_string(""), formatted.location);
        } else if parts > 1 {
            self.emit(opcode::concat(parts), formatted.location);
        }
        BytecodeType::String
    }

    fn lower_form_triple(&mut self, symbol: char, location: SourceLocation) {
        let (value_name, index_name) = match symbol {
            '*' => ("NAME", "TARGET"),
            '+' => ("CALLNAME", "MASTER"),
            '=' => ("CALLNAME", "PLAYER"),
            '/' => ("NAME", "ASSI"),
            '$' => ("CALLNAME", "TARGET"),
            _ => {
                self.emit(opcode::push_string(&symbol.to_string()), location);
                return;
            }
        };
        let variable_key = |name: &str| {
            self.context
                .program
                .variables
                .iter()
                .find(|variable| variable.name.eq_ignore_ascii_case(name))
                .and_then(|variable| self.context.variable_keys.get(variable.id.0))
                .copied()
        };
        let (Some(index), Some(value)) = (variable_key(index_name), variable_key(value_name))
        else {
            self.emit(
                EncodedInstruction::new(
                    Opcode::Trap,
                    format!("FORM triple {symbol}{symbol}{symbol} variables are missing")
                        .into_bytes(),
                ),
                location,
            );
            return;
        };
        self.emit(
            opcode::variable(Opcode::LoadVariable, index, 0, 0),
            location,
        );
        self.emit(
            opcode::variable(Opcode::LoadVariable, value, 1, 0),
            location,
        );
    }
}
