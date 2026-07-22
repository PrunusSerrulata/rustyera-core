//! Stateful HIR-to-bytecode builder.

use super::{
    BTreeMap, BinaryOp, BytecodeType, CallTarget, CompilerDiagnostic, CompilerDiagnosticCode,
    ControlFlowKind, DataBlock, EncodedInstruction, ExecutionBinding, Function, FunctionImport,
    FunctionKind, HirArgument, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirStatementKind, HostImport, ImportKind, InstructionTarget, LineId,
    LoweringContext, NATIVE_ABI_VERSION, NativeImport, Opcode, SemanticType, SourceLocation,
    SymbolKey, TryListBlock, argument_place, assign_tag, binary_tag, bytecode_type,
    compiler_native_contract, extension_binding, formatted_constant, opcode, postfix_tag,
    runtime_import, unary_tag,
};

pub(super) struct Builder<'a> {
    pub(super) hir_function: &'a Function,
    pub(super) context: &'a LoweringContext<'a>,
    pub(super) code: Vec<EncodedInstruction>,
    pub(super) locations: Vec<SourceLocation>,
    pub(super) imports: Vec<FunctionImport>,
    pub(super) native_imports: BTreeMap<SymbolKey, NativeImport>,
    pub(super) host_imports: BTreeMap<SymbolKey, HostImport>,
    control_flow_by_line: BTreeMap<LineId, Vec<&'a erabasic_hir::ControlFlowEdge>>,
    pub(super) diagnostics: Vec<CompilerDiagnostic>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        hir_function: &'a Function,
        _function_key: SymbolKey,
        context: &'a LoweringContext<'a>,
    ) -> Self {
        let mut control_flow_by_line = BTreeMap::new();
        for edge in &hir_function.control_flow {
            control_flow_by_line
                .entry(edge.from)
                .or_insert_with(Vec::new)
                .push(edge);
        }
        Self {
            hir_function,
            context,
            code: Vec::new(),
            locations: Vec::new(),
            imports: Vec::new(),
            native_imports: BTreeMap::new(),
            host_imports: BTreeMap::new(),
            control_flow_by_line,
            diagnostics: Vec::new(),
        }
    }

    pub(super) fn emit(&mut self, instruction: EncodedInstruction, location: SourceLocation) {
        self.code.push(instruction);
        self.locations.push(location);
    }

    pub(super) fn lower_data_block(&mut self, block: &DataBlock<'_>) {
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
            && let Some(key) = self.context.variable_keys.get(&place.variable).copied()
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
                    .to_vec();
            }
        }
        let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
        for jump in end_jumps {
            self.code[jump].payload = end.to_le_bytes().to_vec();
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
                .to_vec();
        }
    }

    pub(super) fn lower_printdata_choice(
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

    pub(super) fn lower_strdata_choice(
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
        if let Some(key) = self.context.variable_keys.get(&place.variable).copied() {
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

    pub(super) fn lower_try_list(&mut self, block: &TryListBlock<'_>) {
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
                    .to_vec();
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
                payload
            };
        }
        let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
        for jump in end_jumps {
            self.code[jump].payload = end.to_le_bytes().to_vec();
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn lower_statement(
        &mut self,
        target: &InstructionTarget,
        arguments: &[HirArgument],
        line: LineId,
        location: SourceLocation,
    ) {
        let name = target.name();
        if let InstructionTarget::BuiltinMethod { return_type, .. } = target {
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
        if name == "FOR" {
            let Some(HirArgument::Place(counter)) = arguments.first() else {
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"FOR counter is not a place".to_vec()),
                    location,
                );
                return;
            };
            self.lower_argument(&HirArgument::Place(counter.clone()), location);
            for argument in arguments.iter().skip(1).take(2) {
                self.lower_argument(argument, location);
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
        if name == "NEXT" {
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
            self.emit(opcode::push_integer(1), location);
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
                    | "REPEAT"
                    | "REND"
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
                .get(&line)
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
        let mut parameter_types = Vec::new();
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
            .and_then(|variable| self.context.variable_keys.get(&variable.id))
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
        if arguments.is_empty() {
            // Bare RETURN exits without changing the legacy RESULT array. Runtime
            // controllers and SAVEINFO observe values assigned immediately before
            // RETURN, so synthesizing RESULT:0 = 0 here is externally visible.
            self.emit(opcode::return_value(false), location);
            return;
        }
        let result = self
            .context
            .program
            .variables
            .iter()
            .find(|variable| variable.name.eq_ignore_ascii_case("RESULT"))
            .and_then(|variable| self.context.variable_keys.get(&variable.id))
            .copied();
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

    pub(super) fn emit_default_method_value(&mut self, location: SourceLocation) {
        match self.hir_function.return_type {
            SemanticType::String => self.emit(opcode::push_string(""), location),
            SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                self.emit(opcode::push_integer(0), location);
            }
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
        let Some(key) = self.context.variable_keys.get(&place.variable).copied() else {
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

    #[allow(clippy::too_many_lines)]
    pub(super) fn lower_static_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let target_id = self
            .control_flow_by_line
            .get(&line)
            .into_iter()
            .flatten()
            .find(|edge| matches!(edge.kind, ControlFlowKind::Call | ControlFlowKind::Jump))
            .and_then(|edge| edge.function);
        let Some(target_id) = target_id else {
            if name.starts_with("TRY") {
                return;
            }
            self.diagnostics.push(CompilerDiagnostic::warning_at(
                CompilerDiagnosticCode::MissingImport,
                location,
                format!("{name} target does not resolve to a function"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"missing function".to_vec()),
                location,
            );
            return;
        };
        let Some(target) = self.context.function_keys.get(&target_id).copied() else {
            if name.starts_with("TRY") {
                // Reference TRY calls do not evaluate arguments when the target is absent.
                return;
            }
            self.diagnostics.push(CompilerDiagnostic::warning_at(
                CompilerDiagnosticCode::MissingImport,
                location,
                format!("{name} target does not resolve to a function"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"missing function".to_vec()),
                location,
            );
            return;
        };
        let target_function = self.context.functions_by_id.get(&target_id).copied();
        if let Some(function) = target_function {
            let method_call = name.ends_with('F');
            let valid_kind = if method_call {
                function.kind == FunctionKind::Method
            } else {
                function.kind != FunctionKind::Method
                    && (function.kind != FunctionKind::Event
                        || self
                            .context
                            .program
                            .call_compatibility
                            .allow_event_as_normal)
            };
            if !valid_kind {
                if name.starts_with("TRY") && method_call {
                    return;
                }
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    format!("{name} target has an incompatible function kind"),
                ));
                self.emit(
                    EncodedInstruction::new(Opcode::Trap, b"incompatible function kind".to_vec()),
                    location,
                );
                return;
            }
        }
        let reference_parameters = target_function
            .map(|function| {
                function
                    .parameters
                    .iter()
                    .map(|parameter| {
                        self.context
                            .program
                            .variables
                            .get(parameter.target.variable.0 as usize)
                            .is_some_and(|variable| variable.reference)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let supplied = arguments.iter().skip(1).collect::<Vec<_>>();
        let mut parameter_types = Vec::new();
        if let Some(function) = target_function {
            if supplied.len() > function.parameters.len() {
                self.diagnostics.push(CompilerDiagnostic::at(
                    CompilerDiagnosticCode::InvalidHir,
                    location,
                    format!("{name} supplies too many arguments"),
                ));
            }
            for (index, parameter) in function.parameters.iter().enumerate() {
                let argument = supplied.get(index).copied();
                if matches!(argument, None | Some(HirArgument::Omitted)) {
                    if let Some(default) = &parameter.default {
                        parameter_types.push(self.lower_expression(default, location));
                    } else if self
                        .context
                        .program
                        .call_compatibility
                        .allow_omitted_arguments
                    {
                        match parameter.target.value_type {
                            SemanticType::String => self.emit(opcode::push_string(""), location),
                            _ => self.emit(opcode::push_integer(0), location),
                        }
                        parameter_types.push(
                            bytecode_type(parameter.target.value_type)
                                .unwrap_or(BytecodeType::Integer),
                        );
                    } else {
                        self.diagnostics.push(CompilerDiagnostic::at(
                            CompilerDiagnosticCode::InvalidHir,
                            location,
                            format!("{name} omits required argument {}", index + 1),
                        ));
                        let value_type = parameter.target.value_type;
                        match value_type {
                            SemanticType::String => self.emit(opcode::push_string(""), location),
                            _ => self.emit(opcode::push_integer(0), location),
                        }
                        parameter_types
                            .push(bytecode_type(value_type).unwrap_or(BytecodeType::Integer));
                    }
                    continue;
                }
                let argument = argument.expect("handled missing argument above");
                if reference_parameters.get(index) == Some(&true)
                    && let HirArgument::Expression(expression) = argument
                    && let HirExprKind::Variable { place } = &expression.kind
                {
                    parameter_types
                        .push(self.lower_argument(&HirArgument::Place(place.clone()), location));
                } else {
                    let actual = self.lower_argument(argument, location);
                    let expected =
                        bytecode_type(parameter.target.value_type).unwrap_or(BytecodeType::Integer);
                    if actual == BytecodeType::Integer
                        && expected == BytecodeType::String
                        && self
                            .context
                            .program
                            .call_compatibility
                            .auto_convert_integer_to_string
                    {
                        self.emit(
                            EncodedInstruction::new(Opcode::ToString, Vec::new()),
                            location,
                        );
                        parameter_types.push(BytecodeType::String);
                    } else {
                        if actual != expected {
                            self.diagnostics.push(CompilerDiagnostic::at(
                                CompilerDiagnosticCode::InvalidHir,
                                location,
                                format!("{name} argument {} has an incompatible type", index + 1),
                            ));
                        }
                        parameter_types.push(actual);
                    }
                }
            }
        }
        let result = target_function.and_then(|function| bytecode_type(function.return_type));
        let import = self.add_import(ImportKind::Function, target);
        self.emit(
            opcode::call(
                Opcode::Call,
                import,
                u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
        if name.ends_with('F') && result.is_some() {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
        }
        if name.contains("JUMP") {
            self.emit(opcode::return_value(result.is_some()), location);
        }
    }

    pub(super) fn lower_dynamic_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let Some(target) = arguments.first() else {
            self.emit(
                EncodedInstruction::new(Opcode::Trap, b"missing dynamic target".to_vec()),
                location,
            );
            return;
        };
        let target_type = self.lower_argument(target, location);
        if target_type != BytecodeType::String {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                format!("{name} target is not a string"),
            ));
        }
        let allow_missing = name.starts_with("TRY");
        let has_catch = self
            .hir_function
            .control_flow
            .iter()
            .any(|edge| edge.from == line && edge.kind == ControlFlowKind::Branch);
        let resolve = self.code.len();
        let method = name.ends_with('F');
        self.emit(opcode::resolve_function(0, allow_missing, method), location);
        let parameter_types = arguments
            .iter()
            .skip(1)
            .map(|argument| {
                // The target signature is not known until ResolveFunction runs.
                // Preserve syntactic lvalues as places so InvokeDynamic can bind
                // them to REF parameters, or dereference them for value parameters.
                if let HirArgument::Expression(expression) = argument
                    && let HirExprKind::Variable { place } = &expression.kind
                {
                    self.lower_argument(&HirArgument::Place(place.clone()), location)
                } else {
                    self.lower_argument(argument, location)
                }
            })
            .collect::<Vec<_>>();
        self.emit(
            opcode::invoke_dynamic(
                u16::try_from(parameter_types.len()).unwrap_or(u16::MAX),
                name.contains("JUMP"),
            ),
            location,
        );
        if allow_missing {
            if has_catch {
                self.emit(opcode::push_integer(1), location);
            }
            let success = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            let missing = self.code.len();
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            if has_catch {
                self.emit(opcode::push_integer(0), location);
            }
            let end = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[resolve].payload = {
                let mut payload = u32::try_from(missing)
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                payload.push(1);
                payload.push(u8::from(method));
                payload
            };
            self.code[success].payload = end.to_le_bytes().to_vec();
        }
    }

    pub(super) fn lower_argument(
        &mut self,
        argument: &HirArgument,
        location: SourceLocation,
    ) -> BytecodeType {
        match argument {
            HirArgument::Expression(expression) => self.lower_expression(expression, location),
            HirArgument::MixedExpression { expression, .. } => {
                self.lower_expression(expression, location)
            }
            HirArgument::Place(place) => {
                for index in &place.indices {
                    self.lower_expression(index, location);
                }
                let value_type = match place.value_type {
                    SemanticType::String => BytecodeType::StringPlace,
                    SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                        BytecodeType::IntegerPlace
                    }
                };
                if let Some(key) = self.context.variable_keys.get(&place.variable).copied() {
                    self.emit(
                        opcode::variable(
                            Opcode::MakePlace,
                            key,
                            u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                            0,
                        ),
                        location,
                    );
                } else {
                    self.emit(
                        EncodedInstruction::new(Opcode::Trap, b"missing variable place".to_vec()),
                        location,
                    );
                }
                value_type
            }
            HirArgument::Formatted(formatted) => self.lower_formatted(formatted, location),
            HirArgument::Raw(value) => {
                self.emit(opcode::push_string(value), location);
                BytecodeType::String
            }
            HirArgument::Omitted => {
                // EraBasic can distinguish an omitted operand from an explicit zero. The
                // internal call ABI reserves i64::MIN until bytecode gains a first-class
                // omitted value.
                self.emit(opcode::push_integer(i64::MIN), location);
                BytecodeType::Integer
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn lower_expression(
        &mut self,
        expression: &HirExpr,
        fallback: SourceLocation,
    ) -> BytecodeType {
        let location = expression.location;
        let result = bytecode_type(expression.value_type).unwrap_or(BytecodeType::Integer);
        match &expression.kind {
            HirExprKind::Integer { value } => self.emit(opcode::push_integer(*value), location),
            HirExprKind::String { value } => self.emit(opcode::push_string(value), location),
            HirExprKind::Variable { place } => {
                for index in &place.indices {
                    self.lower_expression(index, fallback);
                }
                if let Some(key) = self.context.variable_keys.get(&place.variable).copied() {
                    self.emit(
                        opcode::variable(
                            Opcode::LoadVariable,
                            key,
                            u16::try_from(place.indices.len()).unwrap_or(u16::MAX),
                            0,
                        ),
                        location,
                    );
                } else {
                    self.emit(
                        EncodedInstruction::new(Opcode::Trap, b"missing variable".to_vec()),
                        location,
                    );
                }
            }
            HirExprKind::Call { target, arguments } => {
                let builtin = matches!(target, CallTarget::Builtin { .. });
                let parameter_types: Vec<_> = if matches!(target, CallTarget::User { .. }) {
                    Vec::new()
                } else {
                    arguments
                        .iter()
                        .filter_map(|argument| match argument {
                            HirCallArgument::Value(argument) => {
                                Some(self.lower_expression(argument, fallback))
                            }
                            HirCallArgument::Place(place) => Some(self.lower_argument(
                                &HirArgument::Place(place.clone()),
                                expression.location,
                            )),
                            HirCallArgument::Omitted if builtin => {
                                self.emit(opcode::push_integer(i64::MIN), expression.location);
                                Some(BytecodeType::Integer)
                            }
                            HirCallArgument::Omitted => None,
                        })
                        .collect()
                };
                match target {
                    CallTarget::User { function } => {
                        if let Some(key) = self.context.function_keys.get(function).copied()
                            && let Some(target_function) =
                                self.context.functions_by_id.get(function).copied()
                        {
                            if target_function.kind != FunctionKind::Method {
                                self.diagnostics.push(CompilerDiagnostic::at(
                                    CompilerDiagnosticCode::InvalidHir,
                                    location,
                                    format!(
                                        "expression target {} is not a method",
                                        target_function.name
                                    ),
                                ));
                            }
                            if arguments.len() > target_function.parameters.len() {
                                self.diagnostics.push(CompilerDiagnostic::at(
                                    CompilerDiagnosticCode::InvalidHir,
                                    location,
                                    format!(
                                        "method {} receives too many arguments",
                                        target_function.name
                                    ),
                                ));
                            }
                            let mut user_parameter_types = Vec::new();
                            for (index, parameter) in target_function.parameters.iter().enumerate()
                            {
                                let reference = self
                                    .context
                                    .program
                                    .variables
                                    .get(parameter.target.variable.0 as usize)
                                    .is_some_and(|variable| variable.reference);
                                let expected = if reference {
                                    match parameter.target.value_type {
                                        SemanticType::String => BytecodeType::StringPlace,
                                        _ => BytecodeType::IntegerPlace,
                                    }
                                } else {
                                    bytecode_type(parameter.target.value_type)
                                        .unwrap_or(BytecodeType::Integer)
                                };
                                let actual = match arguments.get(index) {
                                    Some(HirCallArgument::Value(value))
                                        if reference
                                            && matches!(
                                                value.kind,
                                                HirExprKind::Variable { .. }
                                            ) =>
                                    {
                                        let HirExprKind::Variable { place } = &value.kind else {
                                            unreachable!("guard checked variable expression")
                                        };
                                        self.lower_argument(
                                            &HirArgument::Place(place.clone()),
                                            fallback,
                                        )
                                    }
                                    Some(HirCallArgument::Value(value)) => {
                                        self.lower_expression(value, fallback)
                                    }
                                    Some(HirCallArgument::Place(place)) => self.lower_argument(
                                        &HirArgument::Place(place.clone()),
                                        location,
                                    ),
                                    Some(HirCallArgument::Omitted) | None => {
                                        if let Some(default) = &parameter.default {
                                            self.lower_expression(default, fallback)
                                        } else if !reference
                                            && self
                                                .context
                                                .program
                                                .call_compatibility
                                                .allow_omitted_arguments
                                        {
                                            match expected {
                                                BytecodeType::String => {
                                                    self.emit(opcode::push_string(""), location);
                                                }
                                                _ => self.emit(opcode::push_integer(0), location),
                                            }
                                            expected
                                        } else {
                                            self.diagnostics.push(CompilerDiagnostic::at(
                                                CompilerDiagnosticCode::InvalidHir,
                                                location,
                                                format!(
                                                    "method {} omits required argument {}",
                                                    target_function.name,
                                                    index + 1
                                                ),
                                            ));
                                            match expected {
                                                BytecodeType::String => {
                                                    self.emit(opcode::push_string(""), location);
                                                }
                                                _ => self.emit(opcode::push_integer(0), location),
                                            }
                                            expected
                                        }
                                    }
                                };
                                if actual == BytecodeType::Integer
                                    && expected == BytecodeType::String
                                    && self
                                        .context
                                        .program
                                        .call_compatibility
                                        .auto_convert_integer_to_string
                                {
                                    self.emit(
                                        EncodedInstruction::new(Opcode::ToString, Vec::new()),
                                        location,
                                    );
                                    user_parameter_types.push(BytecodeType::String);
                                } else {
                                    if actual != expected {
                                        self.diagnostics.push(CompilerDiagnostic::at(
                                            CompilerDiagnosticCode::InvalidHir,
                                            location,
                                            format!(
                                                "method {} argument {} has an incompatible type",
                                                target_function.name,
                                                index + 1
                                            ),
                                        ));
                                    }
                                    user_parameter_types.push(actual);
                                }
                            }
                            let import = self.add_import(ImportKind::Function, key);
                            self.emit(
                                opcode::call(
                                    Opcode::Call,
                                    import,
                                    u16::try_from(user_parameter_types.len()).unwrap_or(u16::MAX),
                                    Some(result),
                                ),
                                location,
                            );
                        } else {
                            self.emit(
                                EncodedInstruction::new(Opcode::Trap, b"missing function".to_vec()),
                                location,
                            );
                        }
                    }
                    CallTarget::Builtin { name } => {
                        self.emit_runtime_call(
                            name,
                            &parameter_types,
                            Some(result),
                            false,
                            location,
                        );
                    }
                    CallTarget::Extension { name } => {
                        self.emit_runtime_call(
                            name,
                            &parameter_types,
                            Some(result),
                            true,
                            location,
                        );
                    }
                    CallTarget::Unresolved { name } => {
                        self.diagnostics.push(CompilerDiagnostic::at(
                            CompilerDiagnosticCode::MissingImport,
                            location,
                            format!("function {name} is unresolved"),
                        ));
                        self.emit(
                            EncodedInstruction::new(Opcode::Trap, b"unresolved call".to_vec()),
                            location,
                        );
                    }
                }
            }
            HirExprKind::Unary { op, operand } => {
                self.lower_expression(operand, fallback);
                self.emit(opcode::unary(unary_tag(*op)), location);
            }
            HirExprKind::Postfix { op, operand } => {
                self.lower_expression(operand, fallback);
                self.emit(opcode::unary(postfix_tag(*op)), location);
            }
            HirExprKind::Binary { op, left, right } => {
                if matches!(
                    op,
                    BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::Nand | BinaryOp::Nor
                ) {
                    self.lower_expression(left, fallback);
                    let branch = self.code.len();
                    self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
                    if matches!(op, BinaryOp::LogicalOr | BinaryOp::Nor) {
                        self.emit(
                            opcode::push_integer(i64::from(*op == BinaryOp::LogicalOr)),
                            location,
                        );
                        let end = self.code.len();
                        self.emit(opcode::jump(Opcode::Jump, 0), location);
                        self.code[branch].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec();
                        self.lower_expression(right, fallback);
                        self.emit(opcode::unary(2), location);
                        if *op == BinaryOp::LogicalOr {
                            self.emit(opcode::unary(2), location);
                        }
                        self.code[end].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec();
                    } else {
                        self.lower_expression(right, fallback);
                        self.emit(opcode::unary(2), location);
                        if *op == BinaryOp::LogicalAnd {
                            self.emit(opcode::unary(2), location);
                        }
                        let end = self.code.len();
                        self.emit(opcode::jump(Opcode::Jump, 0), location);
                        self.code[branch].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec();
                        self.emit(
                            opcode::push_integer(i64::from(*op == BinaryOp::Nand)),
                            location,
                        );
                        self.code[end].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec();
                    }
                } else {
                    self.lower_expression(left, fallback);
                    self.lower_expression(right, fallback);
                    self.emit(opcode::binary(binary_tag(*op)), location);
                }
            }
            HirExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.lower_expression(condition, fallback);
                let false_jump = self.code.len();
                self.emit(opcode::jump(Opcode::JumpIfFalse, 0), location);
                self.lower_expression(then_expr, fallback);
                let end_jump = self.code.len();
                self.emit(opcode::jump(Opcode::Jump, 0), location);
                self.code[false_jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                self.lower_expression(else_expr, fallback);
                self.code[end_jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
            }
            HirExprKind::Formatted { value } => {
                self.lower_formatted(value, fallback);
            }
            HirExprKind::Error => self.emit(
                EncodedInstruction::new(Opcode::Trap, b"invalid expression".to_vec()),
                fallback,
            ),
        }
        result
    }

    pub(super) fn lower_formatted(
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
                    integer,
                    location,
                    ..
                } => {
                    let mut parameters = vec![self.lower_expression(expression, fallback)];
                    if let Some(width) = width {
                        parameters.push(self.lower_expression(width, fallback));
                    }
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
                    self.code[false_jump].payload = u32::try_from(self.code.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes()
                        .to_vec();
                    if let Some(else_value) = else_value {
                        self.lower_formatted(else_value, fallback);
                    } else {
                        self.emit(opcode::push_string(""), *location);
                    }
                    self.code[end_jump].payload = u32::try_from(self.code.len())
                        .unwrap_or(u32::MAX)
                        .to_le_bytes()
                        .to_vec();
                }
                HirFormPart::Triple { symbol, location } => {
                    self.emit(opcode::push_string(&symbol.to_string()), *location);
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

    pub(super) fn emit_runtime_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        extension: bool,
        location: SourceLocation,
    ) {
        let classification = if extension {
            self.context
                .host_registry
                .classification(name)
                .cloned()
                .unwrap_or_else(|| ExecutionBinding::Host(extension_binding(name)))
        } else {
            self.context
                .host_registry
                .classification(name)
                .cloned()
                .unwrap_or(ExecutionBinding::Unsupported {
                    reason: "the callable has no execution catalog entry".into(),
                })
        };
        if let ExecutionBinding::Host(binding) = classification {
            if binding.contract.portability
                == erabasic_bytecode::OperationPortability::FrontendObservation
            {
                self.diagnostics.push(CompilerDiagnostic::notice_at(
                    CompilerDiagnosticCode::FrontendObservation,
                    location,
                    format!(
                        "{name} observes the authoritative frontend environment and may vary across clients"
                    ),
                ));
            }
            let import = runtime_import(
                &binding.namespace,
                &binding.name,
                binding.abi_version,
                parameters,
                result,
            );
            let key = import.key;
            self.host_imports.entry(key).or_insert(HostImport {
                import,
                effect: binding.effect,
                capability: binding.capability,
                snapshot_capability: binding.snapshot_capability,
                contract: binding.contract,
            });
            let index = self.add_import(ImportKind::Host, key);
            self.emit(
                opcode::call(
                    Opcode::CallHost,
                    index,
                    u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                    result,
                ),
                location,
            );
        } else if let ExecutionBinding::Native(contract) = classification {
            self.emit_native_call(name, parameters, result, contract, location);
        } else if let ExecutionBinding::Unsupported { reason } = classification {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::UnsupportedConstruct,
                location,
                format!("{name} is unsupported: {reason}"),
            ));
            self.emit(
                EncodedInstruction::new(Opcode::Trap, format!("unsupported {name}").into_bytes()),
                location,
            );
        }
    }

    pub(super) fn emit_native_call(
        &mut self,
        name: &str,
        parameters: &[BytecodeType],
        result: Option<BytecodeType>,
        contract: erabasic_bytecode::OperationContract,
        location: SourceLocation,
    ) {
        let import = runtime_import(
            "rustyera.vm",
            &name.to_ascii_lowercase(),
            NATIVE_ABI_VERSION,
            parameters,
            result,
        );
        let key = import.key;
        self.native_imports.entry(key).or_insert(NativeImport {
            import,
            effect: contract.effect(),
            contract,
        });
        let index = self.add_import(ImportKind::Native, key);
        self.emit(
            opcode::call(
                Opcode::CallNative,
                index,
                u16::try_from(parameters.len()).unwrap_or(u16::MAX),
                result,
            ),
            location,
        );
    }

    pub(super) fn add_import(&mut self, kind: ImportKind, key: SymbolKey) -> u32 {
        if let Some(index) = self
            .imports
            .iter()
            .position(|import| import.kind == kind && import.key == key)
        {
            return u32::try_from(index).unwrap_or(u32::MAX);
        }
        let index = self.imports.len();
        self.imports.push(FunctionImport { kind, key });
        u32::try_from(index).unwrap_or(u32::MAX)
    }
}
