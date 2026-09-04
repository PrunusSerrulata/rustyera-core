use erabasic_ast::{Argument, Statement};
use erabasic_hir::{
    ConstantValue, FunctionId, HirArgument, HirExpr, HirExprKind, HirStatementKind,
    InstructionTarget, SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    expression::{ExpressionAnalyzer, IndexResolver},
    identifiers::identifier_key,
    symbols::Symbols,
};

use super::super::{
    ParsedProjectSource,
    lowering_support::{
        analyze_case_arguments, analyze_scoped_declaration_statement, resolve_static_target,
    },
    source_support::{confine_formatted_spans, confine_span, map_parser_diagnostic},
};

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub(super) fn analyze_instruction(
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
    let key = identifier_key(name, options.ignore_case);
    let signature = catalog.instructions.get(&key).filter(|_| {
        catalog.extension_instructions.contains(&key)
            || crate::catalog::builtin_instruction_available(&key, &options.compatibility)
    });
    let method_signature = signature
        .is_none()
        .then(|| catalog.functions.get(&key))
        .flatten()
        .filter(|_| {
            catalog.extension_functions.contains(&key)
                || crate::catalog::builtin_function_available(&key, &options.compatibility)
        });
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
    if matches!(key.as_str(), "MATCHALL" | "MATCHALLEX") && method_signature.is_some() {
        analyzer.check_match_source(
            &key,
            &arguments
                .iter()
                .map(|arg| match arg {
                    Argument::Expression(expr)
                    | Argument::MixedExpression {
                        expression: expr, ..
                    } => Some(expr),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            SourceLocation::new(source.source.id, statement.span),
        );
    }
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
                                let constraints = signature.arguments_for_arity(arguments.len());
                                constraints.get(index).or_else(|| {
                                    signature.variadic.then(|| constraints.last()).flatten()
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
    if key == "DT_COLUMN_OPTIONS" {
        let expression_arguments = lowered
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| {
                if index >= 2 && index % 2 == 0 {
                    return None;
                }
                Some(match argument {
                    HirArgument::Expression(expression) => Some(expression.clone()),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        analyzer.check_arguments(
            &expression_arguments,
            &[
                crate::ArgumentConstraint::String,
                crate::ArgumentConstraint::String,
                crate::ArgumentConstraint::Any,
            ],
            3,
            true,
            false,
            SourceLocation::new(source.source.id, statement.span),
        );
        return HirStatementKind::Instruction {
            target: InstructionTarget::Builtin(key),
            arguments: lowered,
        };
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
    if matches!(
        key.as_str(),
        "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYJUMP"
    ) && let Some(HirArgument::Raw(target)) = lowered.first()
        && let Some(callee) = symbols.function(target)
    {
        analyzer.diagnose_user_call_arity(
            target,
            lowered.len().saturating_sub(1),
            callee.parameter_count,
            SourceLocation::new(source.source.id, statement.span),
        );
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
        analyzer.check_graphics_call(
            &key,
            &expression_arguments,
            SourceLocation::new(source.source.id, statement.span),
        );
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
        if matches!(key.as_str(), "GETMETH" | "GETMETHS")
            && !catalog.extension_functions.contains(&key)
        {
            analyzer.check_dynamic_method_name(
                &expression_arguments,
                SourceLocation::new(source.source.id, statement.span),
            );
        }
        analyzer.check_map_output(
            &key,
            &expression_arguments,
            SourceLocation::new(source.source.id, statement.span),
        );
        analyzer.check_graphics_call(
            &key,
            &expression_arguments,
            SourceLocation::new(source.source.id, statement.span),
        );
        analyzer.check_bit_call(
            &key,
            &expression_arguments,
            SourceLocation::new(source.source.id, statement.span),
        );
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
                signature.arguments_for_arity(expression_arguments.len()),
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
