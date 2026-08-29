use super::super::{
    BytecodeType, CompilerDiagnostic, CompilerDiagnosticCode, ControlFlowKind, EncodedInstruction,
    FunctionKind, HirArgument, HirExprKind, ImportKind, LineId, Opcode, SemanticType,
    SourceLocation, bytecode_type, opcode,
};
use super::Builder;

impl Builder<'_> {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn lower_static_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let target_id = self
            .control_flow_by_line
            .get(line.0)
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
        let Some(target) = self.context.function_keys.get(target_id.0).copied() else {
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
        let target_function = self.context.functions_by_id.get(target_id.0).copied();
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
        if let Some(function) = target_function
            && function.parameters.iter().any(|parameter| {
                self.context
                    .program
                    .variables
                    .get(parameter.target.variable.0 as usize)
                    .is_some_and(|variable| variable.reference)
            })
        {
            self.reject_excess_user_arguments(
                arguments.len().saturating_sub(1),
                function.parameters.len(),
                location,
                || format!("{name} supplies too many arguments"),
            );
            let actuals =
                Self::method_statement_arguments(arguments.get(1..).unwrap_or_default(), location);
            let mode = if name.ends_with('F') {
                erabasic_bytecode::UserCallMode::MethodDiscard
            } else if name.contains("JUMP") {
                erabasic_bytecode::UserCallMode::JumpProcedure
            } else {
                erabasic_bytecode::UserCallMode::Procedure
            };
            self.emit(opcode::push_string(&function.name), location);
            self.lower_user_call_actuals(&actuals, mode, false, location);
            return;
        }
        let mut parameter_types =
            Vec::with_capacity(target_function.map_or(0, |function| function.parameters.len()));
        if let Some(function) = target_function {
            self.reject_excess_user_arguments(
                arguments.len().saturating_sub(1),
                function.parameters.len(),
                location,
                || format!("{name} supplies too many arguments"),
            );
            for (index, parameter) in function.parameters.iter().enumerate() {
                let argument = arguments.get(index + 1);
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
                let reference = self
                    .context
                    .program
                    .variables
                    .get(parameter.target.variable.0 as usize)
                    .is_some_and(|variable| variable.reference);
                if reference
                    && let HirArgument::Expression(expression) = argument
                    && let HirExprKind::Variable { place } = &expression.kind
                {
                    parameter_types.push(self.lower_place(place, location));
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

    pub(super) fn reject_excess_user_arguments(
        &mut self,
        supplied: usize,
        formal: usize,
        location: SourceLocation,
        message: impl FnOnce() -> String,
    ) {
        let decision = self
            .context
            .program
            .call_compatibility
            .user_argument_policy
            .decide(supplied, formal);
        if decision.is_rejected() {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                location,
                message(),
            ));
        }
        // The analyzer owns the snake load warning, so warm bytecode-cache reuse
        // cannot swallow it. The following loops still lower only formal slots.
    }

    pub(in super::super) fn lower_dynamic_call(
        &mut self,
        arguments: &[HirArgument],
        line: LineId,
        name: &str,
        location: SourceLocation,
    ) {
        let Some(target) = arguments.first() else {
            self.invalid_user_call("missing dynamic target", location);
            return;
        };
        if self.lower_argument(target, location) != BytecodeType::String {
            self.invalid_user_call("dynamic target is not a string", location);
            return;
        }
        let allow_missing = name.starts_with("TRY");
        let has_catch = self
            .hir_function
            .control_flow
            .iter()
            .any(|edge| edge.from == line && edge.kind == ControlFlowKind::Branch);
        let mode = if name.ends_with('F') {
            erabasic_bytecode::UserCallMode::MethodDiscard
        } else if name.contains("JUMP") {
            erabasic_bytecode::UserCallMode::JumpProcedure
        } else {
            erabasic_bytecode::UserCallMode::Procedure
        };
        let actuals = Self::method_statement_arguments(&arguments[1..], location);
        let Some((resolve, mut spec)) =
            self.lower_user_call_actuals(&actuals, mode, allow_missing, location)
        else {
            return;
        };
        if allow_missing {
            if has_catch {
                self.emit(opcode::push_integer(1), location);
            }
            let success = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            spec.missing_target = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[resolve] = opcode::resolve_user_call(&spec);
            self.emit(
                opcode::abandon_user_call(u32::try_from(resolve).unwrap_or(u32::MAX)),
                location,
            );
            if has_catch {
                self.emit(opcode::push_integer(0), location);
            }
            self.patch_jump(success, self.code.len());
        }
    }

    pub(in super::super) fn lower_call_text(
        &mut self,
        arguments: &[HirArgument],
        name: &str,
        location: SourceLocation,
    ) {
        use erabasic_bytecode::{CallTextMode, CallTextSpec};
        let mode = match name {
            "CALLSTR" => CallTextMode::Call,
            "JUMPSTR" => CallTextMode::Jump,
            "TRYCALLSTR" => CallTextMode::TryCall,
            "TRYJUMPSTR" => CallTextMode::TryJump,
            "TRYCCALLSTR" => CallTextMode::CatchCall,
            "TRYCJUMPSTR" => CallTextMode::CatchJump,
            _ => unreachable!("only complete call-text instructions are dispatched here"),
        };
        let [HirArgument::Expression(expression)] = arguments else {
            self.invalid_user_call("call-text requires exactly one string expression", location);
            return;
        };
        if self.lower_expression(expression, location) != BytecodeType::String {
            self.invalid_user_call("call-text source is not a string", location);
            return;
        }
        let invoke = self.code.len();
        let mut spec = CallTextSpec {
            mode,
            catch_target: 0,
        };
        self.emit(opcode::invoke_call_text(spec), location);
        if mode.has_catch() {
            // Both VM successors leave the same stack; the ordinary TRY planner
            // consumes this locally materialized status. Blank text takes success.
            self.emit(opcode::push_integer(1), location);
            let success = self.code.len();
            self.emit(opcode::jump(Opcode::Jump, 0), location);
            spec.catch_target = u32::try_from(self.code.len()).unwrap_or(u32::MAX);
            self.code[invoke] = opcode::invoke_call_text(spec);
            self.emit(opcode::push_integer(0), location);
            self.patch_jump(success, self.code.len());
        }
    }
}
