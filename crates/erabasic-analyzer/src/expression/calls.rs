use erabasic_ast::{Expr, ExprKind, UnaryOp};
use erabasic_hir::{
    CallTarget, ConstantValue, HirCallArgument, HirExpr, HirExprKind, SemanticType, SourceLocation,
    VariableScope,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity,
    catalog::ArgumentConstraint,
};

use super::ExpressionAnalyzer;

impl ExpressionAnalyzer<'_> {
    pub(super) fn analyze_call(
        &mut self,
        name: &str,
        args: &[Option<Expr>],
        location: SourceLocation,
    ) -> HirExpr {
        let key = self.key(name);
        let values: Vec<_> = args
            .iter()
            .map(|argument| argument.as_ref().map(|argument| self.analyze(argument)))
            .collect();
        if let Some(function) = self.symbols.function(name).cloned() {
            self.diagnose_user_call_arity(name, values.len(), function.parameter_count, location);
            let arguments = values.into_iter().map(value_call_argument).collect();
            return HirExpr {
                kind: HirExprKind::Call {
                    target: CallTarget::User {
                        function: function.id,
                    },
                    arguments,
                },
                value_type: function.return_type,
                constant: None,
                location,
            };
        }
        let Some(signature) = self.catalog.functions.get(&key).filter(|_| {
            self.catalog.extension_functions.contains(&key)
                || crate::catalog::builtin_function_available(&key, &self.options.compatibility)
        }) else {
            self.diagnostic(
                AnalyzerDiagnosticCode::UnknownFunction,
                location,
                format!("unknown expression function {name}"),
            );
            return HirExpr {
                kind: HirExprKind::Call {
                    target: CallTarget::Unresolved {
                        name: name.to_owned(),
                    },
                    arguments: values.into_iter().map(value_call_argument).collect(),
                },
                value_type: SemanticType::Error,
                constant: None,
                location,
            };
        };
        let existvar_mode = key == "EXISTVAR"
            && self
                .options
                .compatibility
                .supports_existvar_expression_probe();
        let existvar_constraints = [ArgumentConstraint::String, ArgumentConstraint::Integer];
        let constraints = if existvar_mode {
            &existvar_constraints[..]
        } else {
            signature.arguments_for_arity(values.len())
        };
        self.check_arguments(
            &values,
            constraints,
            signature.minimum_arguments,
            signature.variadic,
            signature.allow_omitted || existvar_mode,
            location,
        );
        self.check_special_builtin_call(&key, args, &values, existvar_mode, location);
        let dynamic_method = matches!(key.as_str(), "GETMETH" | "GETMETHS")
            && !self.catalog.extension_functions.contains(&key);
        if dynamic_method {
            self.check_dynamic_method_name(&values, location);
        }
        if let Some(expression) = self.fold_builtin_call(&signature.name, &key, &values, location) {
            return expression;
        }
        let regex_output = key == "REGEXPMATCH" && values.len() == 4;
        let arguments = builtin_call_arguments(
            values,
            constraints,
            signature.variadic,
            dynamic_method,
            regex_output,
        );
        let target = if self.catalog.extension_functions.contains(&key) {
            CallTarget::Extension { name: key }
        } else {
            CallTarget::Builtin { name: key }
        };
        HirExpr {
            kind: HirExprKind::Call { target, arguments },
            value_type: signature.return_type,
            constant: None,
            location,
        }
    }

    fn check_special_builtin_call(
        &mut self,
        key: &str,
        args: &[Option<Expr>],
        values: &[Option<HirExpr>],
        existvar_mode: bool,
        location: SourceLocation,
    ) {
        if matches!(key, "MATCHALL" | "MATCHALLEX") {
            self.check_match_source(
                key,
                &args.iter().map(Option::as_ref).collect::<Vec<_>>(),
                location,
            );
        }
        self.check_map_output(key, values, location);
        self.check_graphics_call(key, values, location);
        if existvar_mode && values.first().is_some_and(Option::is_none) {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "EXISTVAR source may not be omitted",
            );
        }
        self.check_bit_call(key, values, location);
    }

    pub(crate) fn check_graphics_call(
        &mut self,
        name: &str,
        values: &[Option<HirExpr>],
        location: SourceLocation,
    ) {
        if name == "SPRITECREATEFROMFILE" && values.iter().take(2).any(Option::is_none) {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "SPRITECREATEFROMFILE name and path may not be omitted",
            );
        }
        if matches!(name, "SETIMAGELAYER" | "SETIMAGELAYERL")
            && values.iter().take(2).any(Option::is_none)
        {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                format!("{name} sprite name and depth may not be omitted"),
            );
        }
        if name == "CBGSETSPRITE" {
            let snake = self.options.compatibility.supports_snake_display_state();
            let valid_arity = if snake {
                (1..=8).contains(&values.len())
            } else {
                values.len() == 4
            };
            let invalid_omission = if snake {
                values.first().is_none_or(Option::is_none)
            } else {
                values.iter().any(Option::is_none)
            };
            if !valid_arity {
                self.diagnostic(
                    AnalyzerDiagnosticCode::InvalidArgumentCount,
                    location,
                    if snake {
                        format!(
                            "CBGSETSPRITE expects 1 to 8 arguments, found {}",
                            values.len()
                        )
                    } else {
                        format!(
                            "CBGSETSPRITE expects exactly 4 arguments, found {}",
                            values.len()
                        )
                    },
                );
            }
            if invalid_omission {
                self.diagnostic(
                    AnalyzerDiagnosticCode::InvalidArgument,
                    location,
                    if snake {
                        "CBGSETSPRITE sprite name may not be omitted"
                    } else {
                        "CBGSETSPRITE arguments may not be omitted in the original profile"
                    },
                );
            }
        }
        let arity = values.len();
        if name != "SPRITECREATE" || !(2..=10).contains(&arity) {
            return;
        }
        let snake = self.options.compatibility.supports_snake_display_state();
        let valid = matches!(arity, 2 | 6) || (snake && matches!(arity, 8 | 10));
        if !valid {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgumentCount,
                location,
                if snake {
                    format!("SPRITECREATE expects 2, 6, 8 or 10 arguments, found {arity}")
                } else {
                    format!("SPRITECREATE expects 2 or 6 arguments, found {arity}")
                },
            );
        }
    }

    pub(crate) fn check_bit_call(
        &mut self,
        name: &str,
        values: &[Option<HirExpr>],
        location: SourceLocation,
    ) {
        if !matches!(name, "BITSET" | "BITGET" | "BITTOGGLE" | "BITINDEXOFFIRST") {
            return;
        }
        let valid = values
            .first()
            .and_then(Option::as_ref)
            .is_some_and(|expression| {
                let HirExprKind::Variable { place } = &expression.kind else {
                    return false;
                };
                self.symbols
                    .variables
                    .get(place.variable.0 as usize)
                    .is_some_and(|variable| {
                        variable.dimensions.len() == 1
                            && variable.mutable
                            && variable.value_type == SemanticType::Integer
                    })
            });
        if !valid {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "BIT input must be a mutable Integer array of rank one",
            );
        }
        if name == "BITSET" && values.get(1).is_none_or(Option::is_none) {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "BITSET index may not be omitted",
            );
        }
    }

    pub(crate) fn check_match_source(
        &mut self,
        name: &str,
        args: &[Option<&Expr>],
        location: SourceLocation,
    ) {
        fn atom(mut expr: &Expr) -> &Expr {
            while let ExprKind::Group(inner) = &expr.kind {
                expr = inner;
            }
            expr
        }
        let token = |expr: &Expr| {
            matches!(
                atom(expr).kind,
                ExprKind::Identifier(_) | ExprKind::Variable { .. }
            )
        };
        let first_valid = args.first().copied().flatten().is_some_and(|expr| {
            if name == "MATCHALLEX" {
                matches!(atom(expr).kind, ExprKind::String(_))
            } else {
                token(expr)
            }
        });
        if !first_valid
            || args.get(1).is_none_or(Option::is_none)
            || args
                .get(4)
                .is_some_and(|arg| arg.is_none_or(|expr| !token(expr)))
        {
            self.diagnostic(AnalyzerDiagnosticCode::InvalidArgument, location,
                "MATCH requires its source token (MATCHALLEX: literal string), needle and any supplied output token");
        }
    }

    fn fold_builtin_call(
        &self,
        name: &str,
        key: &str,
        arguments: &[Option<HirExpr>],
        location: SourceLocation,
    ) -> Option<HirExpr> {
        if name != "GETNUM" || self.catalog.extension_functions.contains(key) {
            return None;
        }
        let value = self.fold_builtin_getnum(arguments)?;
        Some(HirExpr {
            kind: HirExprKind::Integer { value },
            value_type: SemanticType::Integer,
            constant: Some(ConstantValue::Integer(value)),
            location,
        })
    }

    fn fold_builtin_getnum(&self, arguments: &[Option<HirExpr>]) -> Option<i64> {
        if !(2..=3).contains(&arguments.len()) {
            return None;
        }
        let HirExprKind::Variable { place } = &arguments.first()?.as_ref()?.kind else {
            return None;
        };
        let variable = self.symbols.variables.get(place.variable.0 as usize)?;
        if !place.indices.is_empty()
            || variable.scope != VariableScope::Project
            || variable.owner.is_some()
            || variable.reference
        {
            return None;
        }
        let ConstantValue::String(key) = self.pure_constant(arguments.get(1)?.as_ref()?)? else {
            return None;
        };
        let source_dimension = match arguments.get(2) {
            None => 0,
            Some(Some(argument)) => {
                let ConstantValue::Integer(value) = self.pure_constant(argument)? else {
                    return None;
                };
                value
            }
            Some(None) => return None,
        };
        let data_dimension = if source_dimension > 0 {
            source_dimension - 1
        } else {
            source_dimension
        };
        let Ok(data_dimension) = usize::try_from(data_dimension) else {
            return Some(-1);
        };
        Some(
            self.index_resolver
                .resolve_builtin(&variable.name, data_dimension, &key)
                .unwrap_or(-1),
        )
    }

    fn pure_constant(&self, expression: &HirExpr) -> Option<ConstantValue> {
        match &expression.kind {
            HirExprKind::Integer { .. } | HirExprKind::String { .. } => {}
            HirExprKind::Variable { place } => {
                let variable = self.symbols.variables.get(place.variable.0 as usize)?;
                if !place.indices.is_empty()
                    || variable.storage != erabasic_data::StorageScope::Constant
                {
                    return None;
                }
            }
            HirExprKind::Unary { op, operand }
                if !matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) =>
            {
                self.pure_constant(operand)?;
            }
            HirExprKind::Binary { left, right, .. } => {
                self.pure_constant(left)?;
                self.pure_constant(right)?;
            }
            HirExprKind::Ternary {
                condition,
                then_expr,
                else_expr,
            } => {
                self.pure_constant(condition)?;
                self.pure_constant(then_expr)?;
                self.pure_constant(else_expr)?;
            }
            _ => return None,
        }
        expression.constant.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn check_dynamic_method_name(
        &mut self,
        values: &[Option<HirExpr>],
        location: SourceLocation,
    ) {
        if values.first().is_some_and(Option::is_none) {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "dynamic method target name may not be omitted",
            );
        }
    }

    pub fn check_arguments(
        &mut self,
        arguments: &[Option<HirExpr>],
        constraints: &[ArgumentConstraint],
        minimum: usize,
        variadic: bool,
        allow_omitted: bool,
        location: SourceLocation,
    ) {
        if arguments.len() < minimum || (!variadic && arguments.len() > constraints.len()) {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgumentCount,
                location,
                format!(
                    "expected {}{} arguments, found {}",
                    minimum,
                    if variadic { " or more" } else { "" },
                    arguments.len()
                ),
            );
        }
        for (index, argument) in arguments.iter().enumerate() {
            let Some(argument) = argument else {
                if !allow_omitted {
                    self.diagnostic(
                        AnalyzerDiagnosticCode::InvalidArgument,
                        location,
                        format!("argument {} may not be omitted", index + 1),
                    );
                }
                continue;
            };
            let constraint = constraints
                .get(index)
                .or_else(|| variadic.then(|| constraints.last()).flatten());
            if let Some(constraint) = constraint {
                self.check_constraint(argument, *constraint, index + 1);
            }
        }
    }

    fn check_constraint(
        &mut self,
        expression: &HirExpr,
        constraint: ArgumentConstraint,
        index: usize,
    ) {
        let expected = match constraint {
            ArgumentConstraint::Integer
            | ArgumentConstraint::MutableInteger
            | ArgumentConstraint::IntegerOrReference => Some(SemanticType::Integer),
            ArgumentConstraint::String | ArgumentConstraint::MutableString => {
                Some(SemanticType::String)
            }
            ArgumentConstraint::Any
            | ArgumentConstraint::MutableAny
            | ArgumentConstraint::ReferenceAny
            | ArgumentConstraint::ReferenceOrString
            | ArgumentConstraint::MutableReferenceOrString
            | ArgumentConstraint::IntegerOrMutableString
            | ArgumentConstraint::Formatted
            | ArgumentConstraint::Raw => None,
        };
        if let Some(expected) = expected {
            self.expect_type(expression, expected, &format!("argument {index}"));
        }
        if matches!(
            constraint,
            ArgumentConstraint::MutableInteger
                | ArgumentConstraint::MutableString
                | ArgumentConstraint::MutableAny
                | ArgumentConstraint::ReferenceAny
        ) {
            if constraint == ArgumentConstraint::ReferenceAny {
                if !matches!(expression.kind, HirExprKind::Variable { .. }) {
                    self.diagnostic(
                        AnalyzerDiagnosticCode::InvalidArgument,
                        expression.location,
                        format!("argument {index} must be a variable reference"),
                    );
                }
            } else {
                self.expect_mutable_place(expression, &format!("argument {index}"));
            }
        } else if matches!(
            constraint,
            ArgumentConstraint::ReferenceOrString | ArgumentConstraint::MutableReferenceOrString
        ) {
            if matches!(expression.kind, HirExprKind::Variable { .. }) {
                if constraint == ArgumentConstraint::MutableReferenceOrString {
                    self.expect_mutable_place(expression, &format!("argument {index}"));
                }
            } else {
                self.expect_type(
                    expression,
                    SemanticType::String,
                    &format!("argument {index}"),
                );
            }
        } else if constraint == ArgumentConstraint::IntegerOrMutableString {
            if expression.value_type == SemanticType::String {
                self.expect_mutable_place(expression, &format!("argument {index}"));
            } else {
                self.expect_type(
                    expression,
                    SemanticType::Integer,
                    &format!("argument {index}"),
                );
            }
        }
    }

    pub(super) fn expect_type(&mut self, expression: &HirExpr, expected: SemanticType, role: &str) {
        if expression.value_type != expected && expression.value_type != SemanticType::Error {
            self.diagnostic(
                AnalyzerDiagnosticCode::TypeMismatch,
                expression.location,
                format!(
                    "{role} must be {expected:?}, found {:?}",
                    expression.value_type
                ),
            );
        }
    }

    pub(super) fn expect_mutable_place(&mut self, expression: &HirExpr, role: &str) {
        match &expression.kind {
            HirExprKind::Variable { place } if place.mutable => {}
            _ => self.diagnostic(
                AnalyzerDiagnosticCode::InvalidAssignment,
                expression.location,
                format!("{role} must be a mutable variable"),
            ),
        }
    }

    pub(crate) fn diagnose_user_call_arity(
        &mut self,
        name: &str,
        supplied: usize,
        formal: usize,
        location: SourceLocation,
    ) {
        use erabasic_compat::{UserCallArgumentPolicy, UserCallArityDiagnostic};
        // Preserve reference rejection in the existing compiler path. Snake load
        // diagnostics are produced on every analysis, independent of function-cache hits.
        if self.options.compatibility.user_call_argument_policy(false)
            == UserCallArgumentPolicy::RejectExcess
        {
            return;
        }
        let decision = self
            .options
            .compatibility
            .user_call_argument_policy(self.options.strict_user_call_arguments)
            .decide(supplied, formal);
        let Some(level) = decision.diagnostic else {
            return;
        };
        let (severity, reference_level) = match level {
            UserCallArityDiagnostic::Warning => (AnalyzerDiagnosticSeverity::Warning, 1),
            UserCallArityDiagnostic::Error => (AnalyzerDiagnosticSeverity::Error, 2),
        };
        self.diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::ExcessUserArguments,
            severity,
            reference_level,
            location.source,
            self.path,
            self.text,
            location.span,
            format!(
                "user function {name} supplies {supplied} arguments for {formal} parameters; {} excess arguments are not evaluated",
                decision.excess,
            ),
        ));
    }
}

