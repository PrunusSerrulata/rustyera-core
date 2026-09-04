use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog, identifiers::identifier_key, symbols::Symbols,
};
use erabasic_ast::{BinaryOp, Expr, ExprKind, FormPart, FormattedString, PostfixOp, UnaryOp};
use erabasic_hir::{
    CallTarget, ConstantValue, FunctionId, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirPlace, SemanticType, SourceId, SourceLocation,
};

mod calls;
mod value;

use value::{fold_binary, normalize_colon_indices};

pub(crate) use crate::index_resolver::IndexResolver;

pub(crate) struct ExpressionAnalyzer<'a> {
    pub symbols: &'a Symbols,
    pub catalog: &'a Catalog,
    pub options: &'a AnalyzerOptions,
    pub function: FunctionId,
    pub source: SourceId,
    pub path: &'a str,
    pub text: &'a str,
    pub diagnostics: &'a mut Vec<AnalyzerDiagnostic>,
    pub index_resolver: &'a IndexResolver,
}

impl ExpressionAnalyzer<'_> {
    pub fn analyze(&mut self, expression: &Expr) -> HirExpr {
        let location = SourceLocation::new(self.source, expression.span);
        match &expression.kind {
            ExprKind::Integer(value) => HirExpr {
                kind: HirExprKind::Integer { value: *value },
                value_type: SemanticType::Integer,
                constant: Some(ConstantValue::Integer(*value)),
                location,
            },
            ExprKind::String(value) => HirExpr {
                kind: HirExprKind::String {
                    value: value.clone(),
                },
                value_type: SemanticType::String,
                constant: Some(ConstantValue::String(value.clone())),
                location,
            },
            ExprKind::Identifier(name) => self.analyze_identifier(name, &[], location),
            ExprKind::Variable { name, indices } => {
                self.analyze_identifier(name, indices, location)
            }
            ExprKind::Call { name, args } => self.analyze_call(name, args, location),
            ExprKind::Unary { op, operand } => self.analyze_unary(*op, operand, location),
            ExprKind::Postfix { op, operand } => self.analyze_postfix(*op, operand, location),
            ExprKind::Binary { op, left, right } => self.analyze_binary(*op, left, right, location),
            ExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => self.analyze_ternary(condition, then_expr, else_expr, location),
            ExprKind::Formatted(formatted) => {
                let value = self.analyze_formatted(formatted);
                HirExpr {
                    kind: HirExprKind::Formatted { value },
                    value_type: SemanticType::String,
                    constant: None,
                    location,
                }
            }
            ExprKind::Group(inner) => {
                let mut result = self.analyze(inner);
                result.location = location;
                result
            }
            ExprKind::Error => self.error_expression(location),
        }
    }

    pub fn analyze_formatted(&mut self, formatted: &FormattedString) -> HirFormattedString {
        let mut parts = Vec::new();
        for part in &formatted.parts {
            match part {
                FormPart::Text(value) => parts.push(HirFormPart::Text {
                    value: value.clone(),
                }),
                FormPart::StringInterpolation {
                    expression,
                    width,
                    alignment,
                    span,
                }
                | FormPart::IntegerInterpolation {
                    expression,
                    width,
                    alignment,
                    span,
                } => {
                    let integer = matches!(part, FormPart::IntegerInterpolation { .. });
                    let expression = self.analyze(expression);
                    let expected = if integer {
                        SemanticType::Integer
                    } else {
                        SemanticType::String
                    };
                    self.expect_type(&expression, expected, "formatted interpolation");
                    let width = width.as_ref().map(|width| {
                        let width = self.analyze(width);
                        self.expect_type(&width, SemanticType::Integer, "formatted width");
                        Box::new(width)
                    });
                    parts.push(HirFormPart::Interpolation {
                        expression: Box::new(expression),
                        width,
                        alignment: *alignment,
                        integer,
                        location: SourceLocation::new(self.source, *span),
                    });
                }
                FormPart::Conditional {
                    condition,
                    then_value,
                    else_value,
                    span,
                } => {
                    let condition = self.analyze(condition);
                    self.expect_type(&condition, SemanticType::Integer, "FORM condition");
                    parts.push(HirFormPart::Conditional {
                        condition: Box::new(condition),
                        then_value: Box::new(self.analyze_formatted(then_value)),
                        else_value: else_value
                            .as_deref()
                            .map(|value| Box::new(self.analyze_formatted(value))),
                        location: SourceLocation::new(self.source, *span),
                    });
                }
                FormPart::Triple { symbol, span } => parts.push(HirFormPart::Triple {
                    symbol: *symbol,
                    location: SourceLocation::new(self.source, *span),
                }),
            }
        }
        HirFormattedString {
            parts,
            location: SourceLocation::new(self.source, formatted.span),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn analyze_identifier(
        &mut self,
        name: &str,
        indices: &[Expr],
        location: SourceLocation,
    ) -> HirExpr {
        if name.eq_ignore_ascii_case("RAND") {
            // RAND predates expression-function syntax and remains exposed as a
            // pseudo variable (`RAND:max`) by Emuera. Lower both spellings to the
            // same native call so the zero-length schema placeholder is never read.
            let arguments = indices.iter().cloned().map(Some).collect::<Vec<_>>();
            return self.analyze_call(name, &arguments, location);
        }
        if indices.is_empty()
            && let Some(value) = self.index_resolver.resolve_rename(name)
        {
            return HirExpr {
                kind: HirExprKind::Integer { value },
                value_type: SemanticType::Integer,
                constant: Some(ConstantValue::Integer(value)),
                location,
            };
        }
        let Some(variable) = self.symbols.resolve_variable(self.function, name) else {
            if self.catalog.functions.contains_key(&self.key(name)) {
                return self.analyze_call(name, &[], location);
            }
            self.diagnostic(
                AnalyzerDiagnosticCode::UnknownIdentifier,
                location,
                format!("unknown identifier {name}"),
            );
            return self.error_expression(location);
        };
        let maximum_indices = variable.dimensions.len()
            + usize::from(variable.storage == erabasic_data::StorageScope::Character);
        let indices = normalize_colon_indices(indices, maximum_indices);
        let explicit_character = variable.storage == erabasic_data::StorageScope::Character
            && indices.len() > variable.dimensions.len();
        let indices: Vec<_> = indices
            .iter()
            .enumerate()
            .map(|(dimension, index)| {
                let data_dimension = dimension.saturating_sub(usize::from(explicit_character));
                let index_location = SourceLocation::new(self.source, index.span);
                let index = if let ExprKind::Identifier(index_name) = &index.kind
                    && (!explicit_character || dimension > 0)
                    && self.index_resolver.has_table(name, data_dimension)
                {
                    if let Some(value) =
                        self.index_resolver
                            .resolve(name, data_dimension, index_name)
                    {
                        return HirExpr {
                            kind: HirExprKind::Integer { value },
                            value_type: SemanticType::Integer,
                            constant: Some(ConstantValue::Integer(value)),
                            location: index_location,
                        };
                    }
                    if self
                        .symbols
                        .resolve_variable(self.function, index_name)
                        .is_some()
                        || self.catalog.functions.contains_key(&self.key(index_name))
                        || self.index_resolver.resolve_rename(index_name).is_some()
                    {
                        self.analyze(index)
                    } else {
                        // The reference defers reduction of uncalled function bodies. Rust
                        // compiles dynamic-call candidates eagerly, so preserve an unresolved
                        // symbolic index as the equivalent GETNUM lookup instead of rejecting
                        // an otherwise unneeded function during project startup.
                        self.diagnostics.push(AnalyzerDiagnostic::at(
                            AnalyzerDiagnosticCode::DeferredIndex,
                            AnalyzerDiagnosticSeverity::Warning,
                            1,
                            self.source,
                            self.path,
                            self.text,
                            index.span,
                            format!("named index {index_name} for {name} is deferred to runtime"),
                        ));
                        HirExpr {
                            kind: HirExprKind::String {
                                value: index_name.clone(),
                            },
                            value_type: SemanticType::String,
                            constant: Some(ConstantValue::String(index_name.clone())),
                            location: index_location,
                        }
                    }
                } else {
                    self.analyze(index)
                };
                if index.value_type == SemanticType::String
                    && self.index_resolver.has_table(name, data_dimension)
                {
                    // Emuera resolves a runtime string index through the same
                    // CSV name table exposed by GETNUM. Represent that lookup
                    // explicitly so bytecode indices remain integer-typed.
                    let array = HirPlace {
                        variable: variable.id,
                        indices: Vec::new(),
                        value_type: variable.value_type,
                        mutable: variable.mutable,
                        location,
                    };
                    return HirExpr {
                        kind: HirExprKind::Call {
                            target: CallTarget::Builtin {
                                name: "__INDEXBYNAME".into(),
                            },
                            arguments: vec![
                                HirCallArgument::Place(array),
                                HirCallArgument::Value(index),
                                HirCallArgument::Value(HirExpr {
                                    kind: HirExprKind::Integer {
                                        value: i64::try_from(data_dimension).unwrap_or(i64::MAX),
                                    },
                                    value_type: SemanticType::Integer,
                                    constant: Some(ConstantValue::Integer(
                                        i64::try_from(data_dimension).unwrap_or(i64::MAX),
                                    )),
                                    location: index_location,
                                }),
                            ],
                        },
                        value_type: SemanticType::Integer,
                        constant: None,
                        location: index_location,
                    };
                }
                index
            })
            .collect();
        for index in &indices {
            self.expect_type(index, SemanticType::Integer, "array index");
        }
        if indices.len() > maximum_indices {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidDimension,
                location,
                format!(
                    "{} accepts at most {} indices, but {} were provided",
                    variable.name,
                    maximum_indices,
                    indices.len()
                ),
            );
        }
        let place = HirPlace {
            variable: variable.id,
            indices,
            value_type: variable.value_type,
            mutable: variable.mutable,
            location,
        };
        let constant = if variable.storage == erabasic_data::StorageScope::Constant {
            variable.initial_values.first().cloned()
        } else {
            None
        };
        HirExpr {
            value_type: variable.value_type,
            constant,
            kind: HirExprKind::Variable { place },
            location,
        }
    }

    fn analyze_unary(&mut self, op: UnaryOp, operand: &Expr, location: SourceLocation) -> HirExpr {
        let operand = self.analyze(operand);
        self.expect_type(&operand, SemanticType::Integer, "unary operand");
        if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) {
            self.expect_mutable_place(&operand, "increment operand");
        }
        let constant = match (&operand.constant, op) {
            (Some(ConstantValue::Integer(value)), UnaryOp::Plus) => {
                Some(ConstantValue::Integer(*value))
            }
            (Some(ConstantValue::Integer(value)), UnaryOp::Minus) => self
                .options
                .compatibility
                .integer_arithmetic_policy()
                .evaluate(erabasic_compat::IntegerOperation::Negate, *value, None)
                .ok()
                .filter(|result| result.warning.is_none())
                .map(|result| ConstantValue::Integer(result.value)),
            (Some(ConstantValue::Integer(value)), UnaryOp::LogicalNot) => {
                Some(ConstantValue::Integer(i64::from(*value == 0)))
            }
            (Some(ConstantValue::Integer(value)), UnaryOp::BitNot) => {
                Some(ConstantValue::Integer(!value))
            }
            _ => None,
        };
        HirExpr {
            kind: HirExprKind::Unary {
                op,
                operand: Box::new(operand),
            },
            value_type: SemanticType::Integer,
            constant,
            location,
        }
    }

    fn analyze_postfix(
        &mut self,
        op: PostfixOp,
        operand: &Expr,
        location: SourceLocation,
    ) -> HirExpr {
        let operand = self.analyze(operand);
        self.expect_type(&operand, SemanticType::Integer, "postfix operand");
        self.expect_mutable_place(&operand, "postfix operand");
        HirExpr {
            kind: HirExprKind::Postfix {
                op,
                operand: Box::new(operand),
            },
            value_type: SemanticType::Integer,
            constant: None,
            location,
        }
    }

    fn analyze_binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        location: SourceLocation,
    ) -> HirExpr {
        let left = self.analyze(left);
        let right = self.analyze(right);
        let comparison = matches!(
            op,
            BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
        );
        let string_add = op == BinaryOp::Add
            && left.value_type == SemanticType::String
            && right.value_type == SemanticType::String;
        let string_repeat = op == BinaryOp::Multiply
            && matches!(
                (left.value_type, right.value_type),
                (SemanticType::String, SemanticType::Integer)
                    | (SemanticType::Integer, SemanticType::String)
            );
        if comparison {
            if left.value_type != right.value_type
                && !left.value_type.eq(&SemanticType::Error)
                && !right.value_type.eq(&SemanticType::Error)
            {
                self.diagnostic(
                    AnalyzerDiagnosticCode::TypeMismatch,
                    location,
                    "comparison operands must have the same type",
                );
            }
        } else if !string_add && !string_repeat {
            self.expect_type(&left, SemanticType::Integer, "binary left operand");
            self.expect_type(&right, SemanticType::Integer, "binary right operand");
        }
        let value_type = if comparison {
            SemanticType::Integer
        } else if string_add || string_repeat {
            SemanticType::String
        } else if left.value_type == SemanticType::Error || right.value_type == SemanticType::Error
        {
            SemanticType::Error
        } else {
            SemanticType::Integer
        };
        let constant = fold_binary(
            op,
            left.constant.as_ref(),
            right.constant.as_ref(),
            self.options.compatibility.integer_arithmetic_policy(),
        );
        HirExpr {
            kind: HirExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            value_type,
            constant,
            location,
        }
    }

    fn analyze_ternary(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        location: SourceLocation,
    ) -> HirExpr {
        let condition = self.analyze(condition);
        let then_expr = self.analyze(then_expr);
        let else_expr = self.analyze(else_expr);
        self.expect_type(&condition, SemanticType::Integer, "ternary condition");
        let value_type = if then_expr.value_type == else_expr.value_type {
            then_expr.value_type
        } else {
            self.diagnostic(
                AnalyzerDiagnosticCode::TypeMismatch,
                location,
                "ternary branches must have the same type",
            );
            SemanticType::Error
        };
        let constant = match condition.constant {
            Some(ConstantValue::Integer(value)) if value != 0 => then_expr.constant.clone(),
            Some(ConstantValue::Integer(_)) => else_expr.constant.clone(),
            _ => None,
        };
        HirExpr {
            kind: HirExprKind::Ternary {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            },
            value_type,
            constant,
            location,
        }
    }

    pub(super) fn diagnostic(
        &mut self,
        code: AnalyzerDiagnosticCode,
        location: SourceLocation,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(AnalyzerDiagnostic::at(
            code,
            AnalyzerDiagnosticSeverity::Error,
            2,
            location.source,
            self.path,
            self.text,
            location.span,
            message,
        ));
    }

    #[allow(clippy::unused_self)]
    pub(super) fn error_expression(&self, location: SourceLocation) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Error,
            value_type: SemanticType::Error,
            constant: None,
            location,
        }
    }

    pub(super) fn key(&self, name: &str) -> String {
        identifier_key(name, self.options.ignore_case)
    }
}
