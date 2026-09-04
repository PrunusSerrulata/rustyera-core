//! Byte-offset helpers shared by nested parser entry points.

use erabasic_ast::{Diagnostic, Expr, ExprKind, FormPart, FormattedString, Span};
use erabasic_lexer::Token;

pub(crate) fn shifted(span: Span, base: usize) -> Span {
    Span::new(span.start + base, span.end + base)
}

pub(crate) fn shift_tokens(tokens: Vec<Token>, base: usize) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|mut token| {
            token.span = shifted(token.span, base);
            token
        })
        .collect()
}

pub(crate) fn shift_diagnostics(diagnostics: Vec<Diagnostic>, base: usize) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            diagnostic.span = shifted(diagnostic.span, base);
            diagnostic
        })
        .collect()
}

pub(crate) fn map_expression_spans(expression: &mut Expr, map: &impl Fn(Span) -> Span) {
    expression.span = map(expression.span);
    match &mut expression.kind {
        ExprKind::Variable { indices, .. } => {
            for index in indices {
                map_expression_spans(index, map);
            }
        }
        ExprKind::Call { args, .. } => {
            for argument in args.iter_mut().flatten() {
                map_expression_spans(argument, map);
            }
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Postfix { operand, .. }
        | ExprKind::Group(operand) => map_expression_spans(operand, map),
        ExprKind::Binary { left, right, .. } => {
            map_expression_spans(left, map);
            map_expression_spans(right, map);
        }
        ExprKind::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            map_expression_spans(condition, map);
            map_expression_spans(then_expr, map);
            map_expression_spans(else_expr, map);
        }
        ExprKind::Formatted(formatted) => map_formatted_spans(formatted, map),
        ExprKind::Integer(_) | ExprKind::String(_) | ExprKind::Identifier(_) | ExprKind::Error => {}
    }
}

pub(crate) fn map_formatted_spans(formatted: &mut FormattedString, map: &impl Fn(Span) -> Span) {
    formatted.span = map(formatted.span);
    for part in &mut formatted.parts {
        match part {
            FormPart::StringInterpolation {
                expression,
                width,
                span,
                ..
            }
            | FormPart::IntegerInterpolation {
                expression,
                width,
                span,
                ..
            } => {
                *span = map(*span);
                map_expression_spans(expression, map);
                if let Some(width) = width {
                    map_expression_spans(width, map);
                }
            }
            FormPart::Conditional {
                condition,
                then_value,
                else_value,
                span,
            } => {
                *span = map(*span);
                map_expression_spans(condition, map);
                map_formatted_spans(then_value, map);
                if let Some(else_value) = else_value {
                    map_formatted_spans(else_value, map);
                }
            }
            FormPart::Triple { span, .. } => *span = map(*span),
            FormPart::Text(_) => {}
        }
    }
}

pub(crate) fn lines_with_offsets(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut next = (!source.is_empty()).then_some(0);
    std::iter::from_fn(move || {
        let start = next?;
        let delimiter = source.as_bytes()[start..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map(|offset| start + offset);
        let Some(end) = delimiter else {
            next = None;
            return Some((start, &source[start..]));
        };
        let delimiter_length = usize::from(
            source.as_bytes()[end] == b'\r' && source.as_bytes().get(end + 1) == Some(&b'\n'),
        ) + 1;
        next = (end + delimiter_length < source.len()).then_some(end + delimiter_length);
        Some((start, &source[start..end]))
    })
}
