use super::super::{
    BinaryOp, BytecodeType, CallTarget, CompilerDiagnostic, CompilerDiagnosticCode,
    EncodedInstruction, FunctionKind, HirArgument, HirCallArgument, HirExpr, HirExprKind,
    ImportKind, Opcode, SemanticType, SourceLocation, binary_tag, bytecode_type,
    compiler_variable_mutation_contract, opcode, unary_tag,
};
use super::Builder;

impl Builder<'_> {
    pub(in super::super) fn lower_argument(
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
    pub(in super::super) fn lower_expression(
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
                if matches!(
                    op,
                    erabasic_ast::UnaryOp::PreIncrement | erabasic_ast::UnaryOp::PreDecrement
                ) {
                    self.lower_increment_expression(
                        operand,
                        matches!(op, erabasic_ast::UnaryOp::PreIncrement),
                        false,
                        fallback,
                    );
                } else {
                    self.lower_expression(operand, fallback);
                    self.emit(opcode::unary(unary_tag(*op)), location);
                }
            }
            HirExprKind::Postfix { op, operand } => {
                self.lower_increment_expression(
                    operand,
                    matches!(op, erabasic_ast::PostfixOp::Increment),
                    true,
                    fallback,
                );
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
                            .to_vec()
                            .into();
                        self.lower_expression(right, fallback);
                        self.emit(opcode::unary(2), location);
                        if *op == BinaryOp::LogicalOr {
                            self.emit(opcode::unary(2), location);
                        }
                        self.code[end].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec()
                            .into();
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
                            .to_vec()
                            .into();
                        self.emit(
                            opcode::push_integer(i64::from(*op == BinaryOp::Nand)),
                            location,
                        );
                        self.code[end].payload = u32::try_from(self.code.len())
                            .unwrap_or(u32::MAX)
                            .to_le_bytes()
                            .to_vec()
                            .into();
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
                    .to_vec()
                    .into();
                self.lower_expression(else_expr, fallback);
                self.code[end_jump].payload = u32::try_from(self.code.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec()
                    .into();
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

    fn lower_increment_expression(
        &mut self,
        operand: &HirExpr,
        increment: bool,
        postfix: bool,
        fallback: SourceLocation,
    ) {
        let HirExprKind::Variable { place } = &operand.kind else {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                operand.location,
                "increment operand is not a mutable variable",
            ));
            self.lower_expression(operand, fallback);
            return;
        };
        let value_type = self.lower_argument(&HirArgument::Place(place.clone()), fallback);
        if value_type != BytecodeType::IntegerPlace {
            self.diagnostics.push(CompilerDiagnostic::at(
                CompilerDiagnosticCode::InvalidHir,
                operand.location,
                "increment operand is not an integer place",
            ));
        }
        let mode = match (increment, postfix) {
            (true, false) => 0,
            (false, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        self.emit(opcode::push_integer(mode), operand.location);
        self.emit_native_call(
            "__mutate_integer",
            &[BytecodeType::IntegerPlace, BytecodeType::Integer],
            Some(BytecodeType::Integer),
            compiler_variable_mutation_contract(),
            operand.location,
        );
    }
}
