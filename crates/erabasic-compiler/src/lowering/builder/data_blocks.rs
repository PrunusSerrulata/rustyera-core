use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, DataBlock, EncodedInstruction,
    HirArgument, HirStatementKind, Opcode, SemanticType, SourceLocation, TryListBlock,
    argument_place, compiler_native_contract, opcode,
};
use super::Builder;

impl Builder<'_> {
    pub(in super::super) fn lower_data_block(&mut self, block: &DataBlock<'_>) {
        let HirStatementKind::Instruction { target, arguments } = &block.opener.kind else {
            return;
        };
        if block.choices.is_empty() {
            return;
        }
        let name = target.name();
        let location = block.opener.location;
        let is_string = name == "STRDATA";
        let mut skip_jump = None;
        if !is_string {
            self.emit_runtime_call("ISSKIP", &[], Some(BytecodeType::Integer), false, location);
            self.emit(opcode::unary(2), location);
            skip_jump = Some(self.code.len());
            self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
        }
        self.emit(
            opcode::push_integer(i64::try_from(block.choices.len()).unwrap_or(i64::MAX)),
            location,
        );
        self.emit_native_call(
            "RAND",
            &[BytecodeType::Integer],
            Some(BytecodeType::Integer),
            compiler_native_contract(false),
            location,
        );

        if !is_string
            && let Some(place) = argument_place(arguments.first())
            && let Some(key) = self.context.variable_keys.get(place.variable.0).copied()
        {
            self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
            for index in &place.indices {
                self.lower_expression(index, location);
            }
            self.emit(
                opcode::variable(
                    Opcode::MakePlace,
                    key,
                    u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                    0,
                ),
                location,
            );
            self.emit(
                EncodedInstruction::new(Opcode::StorePlace, Vec::new()),
                location,
            );
        }

        let mut end_jumps = Vec::new();
        for (index, choice) in block.choices.iter().enumerate() {
            let false_jump = if index + 1 < block.choices.len() {
                self.emit(EncodedInstruction::new(Opcode::Dup, Vec::new()), location);
                self.emit(
                    opcode::push_integer(i64::try_from(index).unwrap_or(i64::MAX)),
                    location,
                );
                self.emit(opcode::binary(11), location);
                let jump = self.code.len();
                self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
                Some(jump)
            } else {
                None
            };
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            if is_string {
                self.lower_strdata_choice(choice, arguments.first(), location);
            } else {
                self.lower_printdata_choice(choice, name);
            }
            let end = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            end_jumps.push(end);
            if let Some(jump) = false_jump {
                self.code[jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec()
                    .into();
            }
        }
        let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
        for jump in end_jumps {
            self.code[jump].payload = end.to_le_bytes().to_vec().into();
        }
        if !is_string {
            if name.ends_with('L') {
                self.emit_runtime_call("PRINTL", &[], None, false, location);
            } else if name.ends_with('W') {
                self.emit_runtime_call("PRINTW", &[], None, false, location);
            }
        }
        if let Some(jump) = skip_jump {
            self.code[jump].payload = u32::try_from(self.code.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes()
                .to_vec()
                .into();
        }
    }

    pub(in super::super) fn lower_printdata_choice(
        &mut self,
        choice: &[&erabasic_hir::HirStatement],
        opener: &str,
    ) {
        for (index, line) in choice.iter().enumerate() {
            let HirStatementKind::Instruction { arguments, .. } = &line.kind else {
                continue;
            };
            if index != 0 {
                self.emit_runtime_call("PRINTL", &[], None, false, line.location);
            }
            let Some(argument) = arguments.first() else {
                continue;
            };
            let value_type = self.lower_argument(argument, line.location);
            let command = if opener.contains('K') {
                "PRINTK"
            } else if opener.contains('D') {
                "PRINTD"
            } else {
                "PRINT"
            };
            self.emit_runtime_call(command, &[value_type], None, false, line.location);
        }
    }

    pub(in super::super) fn lower_strdata_choice(
        &mut self,
        choice: &[&erabasic_hir::HirStatement],
        destination: Option<&HirArgument>,
        location: SourceLocation,
    ) {
        let mut parts = 0_u16;
        for (index, line) in choice.iter().enumerate() {
            if index != 0 {
                self.emit(opcode::push_string("\n"), line.location);
                parts = parts.saturating_add(1);
            }
            if let HirStatementKind::Instruction { arguments, .. } = &line.kind
                && let Some(argument) = arguments.first()
            {
                self.lower_argument(argument, line.location);
                parts = parts.saturating_add(1);
            }
        }
        if parts == 0 {
            self.emit(opcode::push_string(""), location);
        } else if parts > 1 {
            self.emit(opcode::concat(parts), location);
        }
        let default_destination;
        let place = if let Some(place) = argument_place(destination) {
            place
        } else {
            let Some(variable) = self
                .context
                .program
                .variables
                .iter()
                .find(|variable| variable.name.eq_ignore_ascii_case("RESULTS"))
            else {
                self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
                return;
            };
            default_destination = erabasic_hir::HirPlace {
                variable: variable.id,
                indices: Vec::new(),
                value_type: SemanticType::String,
                mutable: true,
                location,
            };
            &default_destination
        };
        if let Some(key) = self.context.variable_keys.get(place.variable.0).copied() {
            for index in &place.indices {
                self.lower_expression(index, location);
            }
            self.emit(
                opcode::variable(
                    Opcode::MakePlace,
                    key,
                    u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                    0,
                ),
                location,
            );
            self.emit(
                EncodedInstruction::new(Opcode::StorePlace, Vec::new()),
                location,
            );
        } else {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                "STRDATA destination has no stable symbol key",
            ));
        }
    }

    pub(in super::super) fn lower_try_list(&mut self, block: &TryListBlock<'_>) {
        let HirStatementKind::Instruction { target, .. } = &block.opener.kind else {
            return;
        };
        let opener = target.name();
        let mut end_jumps = Vec::new();
        for candidate in &block.candidates {
            let HirStatementKind::Instruction { arguments, .. } = &candidate.kind else {
                continue;
            };
            let Some(target) = arguments.first() else {
                continue;
            };
            self.lower_argument(target, candidate.location);
            if opener == "TRYGOTOLIST" {
                let instruction = self.code.len();
                self.emit(opcode::jump_dynamic_label(0), candidate.location);
                self.code[instruction].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec()
                    .into();
                continue;
            }
            let resolve = self.code.len();
            self.emit(opcode::resolve_function(0, true, false), candidate.location);
            let parameter_types = arguments
                .iter()
                .skip(1)
                .map(|argument| self.lower_argument(argument, candidate.location))
                .collect::<Vec<_>>();
            self.emit(
                opcode::invoke_dynamic(
                    u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                    opener == "TRYJUMPLIST",
                ),
                candidate.location,
            );
            if opener != "TRYJUMPLIST" {
                let end = self.code.len();
                self.emit(opcode::jump(Opcode::Jump, 0), candidate.location);
                end_jumps.push(end);
            }
            let missing = self.code.len();
            self.emit(
                EncodedInstruction::new(Opcode::Pop, Vec::new()),
                candidate.location,
            );
            self.code[resolve].payload = {
                let mut payload = u32::try_from(missing)
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                payload.push(1);
                payload.push(0);
                payload.into()
            };
        }
        let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
        for jump in end_jumps {
            self.code[jump].payload = end.to_le_bytes().to_vec().into();
        }
    }
}
