use std::collections::{BTreeMap, BTreeSet, VecDeque};

use erabasic_ast::{Argument, Expr, ExprKind, Function as AstFunction, Statement, StatementKind};
use erabasic_hir::{
    EventAttributes, Function, FunctionId, FunctionKind, SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    WarningPolicy, symbols::Symbols,
};

use super::{
    ParsedProjectSource, lowering_support::static_target_source,
    statement_analysis::FunctionDefinition,
};

pub(super) fn reachable_functions(
    sources: &[ParsedProjectSource],
    definitions: &[FunctionDefinition],
    symbols: &Symbols,
    options: &AnalyzerOptions,
) -> BTreeSet<FunctionId> {
    if options.analysis_mode || !options.ignore_uncalled_functions {
        return definitions.iter().map(|definition| definition.id).collect();
    }
    let mut reachable: BTreeSet<_> = definitions
        .iter()
        .filter(|definition| matches!(definition.kind, FunctionKind::Event | FunctionKind::System))
        .map(|definition| definition.id)
        .collect();
    let by_id: BTreeMap<_, _> = definitions
        .iter()
        .map(|definition| (definition.id, definition))
        .collect();
    let mut queue: VecDeque<_> = reachable.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        let Some(definition) = by_id.get(&id) else {
            continue;
        };
        let function =
            &sources[definition.source_index].script.functions[definition.function_index];
        if function.body.iter().any(uses_dynamic_call) {
            reachable.extend(definitions.iter().map(|definition| definition.id));
            break;
        }
        let mut calls = Vec::new();
        for statement in &function.body {
            collect_statement_calls(statement, &mut calls);
        }
        for call in calls {
            if let Some(target) = symbols.function(&call)
                && reachable.insert(target.id)
            {
                queue.push_back(target.id);
            }
        }
    }
    reachable
}

fn collect_statement_calls(statement: &Statement, calls: &mut Vec<String>) {
    match &statement.kind {
        StatementKind::Instruction {
            name,
            raw_arguments,
            arguments,
        } => {
            if matches!(
                name.as_str(),
                "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYJUMP"
            ) {
                let target = static_target_source(raw_arguments).trim().trim_matches('"');
                if !target.is_empty() {
                    calls.push(target.to_owned());
                }
            }
            for argument in arguments {
                if let Argument::Expression(expression)
                | Argument::MixedExpression { expression, .. } = argument
                {
                    collect_expression_calls(expression, calls);
                }
            }
        }
        StatementKind::Assignment { value, target, .. } => {
            collect_expression_calls(value, calls);
            for index in &target.indices {
                collect_expression_calls(index, calls);
            }
        }
        StatementKind::GotoLabel { .. } | StatementKind::Directive(_) | StatementKind::Invalid => {}
    }
}

