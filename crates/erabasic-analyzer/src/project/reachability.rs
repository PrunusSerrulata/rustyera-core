use std::collections::{BTreeMap, BTreeSet, VecDeque};

use erabasic_ast::Function as AstFunction;
use erabasic_hir::{
    EventAttributes, Function, FunctionId, FunctionKind, SemanticType, SourceLocation,
};

use crate::{
    AnalyzerDiagnostic, AnalyzerDiagnosticCode, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    WarningPolicy, context::AnalysisParserContext, expression::IndexResolver, symbols::Symbols,
};

use super::{ParsedProjectSource, statement_analysis::FunctionDefinition};

mod dynamic;

use dynamic::{CandidateIndex, collect_calls};

pub(super) fn reachable_functions(
    sources: &[ParsedProjectSource],
    definitions: &[FunctionDefinition],
    symbols: &Symbols,
    options: &AnalyzerOptions,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
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
    let mut candidates = CandidateIndex::new(definitions, sources, options.ignore_case);
    let mut queue: VecDeque<_> = reachable.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        let Some(definition) = by_id.get(&id) else {
            continue;
        };
        let function =
            &sources[definition.source_index].script.functions[definition.function_index];
        let calls = collect_calls(function, symbols, id, context, index_resolver);
        for call in calls.direct {
            if let Some(target) = symbols.function(&call)
                && reachable.insert(target.id)
            {
                queue.push_back(target.id);
            }
        }
        for pattern in calls.dynamic {
            if !pattern.is_bounded() {
                return definitions.iter().map(|definition| definition.id).collect();
            }
            if let Some(exact) = pattern.exact() {
                if let Some(target) = symbols.function(&exact)
                    && reachable.insert(target.id)
                {
                    queue.push_back(target.id);
                }
                continue;
            }
            for target in candidates.resolve(&pattern) {
                if reachable.insert(*target) {
                    queue.push_back(*target);
                }
            }
        }
    }
    reachable
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

pub(super) fn function_semantics(
    function: &AstFunction,
    compatibility: &erabasic_compat::CompatibilityIdentity,
) -> (FunctionKind, SemanticType) {
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
    if is_event_name(&upper)
        || (compatibility.supports_fault_hooks()
            && matches!(upper.as_str(), "BEFORE_ERROR" | "BEFORE_THROW"))
    {
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
