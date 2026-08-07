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

    pub(in super::super) fn lower_dynamic_call(
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
                payload.into()
            };
            self.code[success].payload = end.to_le_bytes().to_vec().into();
        }
    }
}
