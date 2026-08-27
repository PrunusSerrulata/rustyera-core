use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, ControlFlowKind, EncodedInstruction,
    FunctionKind, HirArgument, InstructionTarget, LineId, Opcode, SemanticType, SourceLocation,
    SymbolKey, assign_tag, bytecode_type, compiler_native_contract, formatted_constant, opcode,
};
use super::Builder;

impl Builder<'_> {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn lower_statement(
        &mut self,
        target: &InstructionTarget,
        arguments: &[HirArgument],
        line: LineId,
        location: SourceLocation,
    ) {
        let name = target.name();
        if matches!(name, "VARI" | "VARS") {
            // Scoped array declarations only allocate frame storage. The
            // enclosing lowering loop emits a source-mapped NOP for this line.
            return;
        }
        if let InstructionTarget::BuiltinMethod { return_type, .. } = target {
            if matches!(name, "GETMETH" | "GETMETHS") {
                self.lower_expression_method_statement(name, arguments, location);
                self.store_method_result(*return_type, location);
                return;
            }
            let parameter_types = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect::<Vec<_>>();
            let result = bytecode_type(*return_type);
            let runtime_name = match name {
                "STRLENFORM" => "STRLEN",
                "STRLENFORMU" => "STRLENU",
                _ => name,
            };
            self.emit_runtime_call(runtime_name, &parameter_types, result, false, location);
            if result.is_some() {
                self.store_method_result(*return_type, location);
            }
            return;
        }
        if name == "VARSIZE" {
            let mut parameter_types = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect::<Vec<_>>();
            let result = self
                .context
                .program
                .variables
                .iter()
                .find(|variable| variable.name.eq_ignore_ascii_case("RESULT"))
                .and_then(|variable| self.context.variable_keys.get(variable.id.0))
                .copied();
            if let Some(result) = result {
                self.emit(opcode::variable(Opcode::MakePlace, result, 0, 0), location);
                parameter_types.push(BytecodeType::IntegerPlace);
                self.emit_runtime_call(name, &parameter_types, None, false, location);
            } else {
                self.emit(
                    EncodedInstruction::new(
                        Opcode::Trap,
                        b"VARSIZE result variable is missing".to_vec(),
                    ),
                    location,
                );
            }
            return;
        }
        if name == "GETMILLISECOND" {
            self.emit_runtime_call(name, &[], Some(BytecodeType::Integer), false, location);
            self.store_method_result(SemanticType::Integer, location);
            return;
        }
        if name == "CURRENTREDRAW" {
            self.emit_runtime_call(name, &[], Some(BytecodeType::Integer), false, location);
            self.store_method_result(SemanticType::Integer, location);
            return;
        }
        if name == "ENCODETOUNI" {
            let parameter_type = if let Some(argument) = arguments.first() {
                self.lower_argument(argument, location)
            } else {
                self.emit(opcode::push_string(""), location);
                BytecodeType::String
            };
            self.emit_runtime_call(
                "__ENCODETOUNI_RESULT",
                &[parameter_type],
                None,
                false,
                location,
            );
            return;
        }
        if matches!(name, "BREAK" | "CONTINUE" | "RESTART" | "GOTO" | "TRYGOTO") {
            // Their concrete jump is emitted from the analyzed control-flow edge
            // after this statement; they are not Host operations.
            return;
        }
        if name == "SET" && arguments.len() > 2 {
            self.lower_assignment_list(arguments, location);
            return;
        }
        if name == "TIMES" {
            let parameter_types = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect::<Vec<_>>();
            self.emit_native_call(
                "times",
                &parameter_types,
                None,
                compiler_native_contract(false),
                location,
            );
            return;
        }
        if name == "POWER" {
            let Some(destination) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"POWER destination is missing".to_vec()),
                    location,
                );
                return;
            };
            let parameter_types = arguments
                .iter()
                .skip(1)
                .map(|argument| self.lower_argument(argument, location))
                .collect::<Vec<_>>();
            self.emit_runtime_call(
                "POWER",
                &parameter_types,
                Some(BytecodeType::Integer),
                false,
                location,
            );
            let destination_type = self.lower_argument(destination, location);
            if destination_type != BytecodeType::IntegerPlace {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    "POWER destination is not an integer place",
                ));
            }
            self.emit(
                EncodedInstruction::new(Opcode::StorePlace, Vec::new()),
                location,
            );
            return;
        }
        if name == "DO" {
            return;
        }
        if name == "LOOP" {
            if let Some(condition) = arguments.first() {
                self.lower_argument(condition, location);
            } else {
                self.emit(opcode::push_integer(1), location);
            }
            // JumpIfFalse below becomes a jump-on-true after this inversion.
            self.emit(opcode::unary(2), location);
            return;
        }
        if name == "REPEAT" {
            let counter = self
                .context
                .program
                .variables
                .iter()
                .find(|variable| {
                    variable.owner.is_none() && variable.name.eq_ignore_ascii_case("COUNT")
                })
                .and_then(|variable| self.context.variable_keys.get(variable.id.0))
                .copied();
            let Some(counter) = counter else {
                self.emit(
                    EncodedInstruction::new(
                        Opcode::Trap,
                        b"REPEAT COUNT variable is missing".to_vec(),
                    ),
                    location,
                );
                return;
            };
            let Some(end) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"REPEAT has no count".to_vec()),
                    location,
                );
                return;
            };
            // Emuera defines REPEAT as FOR COUNT:0, 0, count, 1. Reuse the
            // validated FOR state machine so REND, CONTINUE and BREAK share the
            // same counter lifetime and termination rules.
            self.emit(opcode::push_integer(0), location);
            self.emit(opcode::variable(Opcode::MakePlace, counter, 1, 0), location);
            self.emit(opcode::push_integer(0), location);
            self.lower_argument(end, location);
            self.emit(opcode::push_integer(1), location);
            self.emit(
                EncodedInstruction::new(Opcode::ForStart, Vec::new()),
                location,
            );
            return;
        }
        if name == "FOR" {
            let Some(HirArgument::Place(counter)) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"FOR counter is not a place".to_vec()),
                    location,
                );
                return;
            };
            self.lower_place(counter, location);
            match arguments.get(1) {
                Some(HirArgument::Omitted) | None => self.emit(opcode::push_integer(0), location),
                Some(start) => {
                    self.lower_argument(start, location);
                }
            }
            if let Some(end) = arguments.get(2) {
                self.lower_argument(end, location);
            }
            match arguments.get(3) {
                Some(HirArgument::Omitted) | None => self.emit(opcode::push_integer(1), location),
                Some(step) => {
                    self.lower_argument(step, location);
                }
            }
            self.emit(
                EncodedInstruction::new(Opcode::ForStart, Vec::new()),
                location,
            );
            return;
        }
        if matches!(name, "NEXT" | "REND") {
            self.emit(
                EncodedInstruction::new(Opcode::ForNext, Vec::new()),
                location,
            );
            self.emit(opcode::unary(2), location);
            return;
        }
        if name == "SELECTCASE" {
            if let Some(selector) = arguments.first() {
                self.lower_argument(selector, location);
                self.emit(
                    EncodedInstruction::new(Opcode::SelectStart, Vec::new()),
                    location,
                );
            } else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"SELECTCASE has no value".to_vec()),
                    location,
                );
            }
            return;
        }
        if name == "CASE" {
            let mut index = 0;
            let mut comparisons = 0;
            while index < arguments.len() {
                let Some(HirArgument::Raw(tag)) = arguments.get(index) else {
                    break;
                };
                let operation = match tag.as_str() {
                    "eq" => 0,
                    "ne" => 1,
                    "lt" => 2,
                    "le" => 3,
                    "gt" => 4,
                    "ge" => 5,
                    "range" => 6,
                    "and" => 7,
                    _ => break,
                };
                let operands = if operation == 6 { 2 } else { 1 };
                if index + operands >= arguments.len() {
                    break;
                }
                for argument in &arguments[index + 1..=index + operands] {
                    self.lower_argument(argument, location);
                }
                self.emit(
                    EncodedInstruction::new(Opcode::SelectCompare, vec![operation]),
                    location,
                );
                if comparisons > 0 {
                    self.emit(opcode::binary(18), location);
                }
                comparisons += 1;
                index += operands + 1;
            }
            if comparisons == 0 {
                self.emit(opcode::push_integer(0), location);
            }
            return;
        }
        if name == "CASEELSE" {
            self.emit(
                EncodedInstruction::new(Opcode::SelectCompare, vec![8]),
                location,
            );
            return;
        }
        if name == "ENDSELECT" {
            self.emit(
                EncodedInstruction::new(Opcode::SelectEnd, Vec::new()),
                location,
            );
            return;
        }
        if matches!(name, "NOSKIP" | "ENDNOSKIP") {
            // The analyzer's block edge uses the ordinary conditional-branch shape.
            // NOSKIP therefore produces an internal true value while ENDNOSKIP is void.
            self.emit_runtime_call(
                name,
                &[],
                (name == "NOSKIP").then_some(BytecodeType::Integer),
                false,
                location,
            );
            return;
        }
        if name.starts_with("PRINTDATA")
            || matches!(
                name,
                "IF" | "ELSE"
                    | "ELSEIF"
                    | "ENDIF"
                    | "SIF"
                    | "WHILE"
                    | "WEND"
                    | "TRYC"
                    | "CATCH"
                    | "ENDCATCH"
                    | "STRDATA"
                    | "DATALIST"
                    | "DATA"
                    | "DATAFORM"
                    | "ENDDATA"
                    | "ENDLIST"
                    | "TRYCALLLIST"
                    | "TRYJUMPLIST"
                    | "TRYGOTOLIST"
                    | "FUNC"
                    | "ENDFUNC"
            )
        {
            let parameter_types: Vec<_> = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect();
            let direct_condition = matches!(name, "IF" | "ELSEIF" | "SIF" | "WHILE");
            let has_branch = self
                .control_flow_by_line
                .get(line.0)
                .into_iter()
                .flatten()
                .any(|edge| edge.from == line && edge.kind == ControlFlowKind::Branch);
            if name != "ELSE" && !direct_condition && (has_branch || !parameter_types.is_empty()) {
                self.emit_native_call(
                    &format!("control_{}", name.to_ascii_lowercase()),
                    &parameter_types,
                    has_branch.then_some(BytecodeType::Integer),
                    compiler_native_contract(false),
                    location,
                );
            }
            return;
        }
        if name == "RETURN" {
            self.lower_era_return(arguments, location);
            return;
        }
        if name == "RETURNF" {
            if let Some(argument) = arguments.first() {
                self.lower_argument(argument, location);
            } else {
                self.emit_default_method_value(location);
            }
            self.emit(opcode::return_value(true), location);
            return;
        }
        if name == "RETURNFORM" {
            // RETURNFORM is legacy RESULT-list syntax rather than a string method
            // return. Its formatted payload is evaluated by a dedicated native.
            let parameter_types = arguments
                .iter()
                .map(|argument| self.lower_argument(argument, location))
                .collect::<Vec<_>>();
            self.emit_native_call(
                "returnform",
                &parameter_types,
                Some(BytecodeType::Integer),
                compiler_native_contract(false),
                location,
            );
            self.emit(opcode::return_value(true), location);
            return;
        }
        if matches!(name, "TRYCGOTO" | "TRYCGOTOFORM") {
            let Some(target) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"missing dynamic label".to_vec()),
                    location,
                );
                return;
            };
            if let HirArgument::Formatted(formatted) = target
                && let Some(constant) = formatted_constant(formatted)
                && !self
                    .hir_function
                    .labels
                    .iter()
                    .any(|(_, label, _)| label.eq_ignore_ascii_case(&constant))
            {
                // The reference loader resolves constant TRYCGOTO targets early.
                // A missing constant then falls through to CATCH, whose marker skips
                // the fallback body; only a late-bound miss enters that body.
                self.emit(EncodedInstruction::new(Opcode::Nop, Vec::new()), location);
                return;
            }
            self.lower_argument(target, location);
            self.emit(opcode::jump_dynamic_label(0), location);
            return;
        }
        if name == "CALLEVENT" {
            if self.hir_function.kind == FunctionKind::Event {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    "CALLEVENT is not allowed inside an event function",
                ));
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"CALLEVENT inside event".to_vec()),
                    location,
                );
                return;
            }
            let Some(target) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"missing event name".to_vec()),
                    location,
                );
                return;
            };
            let constant_name = match target {
                HirArgument::Raw(value) => Some(value.as_str()),
                HirArgument::Expression(expression) => match &expression.constant {
                    Some(erabasic_hir::ConstantValue::String(value)) => Some(value.as_str()),
                    _ => None,
                },
                HirArgument::MixedExpression { .. }
                | HirArgument::Formatted(_)
                | HirArgument::Place(_)
                | HirArgument::Omitted => None,
            };
            if constant_name.is_none() {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    "CALLEVENT requires a constant string event name",
                ));
            }
            self.lower_argument(target, location);
            self.emit(opcode::invoke_event(), location);
            return;
        }
        if matches!(
            name,
            "CALLFORM"
                | "CALLFORMF"
                | "JUMPFORM"
                | "TRYCALLFORM"
                | "TRYCALLFORMF"
                | "TRYJUMPFORM"
                | "TRYCCALL"
                | "TRYCCALLFORM"
                | "TRYCJUMP"
                | "TRYCJUMPFORM"
        ) {
            self.lower_dynamic_call(arguments, line, name, location);
            return;
        }
        if matches!(
            name,
            "CALL" | "CALLF" | "JUMP" | "TRYCALL" | "TRYCALLF" | "TRYJUMP"
        ) {
            self.lower_static_call(arguments, line, name, location);
            return;
        }
        let mut parameter_types = std::mem::take(&mut self.argument_types);
        parameter_types.reserve(arguments.len().saturating_mul(2));
        for argument in arguments {
            if let HirArgument::MixedExpression { expression, is_px } = argument {
                parameter_types.push(self.lower_expression(expression, location));
                self.emit(opcode::push_integer(i64::from(*is_px)), location);
                parameter_types.push(BytecodeType::Integer);
            } else {
                parameter_types.push(self.lower_argument(argument, location));
            }
        }
        let extension = matches!(target, InstructionTarget::Extension(_));
        self.emit_runtime_call(name, &parameter_types, None, extension, location);
        parameter_types.clear();
        self.argument_types = parameter_types;
    }

    fn store_method_result(&mut self, return_type: SemanticType, location: SourceLocation) {
        let variable_name = match return_type {
            SemanticType::Integer => "RESULT",
            SemanticType::String => "RESULTS",
            SemanticType::Void | SemanticType::Error => return,
        };
        let result = self
            .context
            .program
            .variables
            .iter()
            .find(|variable| variable.name.eq_ignore_ascii_case(variable_name))
            .and_then(|variable| self.context.variable_keys.get(variable.id.0))
            .copied();
        if let Some(result) = result {
            self.emit(
                opcode::variable(Opcode::StoreVariable, result, 0, 0),
                location,
            );
        } else {
            self.emit(
                EncodedInstruction::new(
                    Opcode::Trap,
                    format!("{variable_name} variable is missing").into_bytes(),
                ),
                location,
            );
        }
    }

    fn lower_era_return(&mut self, arguments: &[HirArgument], location: SourceLocation) {
        let result = self.era_result_key();
        if arguments.is_empty() {
            // Emuera treats an omitted RETURN value as zero. Only RESULT:0 is
            // overwritten; the remaining legacy RESULT entries stay unchanged.
            self.reset_era_result(location);
            if self.hir_function.kind == FunctionKind::Method {
                self.emit_default_method_value(location);
                self.emit(opcode::return_value(true), location);
            } else {
                self.emit(opcode::return_value(false), location);
            }
            return;
        }
        let values: &[HirArgument] = arguments;
        if let Some(result) = result {
            for (index, argument) in values.iter().enumerate() {
                self.emit(
                    opcode::push_integer(i64::try_from(index).unwrap_or(i64::MAX)),
                    location,
                );
                self.lower_argument(argument, location);
                self.emit(
                    opcode::variable(Opcode::StoreVariable, result, 1, 0),
                    location,
                );
            }
            if self.hir_function.kind == FunctionKind::Method {
                self.emit_default_method_value(location);
            } else {
                self.emit(opcode::push_integer(0), location);
                self.emit(
                    opcode::variable(Opcode::LoadVariable, result, 1, 0),
                    location,
                );
            }
        } else {
            self.emit(opcode::push_integer(0), location);
        }
        self.emit(opcode::return_value(true), location);
    }

    pub(in super::super) fn emit_default_method_value(&mut self, location: SourceLocation) {
        match self.hir_function.return_type {
            SemanticType::String => self.emit(opcode::push_string(""), location),
            SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                self.emit(opcode::push_integer(0), location);
            }
        }
    }

    pub(in super::super) fn lower_era_fallthrough(&mut self, location: SourceLocation) {
        if self.hir_function.kind == FunctionKind::Method {
            self.emit_default_method_value(location);
            self.emit(opcode::return_value(true), location);
        } else {
            // Reaching the next label is an implicit `RETURN 0` in Emuera.
            self.reset_era_result(location);
            self.emit(opcode::return_value(false), location);
        }
    }

    fn era_result_key(&self) -> Option<SymbolKey> {
        self.context
            .program
            .variables
            .iter()
            .find(|variable| variable.name.eq_ignore_ascii_case("RESULT"))
            .and_then(|variable| self.context.variable_keys.get(variable.id.0))
            .copied()
    }

    fn reset_era_result(&mut self, location: SourceLocation) {
        if let Some(result) = self.era_result_key() {
            self.emit(opcode::push_integer(0), location);
            self.emit(opcode::push_integer(0), location);
            self.emit(
                opcode::variable(Opcode::StoreVariable, result, 1, 0),
                location,
            );
        }
    }

    fn lower_assignment_list(&mut self, arguments: &[HirArgument], location: SourceLocation) {
        let Some(HirArgument::Place(place)) = arguments.first() else {
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"SET list target is not a place".to_vec()),
                location,
            );
            return;
        };
        let Some(key) = self.context.variable_keys.get(place.variable.0).copied() else {
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"SET list variable is missing".to_vec()),
                location,
            );
            return;
        };
        for (offset, value) in arguments.iter().skip(1).enumerate() {
            if place.indices.is_empty() {
                self.emit(
                    opcode::push_integer(i64::try_from(offset).unwrap_or(i64::MAX)),
                    location,
                );
            } else {
                for (position, index) in place.indices.iter().enumerate() {
                    self.lower_expression(index, location);
                    if position + 1 == place.indices.len() && offset != 0 {
                        self.emit(
                            opcode::push_integer(i64::try_from(offset).unwrap_or(i64::MAX)),
                            location,
                        );
                        self.emit(opcode::binary(3), location);
                    }
                }
            }
            self.lower_argument(value, location);
            self.emit(
                opcode::variable(
                    Opcode::StoreVariable,
                    key,
                    u16::try_from(place.indices.len().max(1)).unwrap_or(u16::MAX),
                    assign_tag(erabasic_ast::AssignOp::Assign),
                ),
                location,
            );
        }
    }
}
