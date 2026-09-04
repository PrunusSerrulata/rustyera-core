use erabasic_ast::Expr;
use erabasic_hir::{
    ConstantValue, FunctionId, HirExpr, HirExprKind, HirPlace, HirStatement, HirStatementKind,
    LineId, SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::Symbols,
};

use super::super::{ParsedProjectSource, source_support::map_parser_diagnostic};

#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_runtime_initializer(
    line_id: LineId,
    function: FunctionId,
    source: &ParsedProjectSource,
    runtime: &crate::symbols::FunctionRuntimeInitializer,
    symbols: &Symbols,
    catalog: &Catalog,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> Vec<HirStatement> {
    let location = runtime.initializer.location;
    let Some(variable) = symbols.variables.get(runtime.variable.0 as usize) else {
        return vec![HirStatement {
            id: line_id,
            kind: HirStatementKind::Error,
            location,
        }];
    };
    let mut parsed = erabasic_parser::parse_expression_list_at(
        &runtime.initializer.source,
        location.span.start,
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
    let Some(mut expressions) = parsed.value else {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::InvalidInitializer,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            location.span,
            "dynamic private initializer must contain at least one expression",
        ));
        return vec![HirStatement {
            id: line_id,
            kind: HirStatementKind::Error,
            location,
        }];
    };
    expressions.truncate(runtime.initializer.value_count);
    if expressions.is_empty() {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::InvalidInitializer,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            location.span,
            "dynamic private initializer must contain at least one expression",
        ));
        return vec![HirStatement {
            id: line_id,
            kind: HirStatementKind::Error,
            location,
        }];
    }
    let value_count = expressions.len();
    expressions
        .iter()
        .enumerate()
        .map(|(index, expression)| {
            analyze_runtime_initializer_value(
                line_id,
                index,
                value_count,
                function,
                expression,
                variable,
                source,
                symbols,
                catalog,
                index_resolver,
                options,
                diagnostics,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn analyze_runtime_initializer_value(
    first_line_id: LineId,
    index: usize,
    value_count: usize,
    function: FunctionId,
    expression: &Expr,
    variable: &erabasic_hir::Variable,
    source: &ParsedProjectSource,
    symbols: &Symbols,
    catalog: &Catalog,
    index_resolver: &IndexResolver,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) -> HirStatement {
    let location = SourceLocation::new(source.source.id, expression.span);
    let value = ExpressionAnalyzer {
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
    .analyze(expression);
    if variable.value_type != value.value_type && value.value_type != SemanticType::Error {
        diagnostics.push(AnalyzerDiagnostic::at(
            AnalyzerDiagnosticCode::TypeMismatch,
            AnalyzerDiagnosticSeverity::Error,
            2,
            source.source.id,
            &source.source.relative_path,
            &source.text,
            expression.span,
            "dynamic private initializer type does not match its declaration",
        ));
    }
    let index_value = i64::try_from(index).expect("initializer index fits in i64");
    let indices = (value_count > 1)
        .then_some(HirExpr {
            kind: HirExprKind::Integer { value: index_value },
            value_type: SemanticType::Integer,
            constant: Some(ConstantValue::Integer(index_value)),
            location,
        })
        .into_iter()
        .collect();
    HirStatement {
        id: LineId(
            first_line_id
                .0
                .checked_add(u32::try_from(index).expect("too many initializer values"))
                .expect("too many lines"),
        ),
        kind: HirStatementKind::Assignment {
            target: HirPlace {
                variable: variable.id,
                indices,
                value_type: variable.value_type,
                mutable: variable.mutable,
                location,
            },
            op: erabasic_ast::AssignOp::Assign,
            value,
        },
        location,
    }
}
