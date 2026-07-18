use std::collections::{BTreeMap, BTreeSet};

use erabasic_data::Persistence;
use erabasic_hir::{
    CallTarget, FunctionId, HirArgument, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirPlace, HirStatementKind, InstructionTarget, Program, SourceLocation,
    VariableId,
};

use crate::catalog::CallablePortability;
use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, ParsedProjectSource,
    builtin_callable_portability,
};

pub(crate) fn analyze(
    program: &Program,
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    let persistence = program
        .variables
        .iter()
        .map(|variable| (variable.id, variable.persistence))
        .collect::<BTreeMap<_, _>>();
    let return_taint = summarize_return_taint(program);
    emit_diagnostics(program, sources, diagnostics, &persistence, &return_taint);
}

fn summarize_return_taint(program: &Program) -> BTreeMap<FunctionId, bool> {
    let mut return_taint = BTreeMap::<FunctionId, bool>::new();

    // Function summaries reach a deterministic fixed point before diagnostics
    // are emitted. Recursive groups therefore behave the same regardless of
    // source or collection traversal order.
    loop {
        let mut changed = false;
        for function in &program.functions {
            let mut variables = BTreeSet::new();
            let mut tainted_return = false;
            for line in &function.lines {
                match &line.kind {
                    HirStatementKind::Assignment { target, value, .. } => {
                        if expression_tainted(value, &variables, &return_taint) {
                            variables.insert(target.variable);
                        }
                    }
                    HirStatementKind::Instruction { target, arguments }
                        if target.name().eq_ignore_ascii_case("RETURN") =>
                    {
                        tainted_return |= arguments
                            .iter()
                            .any(|argument| argument_tainted(argument, &variables, &return_taint));
                    }
                    _ => {}
                }
            }
            if tainted_return && !return_taint.get(&function.id).copied().unwrap_or(false) {
                return_taint.insert(function.id, true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    return_taint
}

fn emit_diagnostics(
    program: &Program,
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    persistence: &BTreeMap<VariableId, Persistence>,
    return_taint: &BTreeMap<FunctionId, bool>,
) {
    for function in &program.functions {
        // Taint is scoped to one function. Interprocedural propagation is carried
        // only by the fixed-point return summaries above.
        let mut variables = BTreeSet::<VariableId>::new();
        let mut tainted_control_depth = 0usize;
        for line in &function.lines {
            match &line.kind {
                HirStatementKind::Assignment { target, value, .. } => {
                    emit_expression_notices(value, sources, diagnostics);
                    for index in &target.indices {
                        emit_expression_notices(index, sources, diagnostics);
                    }
                    let value_tainted = expression_tainted(value, &variables, return_taint);
                    let index_tainted = target
                        .indices
                        .iter()
                        .any(|index| expression_tainted(index, &variables, return_taint));
                    if value_tainted || tainted_control_depth != 0 {
                        variables.insert(target.variable);
                    }
                    if (value_tainted || index_tainted || tainted_control_depth != 0)
                        && persistence
                            .get(&target.variable)
                            .copied()
                            .unwrap_or(Persistence::None)
                            != Persistence::None
                    {
                        dependency(
                            sources,
                            diagnostics,
                            line.location,
                            "frontend observation influences persistent game state",
                        );
                    }
                }
                HirStatementKind::Instruction { target, arguments } => {
                    let name = target.name().to_ascii_uppercase();
                    for argument in arguments {
                        emit_argument_notices(argument, sources, diagnostics);
                    }
                    let tainted = arguments
                        .iter()
                        .any(|argument| argument_tainted(argument, &variables, return_taint));
                    if tainted && is_dependency_sink(&name) {
                        dependency(
                            sources,
                            diagnostics,
                            line.location,
                            format!("frontend observation influences {name}"),
                        );
                    }
                    if tainted && is_control_opener(&name) {
                        tainted_control_depth = tainted_control_depth.saturating_add(1);
                    }
                    if is_control_closer(&name) {
                        tainted_control_depth = tainted_control_depth.saturating_sub(1);
                    }
                    if let InstructionTarget::Builtin(name) = target
                        && builtin_callable_portability(name)
                            == CallablePortability::FrontendObservation
                    {
                        source_notice(sources, diagnostics, line.location, name);
                    }
                }
                HirStatementKind::Label { .. } | HirStatementKind::Error => {}
            }
        }
    }
}

fn expression_tainted(
    expression: &HirExpr,
    variables: &BTreeSet<VariableId>,
    return_taint: &BTreeMap<FunctionId, bool>,
) -> bool {
    match &expression.kind {
        HirExprKind::Variable { place } => place_tainted(place, variables, return_taint),
        HirExprKind::Call { target, arguments } => {
            let direct = match target {
                CallTarget::Builtin { name }
                    if builtin_callable_portability(name)
                        == CallablePortability::FrontendObservation =>
                {
                    true
                }
                CallTarget::User { function } => {
                    return_taint.get(function).copied().unwrap_or(false)
                }
                CallTarget::Builtin { .. }
                | CallTarget::Extension { .. }
                | CallTarget::Unresolved { .. } => false,
            };
            direct
                || arguments.iter().any(|argument| match argument {
                    HirCallArgument::Value(value) => {
                        expression_tainted(value, variables, return_taint)
                    }
                    HirCallArgument::Place(place) => place_tainted(place, variables, return_taint),
                    HirCallArgument::Omitted => false,
                })
        }
        HirExprKind::Unary { operand, .. } | HirExprKind::Postfix { operand, .. } => {
            expression_tainted(operand, variables, return_taint)
        }
        HirExprKind::Binary { left, right, .. } => {
            expression_tainted(left, variables, return_taint)
                || expression_tainted(right, variables, return_taint)
        }
        HirExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            expression_tainted(condition, variables, return_taint)
                || expression_tainted(then_expr, variables, return_taint)
                || expression_tainted(else_expr, variables, return_taint)
        }
        HirExprKind::Formatted { value } => formatted_tainted(value, variables, return_taint),
        HirExprKind::Integer { .. } | HirExprKind::String { .. } | HirExprKind::Error => false,
    }
}

fn emit_expression_notices(
    expression: &HirExpr,
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    match &expression.kind {
        HirExprKind::Call { target, arguments } => {
            if let CallTarget::Builtin { name } = target
                && builtin_callable_portability(name) == CallablePortability::FrontendObservation
            {
                source_notice(sources, diagnostics, expression.location, name);
            }
            for argument in arguments {
                match argument {
                    HirCallArgument::Value(value) => {
                        emit_expression_notices(value, sources, diagnostics);
                    }
                    HirCallArgument::Place(place) => {
                        for index in &place.indices {
                            emit_expression_notices(index, sources, diagnostics);
                        }
                    }
                    HirCallArgument::Omitted => {}
                }
            }
        }
        HirExprKind::Variable { place } => {
            for index in &place.indices {
                emit_expression_notices(index, sources, diagnostics);
            }
        }
        HirExprKind::Unary { operand, .. } | HirExprKind::Postfix { operand, .. } => {
            emit_expression_notices(operand, sources, diagnostics);
        }
        HirExprKind::Binary { left, right, .. } => {
            emit_expression_notices(left, sources, diagnostics);
            emit_expression_notices(right, sources, diagnostics);
        }
        HirExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            emit_expression_notices(condition, sources, diagnostics);
            emit_expression_notices(then_expr, sources, diagnostics);
            emit_expression_notices(else_expr, sources, diagnostics);
        }
        HirExprKind::Formatted { value } => {
            emit_formatted_notices(value, sources, diagnostics);
        }
        HirExprKind::Integer { .. } | HirExprKind::String { .. } | HirExprKind::Error => {}
    }
}

fn emit_formatted_notices(
    value: &HirFormattedString,
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    for part in &value.parts {
        match part {
            HirFormPart::Interpolation {
                expression, width, ..
            } => {
                emit_expression_notices(expression, sources, diagnostics);
                if let Some(width) = width {
                    emit_expression_notices(width, sources, diagnostics);
                }
            }
            HirFormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                emit_expression_notices(condition, sources, diagnostics);
                emit_formatted_notices(then_value, sources, diagnostics);
                if let Some(value) = else_value {
                    emit_formatted_notices(value, sources, diagnostics);
                }
            }
            HirFormPart::Text { .. } | HirFormPart::Triple { .. } => {}
        }
    }
}

fn emit_argument_notices(
    argument: &HirArgument,
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    match argument {
        HirArgument::Expression(value)
        | HirArgument::MixedExpression {
            expression: value, ..
        } => emit_expression_notices(value, sources, diagnostics),
        HirArgument::Place(place) => {
            for index in &place.indices {
                emit_expression_notices(index, sources, diagnostics);
            }
        }
        HirArgument::Formatted(value) => emit_formatted_notices(value, sources, diagnostics),
        HirArgument::Raw(_) | HirArgument::Omitted => {}
    }
}

fn place_tainted(
    place: &HirPlace,
    variables: &BTreeSet<VariableId>,
    return_taint: &BTreeMap<FunctionId, bool>,
) -> bool {
    variables.contains(&place.variable)
        || place
            .indices
            .iter()
            .any(|index| expression_tainted(index, variables, return_taint))
}

fn formatted_tainted(
    value: &HirFormattedString,
    variables: &BTreeSet<VariableId>,
    return_taint: &BTreeMap<FunctionId, bool>,
) -> bool {
    value.parts.iter().any(|part| match part {
        HirFormPart::Interpolation {
            expression, width, ..
        } => {
            expression_tainted(expression, variables, return_taint)
                || width
                    .as_ref()
                    .is_some_and(|width| expression_tainted(width, variables, return_taint))
        }
        HirFormPart::Conditional {
            condition,
            then_value,
            else_value,
            ..
        } => {
            expression_tainted(condition, variables, return_taint)
                || formatted_tainted(then_value, variables, return_taint)
                || else_value
                    .as_ref()
                    .is_some_and(|value| formatted_tainted(value, variables, return_taint))
        }
        HirFormPart::Text { .. } | HirFormPart::Triple { .. } => false,
    })
}

fn argument_tainted(
    argument: &HirArgument,
    variables: &BTreeSet<VariableId>,
    return_taint: &BTreeMap<FunctionId, bool>,
) -> bool {
    match argument {
        HirArgument::Expression(value)
        | HirArgument::MixedExpression {
            expression: value, ..
        } => expression_tainted(value, variables, return_taint),
        HirArgument::Place(place) => place_tainted(place, variables, return_taint),
        HirArgument::Formatted(value) => formatted_tainted(value, variables, return_taint),
        HirArgument::Raw(_) | HirArgument::Omitted => false,
    }
}

fn is_control_opener(name: &str) -> bool {
    matches!(
        name,
        "IF" | "SIF" | "WHILE" | "REPEAT" | "FOR" | "SELECTCASE"
    )
}

fn is_control_closer(name: &str) -> bool {
    matches!(name, "ENDIF" | "WEND" | "REND" | "NEXT" | "ENDSELECT")
}

fn is_dependency_sink(name: &str) -> bool {
    is_control_opener(name)
        || name.contains("CALLFORM")
        || name.contains("JUMPFORM")
        || name.contains("GOTOFORM")
        || matches!(name, "RANDOMIZE" | "INITRAND")
        || matches!(
            name,
            "SAVEDATA" | "SAVEGLOBAL" | "SAVEVAR" | "SAVECHARA" | "SAVETEXT" | "SAVEGAME"
        )
}

fn source_notice(
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    location: SourceLocation,
    name: &str,
) {
    emit(
        sources,
        diagnostics,
        location,
        AnalyzerDiagnosticCode::FrontendObservationSource,
        AnalyzerDiagnosticSeverity::Notice,
        format!("{name} observes the authoritative frontend and may vary across clients"),
    );
}

fn dependency(
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    location: SourceLocation,
    message: impl Into<String>,
) {
    emit(
        sources,
        diagnostics,
        location,
        AnalyzerDiagnosticCode::FrontendObservationDependency,
        AnalyzerDiagnosticSeverity::Warning,
        message,
    );
}

fn emit(
    sources: &[ParsedProjectSource],
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
    location: SourceLocation,
    code: AnalyzerDiagnosticCode,
    severity: AnalyzerDiagnosticSeverity,
    message: impl Into<String>,
) {
    let Some(source) = sources
        .iter()
        .find(|source| source.source.id == location.source)
    else {
        return;
    };
    diagnostics.push(AnalyzerDiagnostic::at(
        code,
        severity,
        0,
        location.source,
        &source.source.relative_path,
        &source.text,
        location.span,
        message,
    ));
}
