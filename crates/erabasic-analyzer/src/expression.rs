use erabasic_ast::{BinaryOp, Expr, ExprKind, FormPart, FormattedString, PostfixOp, UnaryOp};
use erabasic_data::{LegacyEncoding, NameTableKind, ProjectData};
use erabasic_hir::{
    CallTarget, ConstantValue, FunctionId, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirPlace, SemanticType, SourceId, SourceLocation,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::{ArgumentConstraint, Catalog},
    symbols::Symbols,
};

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

#[derive(Default)]
pub(crate) struct IndexResolver {
    tables: BTreeMap<(String, usize), BTreeMap<String, i64>>,
    rename: BTreeMap<String, i64>,
    legacy_encoding: LegacyEncoding,
}

impl IndexResolver {
    pub fn new(project: &ProjectData) -> Self {
        let mut result = Self::default();
        for (kind, table) in &project.static_data.name_tables {
            let dimension = dimension_for_kind(*kind);
            let lookup: BTreeMap<_, _> = table
                .lookup
                .iter()
                .map(|(name, index)| (name.clone(), i64::from(*index)))
                .collect();
            for variable in data_variables_for_kind(*kind) {
                result
                    .tables
                    .insert(((*variable).to_owned(), dimension), lookup.clone());
            }
        }
        for (name, table) in &project.static_data.deferred_indices.resolved {
            let (variable, dimension) = name
                .rsplit_once('@')
                .and_then(|(variable, dimension)| {
                    dimension
                        .parse::<usize>()
                        .ok()
                        .map(|dimension| (variable, dimension - 1))
                })
                .unwrap_or((name.as_str(), 0));
            result.tables.insert(
                (variable.to_ascii_uppercase(), dimension),
                table
                    .entries
                    .iter()
                    .map(|(name, index)| {
                        (
                            name.clone(),
                            i64::try_from(*index).expect("deferred index exceeds i64"),
                        )
                    })
                    .collect(),
            );
        }
        result.rename = project
            .static_data
            .rename
            .iter()
            .filter_map(|(name, value)| value.parse().ok().map(|value| (name.clone(), value)))
            .collect();
        result.legacy_encoding = project.static_data.legacy_encoding;
        result
    }

    pub(crate) fn resolve(&self, variable: &str, dimension: usize, name: &str) -> Option<i64> {
        self.tables
            .get(&(variable.to_ascii_uppercase(), dimension))
            .and_then(|table| table.get(name))
            .copied()
    }

    pub(crate) fn resolve_rename(&self, name: &str) -> Option<i64> {
        self.rename.get(name).copied()
    }

    pub(crate) fn has_table(&self, variable: &str, dimension: usize) -> bool {
        self.tables
            .contains_key(&(variable.to_ascii_uppercase(), dimension))
    }

    pub(crate) fn legacy_encoded_len(&self, value: &str) -> usize {
        if value.is_ascii() {
            return value.len();
        }
        let encoding = match self.legacy_encoding {
            LegacyEncoding::Japanese => encoding_rs::SHIFT_JIS,
            LegacyEncoding::Korean => encoding_rs::EUC_KR,
            LegacyEncoding::ChineseHans => encoding_rs::GBK,
            LegacyEncoding::ChineseHant => encoding_rs::BIG5,
        };
        value
            .chars()
            .map(|character| {
                let mut utf8 = [0; 4];
                let encoded = character.encode_utf8(&mut utf8);
                let (bytes, _, had_errors) = encoding.encode(encoded);
                if had_errors { 1 } else { bytes.len() }
            })
            .sum()
    }
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