fn collect_expression_calls(expression: &Expr, calls: &mut Vec<String>) {
    match &expression.kind {
        ExprKind::Call { name, args } => {
            calls.push(name.clone());
            for argument in args.iter().flatten() {
                collect_expression_calls(argument, calls);
            }
        }
        ExprKind::Variable { indices, .. } => {
            for index in indices {
                collect_expression_calls(index, calls);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => {
            collect_expression_calls(operand, calls);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expression_calls(left, calls);
            collect_expression_calls(right, calls);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_expression_calls(condition, calls);
            collect_expression_calls(then_expr, calls);
            collect_expression_calls(else_expr, calls);
        }
        ExprKind::Integer(_)
        | ExprKind::String(_)
        | ExprKind::Identifier(_)
        | ExprKind::Formatted(_)
        | ExprKind::Error => {}
    }
}

fn uses_dynamic_call(statement: &Statement) -> bool {
    // Emuera parses every function once a runtime-resolved call target is reachable.
    // Keep this list aligned with the cross-function dynamic lowering paths so the
    // IgnoreUncalledFunction optimization cannot discard a possible target body.
    matches!(
        &statement.kind,
        StatementKind::Instruction { name, .. }
            if matches!(
                name.as_str(),
                "CALLFORM"
                    | "CALLFORMF"
                    | "JUMPFORM"
                    | "TRYCALLFORM"
                    | "TRYCALLFORMF"
                    | "TRYJUMPFORM"
                    | "TRYCCALL"
                    | "TRYCCALLFORM"
                    | "TRYCJUMP"
                    | "TRYCJUMPFORM"
            )
    )
}

pub(super) fn uncalled_function(
    id: FunctionId,
    kind: FunctionKind,
    return_type: SemanticType,
    definition_order: u32,
    source: &ParsedProjectSource,
    function: &AstFunction,
) -> Function {
    Function {
        id,
        name: function.name.clone(),
        kind,
        event_attributes: event_attributes(kind, function),
        definition_order,
        return_type,
        parameters: Vec::new(),
        lines: Vec::new(),
        labels: Vec::new(),
        control_flow: Vec::new(),
        location: SourceLocation::new(source.source.id, function.span),
    }
}

pub(super) fn event_attributes(kind: FunctionKind, function: &AstFunction) -> EventAttributes {
    if kind != FunctionKind::Event {
        return EventAttributes::default();
    }
    let mut attributes = EventAttributes::default();
    for directive in &function.attributes {
        match directive.name.as_str() {
            "ONLY" if !attributes.only => {
                attributes = EventAttributes {
                    only: true,
                    ..EventAttributes::default()
                };
            }
            "PRI" if !attributes.only => attributes.priority = true,
            "LATER" if !attributes.only => attributes.later = true,
            "SINGLE" if !attributes.only => attributes.single = true,
            _ => {}
        }
    }
    attributes
}

pub(super) fn report_uncalled(
    source: &ParsedProjectSource,
    function: &AstFunction,
    options: &AnalyzerOptions,
    diagnostics: &mut Vec<AnalyzerDiagnostic>,
) {
    if matches!(
        options.function_not_called,
        WarningPolicy::Ignore | WarningPolicy::Later
    ) {
        return;
    }
    diagnostics.push(AnalyzerDiagnostic::at(
        AnalyzerDiagnosticCode::UncalledFunction,
        AnalyzerDiagnosticSeverity::Warning,
        1,
        source.source.id,
        &source.source.relative_path,
        &source.text,
        function.span,
        format!("function {} is never called", function.name),
    ));
}

pub(super) fn function_semantics(function: &AstFunction) -> (FunctionKind, SemanticType) {
    if function
        .attributes
        .iter()
        .any(|directive| directive.name == "FUNCTIONS")
    {
        return (FunctionKind::Method, SemanticType::String);
    }
    if function
        .attributes
        .iter()
        .any(|directive| directive.name == "FUNCTION")
    {
        return (FunctionKind::Method, SemanticType::Integer);
    }
    let upper = function.name.to_ascii_uppercase();
    if is_event_name(&upper) {
        (FunctionKind::Event, SemanticType::Void)
    } else if is_system_name(&upper) {
        (FunctionKind::System, SemanticType::Void)
    } else {
        (FunctionKind::Normal, SemanticType::Void)
    }
}

fn is_event_name(name: &str) -> bool {
    matches!(
        name,
        "EVENTFIRST"
            | "EVENTTRAIN"
            | "EVENTSHOP"
            | "EVENTBUY"
            | "EVENTCOM"
            | "EVENTTURNEND"
            | "EVENTCOMEND"
            | "EVENTEND"
            | "EVENTLOAD"
    )
}

fn is_system_name(name: &str) -> bool {
    is_event_name(name)
        || matches!(
            name,
            "SHOW_STATUS"
                | "SHOW_USERCOM"
                | "USERCOM"
                | "SOURCE_CHECK"
                | "CALLTRAINEND"
                | "SHOW_JUEL"
                | "SHOW_ABLUP_SELECT"
                | "USERABLUP"
                | "SHOW_SHOP"
                | "SAVEINFO"
                | "USERSHOP"
                | "TITLE_LOADGAME"
                | "SYSTEM_AUTOSAVE"
                | "SYSTEM_TITLE"
                | "SYSTEM_LOADEND"
        )
        || numbered_system_name(name, "COM")
        || numbered_system_name(name, "COM_ABLE")
        || numbered_system_name(name, "ABLUP")
}

fn numbered_system_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}