fn value_call_argument(value: Option<HirExpr>) -> HirCallArgument {
    value.map_or(HirCallArgument::Omitted, HirCallArgument::Value)
}

fn argument_keeps_place(constraint: ArgumentConstraint, value_type: SemanticType) -> bool {
    matches!(
        constraint,
        ArgumentConstraint::MutableInteger
            | ArgumentConstraint::MutableString
            | ArgumentConstraint::MutableAny
            | ArgumentConstraint::ReferenceAny
            | ArgumentConstraint::ReferenceOrString
            | ArgumentConstraint::MutableReferenceOrString
    ) || constraint == ArgumentConstraint::IntegerOrMutableString
        && value_type == SemanticType::String
}

// Preserve variable tokens for APIs whose contracts capture a place before evaluation.
fn builtin_call_arguments(
    values: Vec<Option<HirExpr>>,
    constraints: &[ArgumentConstraint],
    variadic: bool,
    dynamic_method: bool,
    regex_output: bool,
) -> Vec<HirCallArgument> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| match value {
            None => HirCallArgument::Omitted,
            Some(expression)
                if dynamic_method && index >= 2
                    || constraints
                        .get(index)
                        .or_else(|| variadic.then(|| constraints.last()).flatten())
                        .is_some_and(|constraint| {
                            argument_keeps_place(*constraint, expression.value_type)
                                || regex_output && index == 2
                        }) =>
            {
                match expression.kind {
                    HirExprKind::Variable { place } => HirCallArgument::Place(place),
                    _ => HirCallArgument::Value(expression),
                }
            }
            Some(expression) => HirCallArgument::Value(expression),
        })
        .collect()
}

impl ExpressionAnalyzer<'_> {
    pub(crate) fn check_map_output(
        &mut self,
        name: &str,
        values: &[Option<HirExpr>],
        location: SourceLocation,
    ) {
        if name != "MAP_VALUES" || values.len() != 3 {
            return;
        }
        let valid = values
            .get(1)
            .and_then(Option::as_ref)
            .is_some_and(|value| match &value.kind {
                HirExprKind::Variable { place } => self
                    .symbols
                    .variables
                    .get(place.variable.0 as usize)
                    .is_some_and(|variable| variable.dimensions.len() == 1),
                _ => false,
            });
        if !valid {
            self.diagnostic(
                AnalyzerDiagnosticCode::InvalidArgument,
                location,
                "MAP_VALUES output must be a one-dimensional String array token",
            );
        }
    }
}
