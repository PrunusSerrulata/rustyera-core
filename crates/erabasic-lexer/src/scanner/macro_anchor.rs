//! Macro-expansion source anchoring for plain and recursively formatted tokens.

use erabasic_ast::Span;

use crate::{FormattedToken, FormattedTokenPart, Token, TokenKind};

pub(super) fn anchor_macro_token(token: &mut Token, invocation: Span) {
    token.span = invocation;
    token.from_macro = true;
    if let TokenKind::Formatted(formatted) = &mut token.kind {
        anchor_macro_formatted(formatted, invocation);
    }
}

fn anchor_macro_formatted(formatted: &mut FormattedToken, invocation: Span) {
    formatted.span = invocation;
    for part in &mut formatted.parts {
        match part {
            FormattedTokenPart::Text(_) => {}
            FormattedTokenPart::StringInterpolation { tokens, span }
            | FormattedTokenPart::IntegerInterpolation { tokens, span } => {
                *span = invocation;
                for token in tokens {
                    anchor_macro_token(token, invocation);
                }
            }
            FormattedTokenPart::Conditional {
                condition,
                then_value,
                else_value,
                span,
            } => {
                *span = invocation;
                for token in condition {
                    anchor_macro_token(token, invocation);
                }
                anchor_macro_formatted(then_value, invocation);
                if let Some(else_value) = else_value {
                    anchor_macro_formatted(else_value, invocation);
                }
            }
            FormattedTokenPart::Triple { span, .. } => *span = invocation,
        }
    }
}
