//! Stateful HIR-to-bytecode builder.

use super::{
    BTreeMap, BytecodeType, CallTarget, CompilerDiagnostic, CompilerDiagnosticCode,
    ControlFlowKind, DataBlock, EncodedInstruction, ExecutionBinding, Function, FunctionImport,
    FunctionKind, HirArgument, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirStatementKind, HostImport, ImportKind, InstructionTarget, LineId,
    LoweringContext, NATIVE_ABI_VERSION, NativeImport, Opcode, SemanticType, SourceLocation,
    SymbolKey, TryListBlock, argument_place, binary_tag, bytecode_type, compiler_native_contract,
    extension_binding, formatted_constant, opcode, postfix_tag, runtime_import, unary_tag,
};

pub(super) struct Builder<'a> {
    pub(super) hir_function: &'a Function,
    pub(super) context: &'a LoweringContext<'a>,
    pub(super) code: Vec<EncodedInstruction>,
    pub(super) locations: Vec<SourceLocation>,
    pub(super) imports: Vec<FunctionImport>,
    pub(super) native_imports: BTreeMap<SymbolKey, NativeImport>,
    pub(super) host_imports: BTreeMap<SymbolKey, HostImport>,
    pub(super) diagnostics: Vec<CompilerDiagnostic>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        hir_function: &'a Function,
        _function_key: SymbolKey,
        context: &'a LoweringContext<'a>,
    ) -> Self {
        Self {
            hir_function,
            context,
            code: Vec::new(),
            locations: Vec::new(),
            imports: Vec::new(),
            native_imports: BTreeMap::new(),
            host_imports: BTreeMap::new(),
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
        let Some(place) = argument_place(destination) else {
            self.emit(EncodedInstruction::new(Opcode::Pop, Vec::new()), location);
            return;
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
                    | "FOR"
                    | "NEXT"
                    | "DO"
                    | "LOOP"
                    | "SELECTCASE"
                    | "CASE"
                    | "CASEELSE"
                    | "ENDSELECT"
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
            let direct_condition = matches!(name, "IF" | "ELSEIF" | "WHILE");
            let has_branch = self
                .hir_function
                .control_flow
                .iter()
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
        if matches!(name, "RETURN" | "RETURNF" | "RETURNFORM") {
            for argument in arguments {
                self.lower_argument(argument, location);
            }
            self.emit(opcode::return_value(!arguments.is_empty()), location);
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
                HirArgument::Formatted(_) | HirArgument::Place(_) | HirArgument::Omitted => None,
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
        let parameter_types: Vec<_> = arguments
            .iter()
            .map(|argument| self.lower_argument(argument, location))
            .collect();
        let extension = matches!(target, InstructionTarget::Extension(_));
        self.emit_runtime_call(name, &parameter_types, None, extension, location);
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn lower_static_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let target = self
            .hir_function
            .control_flow
            .iter()
            .find(|edge| {
                edge.from == line
                    && matches!(edge.kind, ControlFlowKind::Call | ControlFlowKind::Jump)
            })
            .and_then(|edge| edge.function);
        let Some(target) = target.and_then(|id| self.context.function_keys.get(&id).copied())
        else {
            if name.starts_with("TRY") {
                // Reference TRY calls do not evaluate arguments when the target is absent.
                return;
            }
            self.diagnostics.push(CompilerDiagnostic::at(
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
        let target_function = self
            .context
            .program
            .functions
            .iter()
            .find(|function| self.context.function_keys.get(&function.id) == Some(&target));
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
            .map(|argument| self.lower_argument(argument, location))
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
                            && let Some(target_function) = self
                                .context
                                .program
                                .functions
                                .iter()
                                .find(|candidate| candidate.id == *function)
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
                self.lower_expression(left, fallback);
                self.lower_expression(right, fallback);
                self.emit(opcode::binary(binary_tag(*op)), location);
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
            ExecutionBinding::Host(extension_binding(name))
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
