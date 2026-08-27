//! Structural name patterns. Unknown segments widen candidates; they never prove a call valid.

use erabasic_ast::{Argument, BinaryOp, Expr, ExprKind, FormPart, FormattedString};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(super) enum Segment {
    Literal(String),
    Unknown(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct Pattern {
    pub segments: Vec<Segment>,
}

impl Pattern {
    pub fn literal(value: &str) -> Self {
        Self {
            segments: vec![Segment::Literal(value.into())],
        }
    }
    fn unknown(reason: &str) -> Self {
        Self {
            segments: vec![Segment::Unknown(reason.into())],
        }
    }
    fn append(&mut self, other: Self) {
        if self.segments.len().saturating_add(other.segments.len()) > 256 {
            *self = Self::unknown("pattern_complexity_limit_all_symbols_retained");
        } else {
            for segment in other.segments {
                if let (Some(Segment::Literal(previous)), Segment::Literal(value)) =
                    (self.segments.last_mut(), &segment)
                {
                    previous.push_str(value);
                } else {
                    self.segments.push(segment);
                }
            }
        }
    }
    pub fn exact(&self) -> Option<String> {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Literal(value) => Some(value.as_str()),
                Segment::Unknown(_) => None,
            })
            .collect()
    }
    pub fn matches(&self, candidate: &str, ignore_case: bool) -> bool {
        let normalize = |value: &str| {
            if ignore_case {
                value.to_ascii_uppercase()
            } else {
                value.into()
            }
        };
        let candidate = normalize(candidate);
        if let Some(exact) = self.exact() {
            return normalize(&exact) == candidate;
        }
        let mut literals = self
            .segments
            .iter()
            .filter_map(|segment| match segment {
                Segment::Literal(value) => Some(normalize(value)),
                Segment::Unknown(_) => None,
            })
            .collect::<std::collections::VecDeque<_>>();
        let mut offset = 0;
        let mut end = candidate.len();
        if matches!(self.segments.first(), Some(Segment::Literal(_))) {
            let prefix = literals.pop_front().unwrap_or_default();
            if !candidate.starts_with(&prefix) {
                return false;
            }
            offset = prefix.len();
        }
        if matches!(self.segments.last(), Some(Segment::Literal(_))) {
            let suffix = literals.pop_back().unwrap_or_default();
            if !candidate.ends_with(&suffix) {
                return false;
            }
            end -= suffix.len();
        }
        if offset > end {
            return false;
        }
        for literal in literals {
            let Some(found) = candidate[offset..end].find(&literal) else {
                return false;
            };
            offset += found + literal.len();
        }
        true
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Target {
    pub dispatch: String,
    pub namespace: String,
    pub pattern: Pattern,
    pub expected_return: Option<&'static str>,
    pub supplied_slots: usize,
    pub omitted_slots: usize,
    pub executes_body: bool,
}

pub(super) fn expression(name: &str, args: &[Option<Expr>]) -> Target {
    let upper = name.to_ascii_uppercase();
    if upper == "STRFORM" {
        return Target {
            dispatch: "dynamic_form_evaluation".into(),
            namespace: "function".into(),
            pattern: Pattern::unknown("runtime_form_source_may_call_methods_not_statically_parsed"),
            expected_return: None,
            supplied_slots: args.len(),
            omitted_slots: args.iter().filter(|argument| argument.is_none()).count(),
            executes_body: true,
        };
    }
    let dynamic = matches!(upper.as_str(), "GETMETH" | "GETMETHS" | "EXISTMETH");
    Target {
        dispatch: if dynamic {
            "dynamic_method"
        } else {
            "direct_expression"
        }
        .into(),
        namespace: "function".into(),
        pattern: if dynamic {
            args.first().and_then(Option::as_ref).map_or_else(
                || Pattern::unknown("missing_target_expression"),
                |value| expr(value, 0),
            )
        } else {
            Pattern::literal(name)
        },
        expected_return: match upper.as_str() {
            "GETMETH" => Some("integer"),
            "GETMETHS" => Some("string"),
            _ => None,
        },
        supplied_slots: if dynamic {
            args.len().saturating_sub(2)
        } else {
            args.len()
        },
        omitted_slots: args
            .iter()
            .skip(if dynamic { 2 } else { 0 })
            .filter(|argument| argument.is_none())
            .count(),
        executes_body: upper != "EXISTMETH",
    }
}

pub(super) fn instruction(name: &str, arguments: &[Argument]) -> Option<Target> {
    let function = matches!(
        name,
        "CALL"
            | "CALLF"
            | "JUMP"
            | "TRYCALL"
            | "TRYCCALL"
            | "TRYJUMP"
            | "TRYCJUMP"
            | "CALLFORM"
            | "CALLFORMF"
            | "JUMPFORM"
            | "TRYCALLFORM"
            | "TRYCALLFORMF"
            | "TRYCCALLFORM"
            | "TRYJUMPFORM"
            | "TRYCJUMPFORM"
            | "FUNC"
    );
    let label = matches!(
        name,
        "GOTO" | "TRYGOTO" | "TRYCGOTO" | "GOTOFORM" | "TRYGOTOFORM" | "TRYCGOTOFORM"
    );
    if !function && !label {
        return None;
    }
    let pattern = match arguments.first() {
        Some(Argument::Formatted(form)) => formatted(form, 0),
        Some(
            Argument::Expression(value)
            | Argument::MixedExpression {
                expression: value, ..
            },
        ) => expr(value, 0),
        Some(Argument::Raw(value)) => Pattern::literal(value.trim()),
        _ => Pattern::unknown("unparsed_or_omitted_target"),
    };
    Some(Target {
        dispatch: if name.contains("FORM") || name == "FUNC" || pattern.exact().is_none() {
            "dynamic_statement"
        } else {
            "direct_statement"
        }
        .into(),
        namespace: if label { "label" } else { "function" }.into(),
        pattern,
        expected_return: None,
        supplied_slots: arguments.len().saturating_sub(1),
        omitted_slots: arguments
            .iter()
            .skip(1)
            .filter(|argument| matches!(argument, Argument::Omitted(_)))
            .count(),
        executes_body: true,
    })
}

fn expr(value: &Expr, depth: usize) -> Pattern {
    if depth > 64 {
        return Pattern::unknown("expression_depth_limit_all_symbols_retained");
    }
    match &value.kind {
        ExprKind::String(value) => Pattern::literal(value),
        ExprKind::Group(value) => expr(value, depth + 1),
        ExprKind::Formatted(value) => formatted(value, depth + 1),
        ExprKind::Binary {
            op: BinaryOp::Add,
            left,
            right,
        } => {
            let mut result = expr(left, depth + 1);
            result.append(expr(right, depth + 1));
            result
        }
        _ => Pattern::unknown("runtime_value_or_unresolved_expression_type"),
    }
}

fn formatted(form: &FormattedString, depth: usize) -> Pattern {
    if depth > 64 {
        return Pattern::unknown("form_depth_limit_all_symbols_retained");
    }
    let mut result = Pattern {
        segments: Vec::new(),
    };
    for part in &form.parts {
        result.append(match part {
            FormPart::Text(value) => Pattern::literal(value),
            FormPart::StringInterpolation {
                expression,
                width: None,
                ..
            } => expr(expression, depth + 1),
            FormPart::IntegerInterpolation {
                expression,
                width: None,
                ..
            } => match &expression.kind {
                ExprKind::Integer(value) => Pattern::literal(&value.to_string()),
                _ => Pattern::unknown("runtime_integer_interpolation"),
            },
            FormPart::Conditional { .. } => {
                Pattern::unknown("conditional_form_branch_not_evaluated")
            }
            FormPart::Triple { .. } => Pattern::unknown("profile_triple_symbol_expansion"),
            _ => Pattern::unknown("formatted_width_or_alignment_not_evaluated"),
        });
    }
    result
}
