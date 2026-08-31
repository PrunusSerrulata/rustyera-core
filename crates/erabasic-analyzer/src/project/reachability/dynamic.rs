use std::collections::BTreeMap;

use erabasic_ast::{
    Argument, BinaryOp, Expr, ExprKind, FormPart, FormattedString, Function, Statement,
    StatementKind,
};
use erabasic_hir::{FunctionId, SemanticType};

use crate::{
    context::AnalysisParserContext, expression::IndexResolver, identifiers::identifier_key,
    symbols::Symbols,
};

use super::super::{
    ParsedProjectSource, lowering_support::resolve_static_target,
    statement_analysis::FunctionDefinition,
};

const MAX_DYNAMIC_EXPRESSION_DEPTH: usize = 64;

#[derive(Default)]
pub(super) struct Calls {
    pub(super) direct: Vec<String>,
    pub(super) dynamic: Vec<NamePattern>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum NameSegment {
    Literal(String),
    Unknown,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct NamePattern {
    segments: Vec<NameSegment>,
}

impl NamePattern {
    fn literal(value: impl Into<String>) -> Self {
        Self {
            segments: vec![NameSegment::Literal(value.into())],
        }
    }

    fn unknown() -> Self {
        Self {
            segments: vec![NameSegment::Unknown],
        }
    }

    fn append(&mut self, other: Self) {
        if self.segments.len().saturating_add(other.segments.len()) > 256 {
            *self = Self::unknown();
            return;
        }
        for segment in other.segments {
            if let (Some(NameSegment::Literal(previous)), NameSegment::Literal(value)) =
                (self.segments.last_mut(), &segment)
            {
                previous.push_str(value);
            } else {
                self.segments.push(segment);
            }
        }
    }

    pub(super) fn exact(&self) -> Option<String> {
        self.segments
            .iter()
            .map(|segment| match segment {
                NameSegment::Literal(value) => Some(value.as_str()),
                NameSegment::Unknown => None,
            })
            .collect()
    }

    pub(super) fn is_bounded(&self) -> bool {
        self.exact().is_some_and(|value| !value.is_empty())
            || self
                .segments
                .iter()
                .any(|segment| matches!(segment, NameSegment::Literal(value) if !value.is_empty()))
    }

    fn normalized(&self, ignore_case: bool) -> Self {
        Self {
            segments: self
                .segments
                .iter()
                .map(|segment| match segment {
                    NameSegment::Literal(value) => {
                        NameSegment::Literal(identifier_key(value, ignore_case))
                    }
                    NameSegment::Unknown => NameSegment::Unknown,
                })
                .collect(),
        }
    }

    fn matches_normalized(&self, candidate: &str) -> bool {
        if let Some(exact) = self.exact() {
            return exact == candidate;
        }
        let mut literals = self
            .segments
            .iter()
            .filter_map(|segment| match segment {
                NameSegment::Literal(value) => Some(value.as_str()),
                NameSegment::Unknown => None,
            })
            .collect::<std::collections::VecDeque<_>>();
        let mut offset = 0;
        let mut end = candidate.len();
        if matches!(self.segments.first(), Some(NameSegment::Literal(_))) {
            let prefix = literals.pop_front().unwrap_or_default();
            if !candidate.starts_with(prefix) {
                return false;
            }
            offset = prefix.len();
        }
        if matches!(self.segments.last(), Some(NameSegment::Literal(_))) {
            let suffix = literals.pop_back().unwrap_or_default();
            if !candidate.ends_with(suffix) {
                return false;
            }
            end = end.saturating_sub(suffix.len());
        }
        if offset > end {
            return false;
        }
        for literal in literals {
            let Some(found) = candidate[offset..end].find(literal) else {
                return false;
            };
            offset += found + literal.len();
        }
        true
    }
}

pub(super) struct CandidateIndex {
    names: Vec<(FunctionId, String)>,
    ignore_case: bool,
    cache: BTreeMap<NamePattern, Vec<FunctionId>>,
}

impl CandidateIndex {
    pub(super) fn new(
        definitions: &[FunctionDefinition],
        sources: &[ParsedProjectSource],
        ignore_case: bool,
    ) -> Self {
        Self {
            names: definitions
                .iter()
                .map(|definition| {
                    let function = &sources[definition.source_index].script.functions
                        [definition.function_index];
                    (definition.id, identifier_key(&function.name, ignore_case))
                })
                .collect(),
            ignore_case,
            cache: BTreeMap::new(),
        }
    }

    pub(super) fn resolve(&mut self, pattern: &NamePattern) -> &[FunctionId] {
        let key = pattern.normalized(self.ignore_case);
        if !self.cache.contains_key(&key) {
            let matches = self
                .names
                .iter()
                .filter(|(_, name)| key.matches_normalized(name))
                .map(|(id, _)| *id)
                .collect();
            self.cache.insert(key.clone(), matches);
        }
        self.cache.get(&key).expect("candidate pattern was cached")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FunctionList {
    Calls,
    Labels,
}

pub(super) fn collect_calls(
    function: &Function,
    symbols: &Symbols,
    function_id: FunctionId,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
) -> Calls {
    let mut calls = Calls::default();
    let private_types_pending = function
        .attributes
        .iter()
        .any(|directive| matches!(directive.name.as_str(), "DIM" | "DIMS"))
        || function.body.iter().any(|statement| {
            matches!(
                &statement.kind,
                StatementKind::Instruction { name, .. } if matches!(name.as_str(), "VARI" | "VARS")
            )
        });
    let mut list = None;
    for statement in &function.body {
        if let StatementKind::Instruction { name, .. } = &statement.kind {
            list = match name.as_str() {
                "TRYCALLLIST" | "TRYJUMPLIST" => Some(FunctionList::Calls),
                "TRYGOTOLIST" => Some(FunctionList::Labels),
                "ENDFUNC" => None,
                _ => list,
            };
        }
        collect_statement_calls(statement, &mut calls, context, index_resolver, list);
        collect_numeric_assignment_calls(
            statement,
            &mut calls,
            symbols,
            function_id,
            context,
            private_types_pending,
        );
    }
    calls
}

fn collect_numeric_assignment_calls(
    statement: &Statement,
    calls: &mut Calls,
    symbols: &Symbols,
    function: FunctionId,
    context: &AnalysisParserContext,
    private_types_pending: bool,
) {
    let StatementKind::Assignment {
        target,
        op: erabasic_ast::AssignOp::Assign,
        value,
        raw_value,
        ..
    } = &statement.kind
    else {
        return;
    };
    if !private_types_pending
        && symbols
            .resolve_variable(function, &target.name)
            .is_some_and(|variable| variable.value_type == SemanticType::String)
    {
        return;
    }
    let parsed = erabasic_parser::parse_expression_list_at(raw_value, value.span.start, context);
    for expression in parsed.value.iter().flatten() {
        collect_expression_calls(expression, calls, context, 0);
    }
}

fn collect_statement_calls(
    statement: &Statement,
    calls: &mut Calls,
    context: &AnalysisParserContext,
    index_resolver: &IndexResolver,
    list: Option<FunctionList>,
) {
    match &statement.kind {
        StatementKind::Instruction {
            name,
            raw_arguments,
            arguments,
        } => {
            if matches!(
                name.as_str(),
                "CALL" | "CALLF" | "JUMP" | "BEGIN" | "TRYCALL" | "TRYCALLF" | "TRYJUMP"
            ) {
                let target = resolve_static_target(raw_arguments, index_resolver);
                if !target.is_empty() {
                    calls.direct.push(target);
                }
            } else if (name == "FUNC" && list == Some(FunctionList::Calls))
                || matches!(
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
            {
                calls.dynamic.push(
                    arguments
                        .first()
                        .map_or_else(NamePattern::unknown, argument_pattern),
                );
            } else if matches!(
                name.as_str(),
                "CALLSTR" | "JUMPSTR" | "TRYCALLSTR" | "TRYJUMPSTR" | "TRYCCALLSTR" | "TRYCJUMPSTR"
            ) {
                calls.dynamic.push(
                    arguments
                        .first()
                        .and_then(argument_exact_text)
                        .and_then(|text| call_text_target(&text))
                        .map_or_else(NamePattern::unknown, NamePattern::literal),
                );
            }
            for argument in arguments {
                match argument {
                    Argument::Expression(expression)
                    | Argument::MixedExpression { expression, .. } => {
                        collect_expression_calls(expression, calls, context, 0);
                    }
                    Argument::Formatted(value) => {
                        collect_formatted_calls(value, calls, context, 0);
                    }
                    Argument::Raw(_) | Argument::Omitted(_) => {}
                }
            }
        }
        StatementKind::Assignment { value, target, .. } => {
            collect_expression_calls(value, calls, context, 0);
            for index in &target.indices {
                collect_expression_calls(index, calls, context, 0);
            }
        }
        StatementKind::GotoLabel { .. } | StatementKind::Directive(_) | StatementKind::Invalid => {}
    }
}

fn collect_expression_calls(
    expression: &Expr,
    calls: &mut Calls,
    context: &AnalysisParserContext,
    dynamic_depth: usize,
) {
    match &expression.kind {
        ExprKind::Call { name, args } => {
            let upper = name.to_ascii_uppercase();
            if matches!(upper.as_str(), "GETMETH" | "GETMETHS" | "EXISTMETH") {
                calls.dynamic.push(
                    args.first()
                        .and_then(Option::as_ref)
                        .map_or_else(NamePattern::unknown, |value| expression_pattern(value, 0)),
                );
            } else if matches!(upper.as_str(), "STRFORM" | "STRFORMCHECK")
                || (upper == "EXISTVAR" && existvar_evaluates_expression(args))
            {
                collect_runtime_expression_source(
                    args.first().and_then(Option::as_ref),
                    calls,
                    context,
                    dynamic_depth,
                );
            } else {
                calls.direct.push(name.clone());
            }
            for argument in args.iter().flatten() {
                collect_expression_calls(argument, calls, context, dynamic_depth);
            }
        }
        ExprKind::Variable { indices, .. } => {
            for index in indices {
                collect_expression_calls(index, calls, context, dynamic_depth);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => {
            collect_expression_calls(operand, calls, context, dynamic_depth);
        }
        ExprKind::Binary { left, right, .. } => {
            collect_expression_calls(left, calls, context, dynamic_depth);
            collect_expression_calls(right, calls, context, dynamic_depth);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_expression_calls(condition, calls, context, dynamic_depth);
            collect_expression_calls(then_expr, calls, context, dynamic_depth);
            collect_expression_calls(else_expr, calls, context, dynamic_depth);
        }
        ExprKind::Formatted(value) => {
            collect_formatted_calls(value, calls, context, dynamic_depth);
        }
        ExprKind::Integer(_) | ExprKind::String(_) | ExprKind::Identifier(_) | ExprKind::Error => {}
    }
}

fn collect_formatted_calls(
    value: &FormattedString,
    calls: &mut Calls,
    context: &AnalysisParserContext,
    dynamic_depth: usize,
) {
    for part in &value.parts {
        match part {
            FormPart::StringInterpolation {
                expression, width, ..
            }
            | FormPart::IntegerInterpolation {
                expression, width, ..
            } => {
                collect_expression_calls(expression, calls, context, dynamic_depth);
                if let Some(width) = width {
                    collect_expression_calls(width, calls, context, dynamic_depth);
                }
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                ..
            } => {
                collect_expression_calls(condition, calls, context, dynamic_depth);
                collect_formatted_calls(then_value, calls, context, dynamic_depth);
                if let Some(else_value) = else_value {
                    collect_formatted_calls(else_value, calls, context, dynamic_depth);
                }
            }
            FormPart::Text(_) | FormPart::Triple { .. } => {}
        }
    }
}

fn collect_runtime_expression_source(
    source: Option<&Expr>,
    calls: &mut Calls,
    context: &AnalysisParserContext,
    dynamic_depth: usize,
) {
    if dynamic_depth >= MAX_DYNAMIC_EXPRESSION_DEPTH {
        calls.dynamic.push(NamePattern::unknown());
        return;
    }
    let Some(source_expression) = source else {
        calls.dynamic.push(NamePattern::unknown());
        return;
    };
    let Some(source) = exact_expression_text(source_expression) else {
        calls.dynamic.push(NamePattern::unknown());
        return;
    };
    let parsed =
        erabasic_parser::parse_expression_list_at(&source, source_expression.span.start, context);
    if parsed.has_errors() || parsed.value.is_none() {
        calls.dynamic.push(NamePattern::unknown());
        return;
    }
    for expression in parsed.value.iter().flatten() {
        collect_expression_calls(expression, calls, context, dynamic_depth + 1);
    }
}

fn existvar_evaluates_expression(arguments: &[Option<Expr>]) -> bool {
    match arguments.get(1).and_then(Option::as_ref) {
        None
        | Some(Expr {
            kind: ExprKind::Integer(0),
            ..
        }) => false,
        Some(_) => true,
    }
}

fn call_text_target(source: &str) -> Option<String> {
    let source = source.trim();
    let end = source
        .char_indices()
        .find_map(|(index, character)| {
            (character == '(' || character == ',' || character.is_whitespace()).then_some(index)
        })
        .unwrap_or(source.len());
    let target = source[..end].trim().trim_matches('"');
    (!target.is_empty()).then(|| target.to_owned())
}

fn argument_pattern(argument: &Argument) -> NamePattern {
    match argument {
        Argument::Formatted(value) => formatted_pattern(value, 0),
        Argument::Expression(value)
        | Argument::MixedExpression {
            expression: value, ..
        } => expression_pattern(value, 0),
        Argument::Raw(value) => NamePattern::literal(value.trim()),
        Argument::Omitted(_) => NamePattern::unknown(),
    }
}

fn argument_exact_text(argument: &Argument) -> Option<String> {
    match argument {
        Argument::Formatted(value) => formatted_pattern(value, 0).exact(),
        Argument::Expression(value)
        | Argument::MixedExpression {
            expression: value, ..
        } => exact_expression_text(value),
        Argument::Raw(value) => Some(value.trim().to_owned()),
        Argument::Omitted(_) => None,
    }
}

fn expression_pattern(expression: &Expr, depth: usize) -> NamePattern {
    if depth >= MAX_DYNAMIC_EXPRESSION_DEPTH {
        return NamePattern::unknown();
    }
    match &expression.kind {
        ExprKind::String(value) => NamePattern::literal(value),
        ExprKind::Integer(value) => NamePattern::literal(value.to_string()),
        ExprKind::Group(value) => expression_pattern(value, depth + 1),
        ExprKind::Formatted(value) => formatted_pattern(value, depth + 1),
        ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => {
            let mut pattern = expression_pattern(left, depth + 1);
            pattern.append(expression_pattern(right, depth + 1));
            pattern
        }
        ExprKind::Call { name, args } if name.eq_ignore_ascii_case("TOSTR") && args.len() == 1 => {
            args[0].as_ref().map_or_else(NamePattern::unknown, |value| {
                expression_pattern(value, depth + 1)
            })
        }
        _ => NamePattern::unknown(),
    }
}

fn exact_expression_text(expression: &Expr) -> Option<String> {
    expression_pattern(expression, 0).exact()
}

fn formatted_pattern(value: &FormattedString, depth: usize) -> NamePattern {
    if depth >= MAX_DYNAMIC_EXPRESSION_DEPTH {
        return NamePattern::unknown();
    }
    let mut result = NamePattern {
        segments: Vec::new(),
    };
    for part in &value.parts {
        result.append(match part {
            FormPart::Text(value) => NamePattern::literal(value),
            FormPart::StringInterpolation {
                expression,
                width: None,
                alignment: None,
                ..
            }
            | FormPart::IntegerInterpolation {
                expression,
                width: None,
                alignment: None,
                ..
            } => expression_pattern(expression, depth + 1),
            FormPart::StringInterpolation { .. }
            | FormPart::IntegerInterpolation { .. }
            | FormPart::Conditional { .. }
            | FormPart::Triple { .. } => NamePattern::unknown(),
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_ast::Span;

    #[test]
    fn normalized_patterns_match_ordered_literals_without_allocating_candidates() {
        let mut prefix = NamePattern::literal("PRE_");
        prefix.append(NamePattern::unknown());
        assert!(prefix.normalized(true).matches_normalized("PRE_関数"));

        let mut middle = NamePattern::unknown();
        middle.append(NamePattern::literal("_中_"));
        middle.append(NamePattern::unknown());
        assert!(middle.normalized(true).matches_normalized("前_中_後"));

        let mut suffix = NamePattern::unknown();
        suffix.append(NamePattern::literal("_END"));
        assert!(suffix.normalized(true).matches_normalized("関数_END"));
        assert!(!suffix.normalized(true).matches_normalized("END_関数"));

        let mut pattern = NamePattern::literal("pre_");
        pattern.append(NamePattern::unknown());
        pattern.append(NamePattern::literal("_中_"));
        pattern.append(NamePattern::unknown());
        pattern.append(NamePattern::literal("_END"));
        let pattern = pattern.normalized(true);
        assert!(pattern.matches_normalized("PRE_a_中_b_END"));
        assert!(!pattern.matches_normalized("PRE_a_END_中_b"));
        assert!(!pattern.matches_normalized("PRE_a_中_b_end"));
        assert_eq!(identifier_key("関数名", true), "関数名");
    }

    #[test]
    fn candidate_index_caches_equivalent_case_folded_patterns() {
        let mut index = CandidateIndex {
            names: vec![
                (FunctionId(1), identifier_key("Target_One", true)),
                (FunctionId(2), identifier_key("Other", true)),
            ],
            ignore_case: true,
            cache: BTreeMap::new(),
        };
        let mut lower = NamePattern::literal("target_");
        lower.append(NamePattern::unknown());
        let mut upper = NamePattern::literal("TARGET_");
        upper.append(NamePattern::unknown());
        assert_eq!(index.resolve(&lower), &[FunctionId(1)]);
        assert_eq!(index.resolve(&upper), &[FunctionId(1)]);
        assert_eq!(index.cache.len(), 1);
    }

    #[test]
    fn runtime_expression_depth_limit_conservatively_becomes_unbounded() {
        let options = crate::AnalyzerOptions::default();
        let catalog = crate::catalog::Catalog::build(&crate::ExtensionRegistry::default());
        let context = AnalysisParserContext::new(
            &erabasic_data::ProjectSchema::builtin_defaults(),
            &catalog,
            std::iter::empty(),
            &options,
        );
        let source = Expr {
            kind: ExprKind::String("HELPER()".into()),
            span: Span::new(0, 8),
        };
        let mut calls = Calls::default();
        collect_runtime_expression_source(
            Some(&source),
            &mut calls,
            &context,
            MAX_DYNAMIC_EXPRESSION_DEPTH,
        );
        assert_eq!(calls.dynamic.len(), 1);
        assert!(!calls.dynamic[0].is_bounded());
    }
}