    fn analyze_call(
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
        if let Some(function) = self.symbols.function(name) {
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
        let Some(signature) = self.catalog.functions.get(&key) else {
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
        self.check_arguments(
            &values,
            &signature.arguments,
            signature.minimum_arguments,
            signature.variadic,
            signature.allow_omitted,
            location,
        );
        let argument_count = values.len();
        let arguments = values
            .into_iter()
            .enumerate()
            .map(|(index, value)| match value {
                None => HirCallArgument::Omitted,
                Some(expression)
                    if signature
                        .arguments
                        .get(index)
                        .or_else(|| {
                            signature
                                .variadic
                                .then(|| signature.arguments.last())
                                .flatten()
                        })
                        .is_some_and(|constraint| {
                            matches!(
                                constraint,
                                ArgumentConstraint::MutableInteger
                                    | ArgumentConstraint::MutableString
                                    | ArgumentConstraint::MutableAny
                                    | ArgumentConstraint::ReferenceAny
                                    | ArgumentConstraint::ReferenceOrString
                                    | ArgumentConstraint::MutableReferenceOrString
                            ) || *constraint == ArgumentConstraint::IntegerOrMutableString
                                && expression.value_type == SemanticType::String
                                || key == "REGEXPMATCH" && argument_count == 4 && index == 2
                        }) =>
                {
                    match expression.kind {
                        HirExprKind::Variable { place } => HirCallArgument::Place(place),
                        _ => HirCallArgument::Value(expression),
                    }
                }
                Some(expression) => HirCallArgument::Value(expression),
            })
            .collect();
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
            (Some(ConstantValue::Integer(value)), UnaryOp::Minus) => {
                Some(ConstantValue::Integer(value.wrapping_neg()))
            }
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
        let constant = fold_binary(op, left.constant.as_ref(), right.constant.as_ref());
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

    #[allow(clippy::too_many_arguments)]
    pub fn check_arguments(
        &mut self,
        arguments: &[Option<HirExpr>],
        constraints: &[ArgumentConstraint],
        minimum: usize,
        variadic: bool,
        allow_omitted: bool,
        location: SourceLocation,
    ) {
        let supplied = arguments
            .iter()
            .filter(|argument| argument.is_some())
            .count();
        if supplied < minimum || (!variadic && arguments.len() > constraints.len()) {
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

    fn expect_type(&mut self, expression: &HirExpr, expected: SemanticType, role: &str) {
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

    fn expect_mutable_place(&mut self, expression: &HirExpr, role: &str) {
        match &expression.kind {
            HirExprKind::Variable { place } if place.mutable => {}
            _ => self.diagnostic(
                AnalyzerDiagnosticCode::InvalidAssignment,
                expression.location,
                format!("{role} must be a mutable variable"),
            ),
        }
    }

    fn diagnostic(
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
    fn error_expression(&self, location: SourceLocation) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Error,
            value_type: SemanticType::Error,
            constant: None,
            location,
        }
    }

    fn key(&self, name: &str) -> String {
        if self.options.ignore_case {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }
}

fn normalize_colon_indices(indices: &[Expr], maximum: usize) -> Cow<'_, [Expr]> {
    if maximum == 0 || indices.len() <= maximum {
        return Cow::Borrowed(indices);
    }
    // Syntax parsing cannot know whether `TMP:ARG:0` describes a two-dimensional
    // TMP or a one-dimensional TMP indexed by ARG:0. Once the declaration is
    // resolved, fold the excess colon tail into the final permitted index. This
    // mirrors the reference's variable-token-aware expression parser without
    // leaking semantic variable shapes into the public parser context.
    let nested_start = maximum - 1;
    let mut normalized = indices[..nested_start].to_vec();
    let mut nested = indices[nested_start].clone();
    let extra = indices[nested_start + 1..].to_vec();
    let end = extra.last().map_or(nested.span, |value| value.span);
    nested.kind = match nested.kind {
        ExprKind::Identifier(name) => ExprKind::Variable {
            name,
            indices: extra,
        },
        ExprKind::Variable {
            name,
            indices: mut existing,
        } => {
            existing.extend(extra);
            ExprKind::Variable {
                name,
                indices: existing,
            }
        }
        _ => return Cow::Borrowed(indices),
    };
    nested.span = nested.span.join(end);
    normalized.push(nested);
    Cow::Owned(normalized)
}

fn value_call_argument(value: Option<HirExpr>) -> HirCallArgument {
    value.map_or(HirCallArgument::Omitted, HirCallArgument::Value)
}

fn data_variables_for_kind(kind: NameTableKind) -> &'static [&'static str] {
    match kind {
        NameTableKind::Abl => &["ABL"],
        NameTableKind::Exp => &["EXP"],
        NameTableKind::Talent => &["TALENT"],
        NameTableKind::Palam => &["PALAM", "UP", "DOWN", "JUEL", "GOTJUEL", "CUP", "CDOWN"],
        NameTableKind::Train => &["TRAIN"],
        NameTableKind::Mark => &["MARK"],
        // ITEM.csv names are shared by every item-indexed built-in variable.
        NameTableKind::Item => &["ITEM", "ITEMSALES", "ITEMPRICE", "ITEMNAME"],
        NameTableKind::Base => &["BASE", "MAXBASE", "LOSEBASE", "DOWNBASE"],
        NameTableKind::Source => &["SOURCE"],
        NameTableKind::Ex => &["EX", "NOWEX"],
        // STR.CSV contains initial string values. Symbolic STR indices come from
        // STRNAME.CSV in the reference implementation.
        NameTableKind::Str => &[],
        NameTableKind::Equip => &["EQUIP"],
        NameTableKind::Tequip => &["TEQUIP"],
        NameTableKind::Flag => &["FLAG"],
        NameTableKind::Tflag => &["TFLAG"],
        NameTableKind::Cflag => &["CFLAG"],
        NameTableKind::Tcvar => &["TCVAR"],
        NameTableKind::Cstr => &["CSTR"],
        NameTableKind::Stain => &["STAIN"],
        NameTableKind::Cdflag1 | NameTableKind::Cdflag2 => &["CDFLAG"],
        NameTableKind::Strname => &["STR", "STRNAME"],
        NameTableKind::Tstr => &["TSTR"],
        NameTableKind::Savestr => &["SAVESTR"],
        NameTableKind::Global => &["GLOBAL"],
        NameTableKind::Globals => &["GLOBALS"],
        NameTableKind::Day => &["DAY"],
        NameTableKind::Time => &["TIME"],
        NameTableKind::Money => &["MONEY"],
    }
}

fn dimension_for_kind(kind: NameTableKind) -> usize {
    usize::from(kind == NameTableKind::Cdflag2)
}

#[allow(clippy::too_many_lines)]
fn fold_binary(
    op: BinaryOp,
    left: Option<&ConstantValue>,
    right: Option<&ConstantValue>,
) -> Option<ConstantValue> {
    match (left?, right?) {
        (ConstantValue::String(left), ConstantValue::String(right)) => match op {
            BinaryOp::Add => Some(ConstantValue::String(format!("{left}{right}"))),
            BinaryOp::Equal => Some(ConstantValue::Integer(i64::from(left == right))),
            BinaryOp::NotEqual => Some(ConstantValue::Integer(i64::from(left != right))),
            BinaryOp::Less => Some(ConstantValue::Integer(i64::from(left < right))),
            BinaryOp::LessEqual => Some(ConstantValue::Integer(i64::from(left <= right))),
            BinaryOp::Greater => Some(ConstantValue::Integer(i64::from(left > right))),
            BinaryOp::GreaterEqual => Some(ConstantValue::Integer(i64::from(left >= right))),
            _ => None,
        },
        (ConstantValue::String(value), ConstantValue::Integer(count))
        | (ConstantValue::Integer(count), ConstantValue::String(value))
            if op == BinaryOp::Multiply && (0..10_000).contains(count) =>
        {
            Some(ConstantValue::String(
                value.repeat(usize::try_from(*count).ok()?),
            ))
        }
        (ConstantValue::Integer(left), ConstantValue::Integer(right)) => {
            Some(ConstantValue::Integer(match op {
                BinaryOp::Multiply => left.wrapping_mul(*right),
                BinaryOp::Divide if *right != 0 => left.wrapping_div(*right),
                BinaryOp::Modulo if *right != 0 => left.wrapping_rem(*right),
                BinaryOp::Add => left.wrapping_add(*right),
                BinaryOp::Subtract => left.wrapping_sub(*right),
                BinaryOp::ShiftLeft => left.wrapping_shl(u32::try_from(*right).unwrap_or_default()),
                BinaryOp::ShiftRight => {
                    left.wrapping_shr(u32::try_from(*right).unwrap_or_default())
                }
                BinaryOp::Less => i64::from(left < right),
                BinaryOp::LessEqual => i64::from(left <= right),
                BinaryOp::Greater => i64::from(left > right),
                BinaryOp::GreaterEqual => i64::from(left >= right),
                BinaryOp::Equal => i64::from(left == right),
                BinaryOp::NotEqual => i64::from(left != right),
                BinaryOp::BitAnd => left & right,
                BinaryOp::BitXor => left ^ right,
                BinaryOp::BitOr => left | right,
                BinaryOp::LogicalAnd => i64::from(*left != 0 && *right != 0),
                BinaryOp::LogicalXor => i64::from((*left != 0) ^ (*right != 0)),
                BinaryOp::LogicalOr => i64::from(*left != 0 || *right != 0),
                BinaryOp::Nand => i64::from(!(*left != 0 && *right != 0)),
                BinaryOp::Nor => i64::from(!(*left != 0 || *right != 0)),
                BinaryOp::Divide | BinaryOp::Modulo => return None,
            }))
        }
        _ => None,
    }
}
