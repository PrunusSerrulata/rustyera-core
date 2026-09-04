use erabasic_ast::{Argument, Expr, ExprKind, Span, Statement, StatementKind};
use erabasic_hir::{
    FunctionId, HirArgument, HirExprKind, HirStatement, HirStatementKind, InstructionTarget,
    LabelId, LineId, SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::Symbols,
};

use super::super::{ParsedProjectSource, source_support::map_parser_diagnostic};
use super::instructions::analyze_instruction;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) fn analyze_statement(
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
