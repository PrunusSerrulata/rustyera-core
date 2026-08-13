//! Byte-offset helpers shared by nested parser entry points.

use erabasic_ast::{Diagnostic, Span};
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
