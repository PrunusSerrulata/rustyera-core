use std::collections::BTreeSet;

use erabasic_ast::{Expr, ExprKind, Function as AstFunction, VariableRef};
use erabasic_hir::{
    ConstantValue, Function, FunctionId, FunctionKind, HirExpr, HirExprKind, LineId, Parameter,
    SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    catalog::Catalog,
    context::AnalysisParserContext,
    control_flow::build_control_flow,
    expression::{ExpressionAnalyzer, IndexResolver},
    symbols::Symbols,
};

use super::{ParsedProjectSource, reachability::event_attributes};

mod instructions;
mod runtime_initializers;
mod statements;

use runtime_initializers::analyze_runtime_initializer;
use statements::analyze_statement;

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
    for runtime in symbols.runtime_initializers(id) {
        let line_id = LineId(u32::try_from(lines.len()).expect("too many lines"));
        lines.extend(analyze_runtime_initializer(
            line_id,
            id,
            source,
            runtime,
            symbols,
            catalog,
            context,
            index_resolver,
            options,
            diagnostics,
        ));
    }
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
