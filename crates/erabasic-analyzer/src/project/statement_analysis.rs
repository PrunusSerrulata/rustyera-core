use std::collections::BTreeSet;

use erabasic_ast::{
    Argument, Expr, ExprKind, Function as AstFunction, Span, Statement, StatementKind, VariableRef,
};
use erabasic_hir::{
    ConstantValue, Function, FunctionId, FunctionKind, HirArgument, HirExpr, HirExprKind,
    HirStatement, HirStatementKind, InstructionTarget, LabelId, LineId, Parameter, SemanticType,
    SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    control_flow::build_control_flow,
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::Symbols,
};

use super::{
    ParsedProjectSource,
    lowering_support::{
        analyze_case_arguments, analyze_scoped_declaration_statement, resolve_static_target,
    },
    reachability::event_attributes,
    source_support::{confine_formatted_spans, confine_span, key, map_parser_diagnostic},
};

pub(super) struct FunctionDefinition {
    pub(super) source_index: usize,
    pub(super) function_index: usize,
    pub(super) id: FunctionId,
    pub(super) kind: FunctionKind,
    pub(super) return_type: SemanticType,
    pub(super) shadowed: bool,
    pub(super) definition_order: u32,
}

pub(super) fn should_analyze_function(
    definition: &FunctionDefinition,
    reachable: &BTreeSet<FunctionId>,
    options: &AnalyzerOptions,
) -> bool {
    options.analysis_mode
        || (!definition.shadowed
            && (!options.ignore_uncalled_functions || reachable.contains(&definition.id)))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub(super) fn analyze_function(
    id: FunctionId,
    kind: FunctionKind,
    return_type: SemanticType,
    definition_order: u32,
    source: &ParsedProjectSource,
    function: &AstFunction,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> Function {
    let mut parameters = Vec::new();
    for parameter in &function.parameters {
        let target = parameter.target.clone().unwrap_or_else(|| VariableRef {
            name: parameter.name.clone(),
            indices: Vec::new(),
            span: parameter.span,
        });
        let target_expression = Expr {
            kind: ExprKind::Variable {
                name: target.name.clone(),
                indices: target.indices,
            },
            span: target.span,
        };
        let analyzed_target = ExpressionAnalyzer {
            symbols,
            catalog,
            options,
            function: id,
            source: source.source.id,
            path: &source.source.relative_path,
            text: &source.text,
            diagnostics,
            index_resolver,
        }
        .analyze(&target_expression);
        let HirExprKind::Variable { place } = analyzed_target.kind else {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                format!("function parameter {} is not a variable", parameter.name),
            ));
            continue;
        };
        let variable = symbols.variables.get(place.variable.0 as usize);
        let reference = variable.is_some_and(|variable| variable.reference);
        let can_default = matches!(target.name.to_ascii_uppercase().as_str(), "ARG" | "ARGS")
            || variable
                .is_some_and(|variable| variable.scope == erabasic_hir::VariableScope::Function);
        let default = parameter
            .default
            .as_ref()
            .map(|expression| {
                ExpressionAnalyzer {
                    symbols,
                    catalog,
                    options,
                    function: id,
                    source: source.source.id,
                    path: &source.source.relative_path,
                    text: &source.text,
                    diagnostics,
                    index_resolver,
                }
                .analyze(expression)
            })
            .or_else(|| {
                (!reference && can_default).then(|| {
                    let constant = match place.value_type {
                        SemanticType::String => ConstantValue::String(String::new()),
                        _ => ConstantValue::Integer(0),
                    };
                    HirExpr {
                        kind: match &constant {
                            ConstantValue::Integer(value) => HirExprKind::Integer { value: *value },
                            ConstantValue::String(value) => HirExprKind::String {
                                value: value.clone(),
                            },
                        },
                        value_type: place.value_type,
                        constant: Some(constant),
                        location: SourceLocation::new(source.source.id, parameter.span),
                    }
                })
            });
        if let Some(default) = &default
            && default.value_type != place.value_type
            && default.value_type != SemanticType::Error
        {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::TypeMismatch,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "parameter default type does not match its variable",
            ));
        }
        if let Some(default) = &default
            && default.constant.is_none()
            && default.value_type != SemanticType::Error
        {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "parameter default must be a compile-time constant",
            ));
        }
        if parameter.default.is_some() && reference {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgument,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                parameter.span,
                "reference parameters cannot have defaults",
            ));
        }
        parameters.push(Parameter {
            target: place,
            default,
        });
    }

    let mut lines = Vec::new();
    let mut next_label = 0u32;
    for statement in &function.body {
        let line_id = LineId(u32::try_from(lines.len()).expect("too many lines"));
        lines.push(analyze_statement(
            line_id,
            &mut next_label,
            id,
            source,
            statement,
            symbols,
            catalog,
            context,
            index_resolver,
            options,
            diagnostics,
        ));
    }
    let (labels, control_flow) = build_control_flow(
        &lines,
        symbols,
        source.source.id,
        &source.source.relative_path,
        &source.text,
        diagnostics,
    );
    Function {
        id,
        name: function.name.clone(),
        kind,
        event_attributes: event_attributes(kind, function),
        definition_order,
        return_type,
        parameters,
        lines,
        labels,
        control_flow,
        location: SourceLocation::new(source.source.id, function.span),
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn analyze_statement(
    line_id: LineId,
    next_label: &mut u32,
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatement {
    let location_span = match &statement.kind {
        StatementKind::Instruction { arguments, .. } if matches!(arguments.first(), Some(Argument::Raw(value)) if !value.is_empty()) =>
        {
            let Argument::Raw(value) = &arguments[0] else {
                unreachable!("guard requires a raw argument")
            };
            source.text[statement.span.start..statement.span.end]
                .find(value)
                .map_or(statement.span, |offset| {
                    Span::new(statement.span.start + offset, statement.span.end)
                })
        }
        _ => statement.span,
    };
    let location = SourceLocation::new(source.source.id, location_span);
    let kind = match &statement.kind {
        StatementKind::Assignment {
            target,
            op,
            value,
            additional_values,
            raw_value,
        } => {
            let target_expression = Expr {
                kind: ExprKind::Variable {
                    name: target.name.clone(),
                    indices: target.indices.clone(),
                },
                span: target.span,
            };
            let analyzed_target = ExpressionAnalyzer {
                symbols,
                catalog,
                options,
                function,
                source: source.source.id,
                path: &source.source.relative_path,
                text: &source.text,
                diagnostics,
                index_resolver,
            }
            .analyze(&target_expression);
            let HirExprKind::Variable { place } = analyzed_target.kind else {
                return HirStatement {
                    id: line_id,
                    kind: HirStatementKind::Error,
                    location,
                };
            };
            let form_assignment =
                place.value_type == SemanticType::String && *op == erabasic_ast::AssignOp::Assign;
            let mut reparsed_values = Vec::new();
            let mut reparse_had_errors = false;
            let value = if form_assignment {
                let mut parsed = erabasic_parser::parse_assignment_formatted_at(
                    raw_value,
                    value.span.start,
                    context,
                );
                for diagnostic in parsed.diagnostics.drain(..) {
                    diagnostics.push(map_parser_diagnostic(
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        &diagnostic,
                    ));
                }
                parsed.value.map_or_else(
                    || Expr {
                        kind: ExprKind::Error,
                        span: value.span,
                    },
                    |formatted| Expr {
                        kind: ExprKind::Formatted(formatted),
                        span: value.span,
                    },
                )
            } else if *op == erabasic_ast::AssignOp::Assign {
                let mut parsed =
                    erabasic_parser::parse_expression_list_at(raw_value, value.span.start, context);
                reparse_had_errors = parsed.has_errors();
                for diagnostic in parsed.diagnostics.drain(..) {
                    diagnostics.push(map_parser_diagnostic(
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        &diagnostic,
                    ));
                }
                reparsed_values = parsed.value.unwrap_or_default();
                reparsed_values.first().cloned().unwrap_or(Expr {
                    kind: ExprKind::Error,
                    span: value.span,
                })
            } else {
                value.clone()
            };
            let default_omitted = |expression: &Expr| {
                if !reparse_had_errors && matches!(expression.kind, ExprKind::Error) {
                    Expr {
                        kind: match place.value_type {
                            SemanticType::String => ExprKind::String(String::new()),
                            SemanticType::Integer | SemanticType::Void | SemanticType::Error => {
                                ExprKind::Integer(0)
                            }
                        },
                        span: expression.span,
                    }
                } else {
                    expression.clone()
                }
            };
            let value = default_omitted(&value);
            let mut values = vec![
                ExpressionAnalyzer {
                    symbols,
                    catalog,
                    options,
                    function,
                    source: source.source.id,
                    path: &source.source.relative_path,
                    text: &source.text,
                    diagnostics,
                    index_resolver,
                }
                .analyze(&value),
            ];
            if !form_assignment {
                let additional = if *op == erabasic_ast::AssignOp::Assign {
                    reparsed_values.iter().skip(1).collect::<Vec<_>>()
                } else {
                    additional_values.iter().collect::<Vec<_>>()
                };
                for additional in additional {
                    values.push(
                        ExpressionAnalyzer {
                            symbols,
                            catalog,
                            options,
                            function,
                            source: source.source.id,
                            path: &source.source.relative_path,
                            text: &source.text,
                            diagnostics,
                            index_resolver,
                        }
                        .analyze(&default_omitted(additional)),
                    );
                }
            }
            if !place.mutable {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::InvalidAssignment,
                    AnalyzerDiagnosticSeverity::Error,
                    2,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    target.span,
                    "assignment target is immutable",
                ));
            }
            for analyzed_value in &values {
                if place.value_type != analyzed_value.value_type
                    && analyzed_value.value_type != SemanticType::Error
                {
                    diagnostics.push(AnalyzerDiagnostic::at(
                        AnalyzerDiagnosticCode::TypeMismatch,
                        AnalyzerDiagnosticSeverity::Error,
                        2,
                        source.source.id,
                        &source.source.relative_path,
                        &source.text,
                        analyzed_value.location.span,
                        "assignment value type does not match its target",
                    ));
                }
            }
            if values.len() > 1 {
                let mut arguments = vec![HirArgument::Place(place)];
                arguments.extend(values.into_iter().map(HirArgument::Expression));
                HirStatementKind::Instruction {
                    target: InstructionTarget::Builtin("SET".into()),
                    arguments,
                }
            } else {
                HirStatementKind::Assignment {
                    target: place,
                    op: *op,
                    value: values.pop().expect("an assignment always has one value"),
                }
            }
        }
        StatementKind::Instruction {
            name,
            arguments,
            raw_arguments,
        } => analyze_instruction(
            function,
            source,
            statement,
            name,
            arguments,
            raw_arguments,
            symbols,
            catalog,
            context,
            index_resolver,
            options,
            diagnostics,
        ),
        StatementKind::GotoLabel { name } => {
            let label = LabelId(*next_label);
            *next_label += 1;
            HirStatementKind::Label {
                label,
                name: name.clone(),
            }
        }
        StatementKind::Directive(_) | StatementKind::Invalid => HirStatementKind::Error,
    };
    HirStatement {
        id: line_id,
        kind,
        location,
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn analyze_instruction(
    function: FunctionId,
    source: &ParsedProjectSource,
    statement: &Statement,
    name: &str,
    arguments: &[Argument],
    raw_arguments: &str,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatementKind {
    let key = key(name, options.ignore_case);
    let signature = catalog.instructions.get(&key);
    let method_signature = signature
        .is_none()
        .then(|| catalog.functions.get(&key))
        .flatten();
    if matches!(key.as_str(), "VARI" | "VARS") {
        return analyze_scoped_declaration_statement(
            function,
            source,
            statement,
            &key,
            raw_arguments,
            symbols,
            catalog,
            context,
            index_resolver,
            options,
            diagnostics,
        );
    }
    if signature.is_none() && method_signature.is_none() {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::UnknownInstruction,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            statement.span,
            format!("unknown instruction {name}"),
        ));
    }
    let static_target = matches!(
        key.as_str(),
        "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYJUMP" | "GOTO" | "TRYGOTO"
    );
    if key == "CASE" {
        return HirStatementKind::Instruction {
            target: InstructionTarget::Builtin(key),
            arguments: analyze_case_arguments(
                function,
                source,
                statement,
                raw_arguments,
                symbols,
                catalog,
                context,
                index_resolver,
                options,
                diagnostics,
            ),
        };
    }
    let mut analyzer = ExpressionAnalyzer {
        symbols,
        catalog,
        options,
        function,
        source: source.source.id,
        path: &source.source.relative_path,
        text: &source.text,
        diagnostics,
        index_resolver,
    };
    let mut lowered = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if static_target && index == 0 {
            lowered.push(HirArgument::Raw(resolve_static_target(
                raw_arguments,
                index_resolver,
            )));
            continue;
        }
        lowered.push(match argument {
            Argument::Expression(expression) => {
                if ((key == "ARRAYSORT" && index == 1) || (key == "SORTCHARA" && index <= 1))
                    && let erabasic_ast::ExprKind::Identifier(order) = &expression.kind
                    && matches!(order.to_ascii_uppercase().as_str(), "FORWARD" | "BACK")
                {
                    lowered.push(HirArgument::Raw(order.to_ascii_uppercase()));
                    continue;
                }
                let expression = analyzer.analyze(expression);
                let constant_form = if index == 0 && key.contains("FORMS") {
                    match &expression.constant {
                        Some(ConstantValue::String(value)) => Some(value.clone()),
                        Some(ConstantValue::Integer(_)) | None => None,
                    }
                } else {
                    None
                };
                if let Some(template) = constant_form {
                    // FORMS instructions evaluate their string expression first and then parse
                    // the result as FORM text in the current function scope. Constant templates
                    // can be lowered into ordinary formatted HIR without adding a runtime parser.
                    let expression_span = expression.location.span;
                    let mut parsed = erabasic_parser::parse_formatted_at(
                        &template,
                        expression_span.start,
                        context,
                    );
                    for diagnostic in &mut parsed.diagnostics {
                        confine_span(&mut diagnostic.span, expression_span);
                    }
                    for diagnostic in parsed.diagnostics.drain(..) {
                        analyzer.diagnostics.push(map_parser_diagnostic(
                            source.source.id,
                            &source.source.relative_path,
                            &source.text,
                            &diagnostic,
                        ));
                    }
                    if let Some(mut formatted) = parsed.value {
                        confine_formatted_spans(&mut formatted, expression_span);
                        HirArgument::Formatted(analyzer.analyze_formatted(&formatted))
                    } else {
                        HirArgument::Expression(expression)
                    }
                } else {
                    let constraint = signature
                        .and_then(|signature| {
                            signature.arguments.get(index).or_else(|| {
                                signature
                                    .variadic
                                    .then(|| signature.arguments.last())
                                    .flatten()
                            })
                        })
                        .or_else(|| {
                            method_signature.and_then(|signature| {
                                signature.arguments.get(index).or_else(|| {
                                    signature
                                        .variadic
                                        .then(|| signature.arguments.last())
                                        .flatten()
                                })
                            })
                        });
                    let mutable = constraint.is_some_and(|constraint| {
                        matches!(
                            constraint,
                            crate::ArgumentConstraint::MutableInteger
                                | crate::ArgumentConstraint::MutableString
                                | crate::ArgumentConstraint::MutableAny
                                | crate::ArgumentConstraint::ReferenceAny
                                | crate::ArgumentConstraint::ReferenceOrString
                                | crate::ArgumentConstraint::MutableReferenceOrString
                        ) || *constraint == crate::ArgumentConstraint::IntegerOrMutableString
                            && expression.value_type == SemanticType::String
                    });
                    if mutable {
                        if let HirExprKind::Variable { place } = expression.kind {
                            HirArgument::Place(place)
                        } else {
                            HirArgument::Expression(expression)
                        }
                    } else {
                        HirArgument::Expression(expression)
                    }
                }
            }
            Argument::MixedExpression { expression, is_px } => {
                let expression = analyzer.analyze(expression);
                if key == "PRINT_IMG" && expression.value_type == SemanticType::String {
                    HirArgument::Expression(expression)
                } else {
                    HirArgument::MixedExpression {
                        expression,
                        is_px: *is_px,
                    }
                }
            }
            Argument::Formatted(formatted) => {
                HirArgument::Formatted(analyzer.analyze_formatted(formatted))
            }
            Argument::Raw(value) => HirArgument::Raw(value.clone()),
            Argument::Omitted(_) => HirArgument::Omitted,
        });
    }
    if static_target && lowered.is_empty() && !raw_arguments.trim().is_empty() {
        lowered.push(HirArgument::Raw(resolve_static_target(
            raw_arguments,
            index_resolver,
        )));
    }
    if static_target && lowered.len() > 1 && matches!(lowered.last(), Some(HirArgument::Omitted)) {
        // Emuera treats a final comma after a static CALL/JUMP target as the end
        // of its argument list, not as an extra omitted user-function argument.
        lowered.pop();
    }
    if matches!(key.as_str(), "IF" | "ELSEIF" | "SIF" | "WHILE" | "REPEAT")
        && matches!(lowered.last(), Some(HirArgument::Omitted))
    {
        // The reference condition builders consume their first term and tolerate
        // a dangling comma left by translated scripts.
        lowered.pop();
    }
    if let Some(signature) = signature {
        let expression_arguments: Vec<_> = lowered
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression)
                | HirArgument::MixedExpression { expression, .. } => Some(expression.clone()),
                HirArgument::Place(place) => Some(erabasic_hir::HirExpr {
                    kind: HirExprKind::Variable {
                        place: place.clone(),
                    },
                    value_type: place.value_type,
                    constant: None,
                    location: place.location,
                }),
                HirArgument::Formatted(value) => Some(HirExpr {
                    kind: HirExprKind::Formatted {
                        value: value.clone(),
                    },
                    value_type: SemanticType::String,
                    constant: None,
                    location: value.location,
                }),
                HirArgument::Omitted | HirArgument::Raw(_) => None,
            })
            .collect();
        if !matches!(
            signature.argument_style,
            erabasic_parser::ArgumentStyle::Formatted
                | erabasic_parser::ArgumentStyle::Raw
                | erabasic_parser::ArgumentStyle::Times
                | erabasic_parser::ArgumentStyle::DynamicCall
        ) && !static_target
        {
            analyzer.check_arguments(
                &expression_arguments,
                &signature.arguments,
                signature.minimum_arguments,
                signature.variadic,
                signature.allow_omitted,
                SourceLocation::new(source.source.id, statement.span),
            );
        } else if lowered.len() < signature.minimum_arguments {
            diagnostics.push(AnalyzerDiagnostic::at(
                AnalyzerDiagnosticCode::InvalidArgumentCount,
                AnalyzerDiagnosticSeverity::Error,
                2,
                source.source.id,
                &source.source.relative_path,
                &source.text,
                statement.span,
                format!(
                    "{} requires at least {} arguments",
                    key, signature.minimum_arguments
                ),
            ));
        }
    } else if let Some(signature) = method_signature {
        let expression_arguments: Vec<_> = lowered
            .iter()
            .map(|argument| match argument {
                HirArgument::Expression(expression)
                | HirArgument::MixedExpression { expression, .. } => Some(expression.clone()),
                HirArgument::Place(place) => Some(erabasic_hir::HirExpr {
                    kind: HirExprKind::Variable {
                        place: place.clone(),
                    },
                    value_type: place.value_type,
                    constant: None,
                    location: place.location,
                }),
                HirArgument::Omitted | HirArgument::Formatted(_) | HirArgument::Raw(_) => None,
            })
            .collect();
        if key.contains("FORM") && !key.contains("FORMS") {
            if lowered.len() < signature.minimum_arguments {
                diagnostics.push(AnalyzerDiagnostic::at(
                    AnalyzerDiagnosticCode::InvalidArgumentCount,
                    AnalyzerDiagnosticSeverity::Error,
                    2,
                    source.source.id,
                    &source.source.relative_path,
                    &source.text,
                    statement.span,
                    format!(
                        "{} requires at least {} arguments",
                        key, signature.minimum_arguments
                    ),
                ));
            }
        } else {
            analyzer.check_arguments(
                &expression_arguments,
                &signature.arguments,
                signature.minimum_arguments,
                signature.variadic,
                signature.allow_omitted,
                SourceLocation::new(source.source.id, statement.span),
            );
        }
    }
    let target = if let Some(method_signature) = method_signature {
        InstructionTarget::BuiltinMethod {
            name: key,
            return_type: method_signature.return_type,
        }
    } else if signature.is_none() {
        InstructionTarget::Unresolved(key)
    } else if catalog.extension_instructions.contains(&key) {
        InstructionTarget::Extension(key)
    } else {
        InstructionTarget::Builtin(key)
    };
    HirStatementKind::Instruction {
        target,
        arguments: lowered,
    }
}
