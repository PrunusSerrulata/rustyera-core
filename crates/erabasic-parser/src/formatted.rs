use erabasic_ast::{Alignment, Diagnostic, Expr, FormPart, FormattedString, ParseOutput};
use erabasic_lexer::{FormattedToken, FormattedTokenPart, Token, TokenKind, lex_formatted};

use crate::ParserContext;
use crate::expression::ExpressionParser;
use crate::util::{map_formatted_spans, shifted, split_top_level};

/// Parse FORM text and place every span at its UTF-8 byte offset in the source file.
#[must_use]
pub fn parse_formatted_at(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<FormattedString> {
    let (form, lex_diagnostics) = lex_formatted(source, context.lexer_config(), context.macros());
    let mut output = lower_formatted(&form);
    output.diagnostics.splice(0..0, lex_diagnostics);
    shift_formatted(&mut output, base);
    output
}

/// Parse the FORM right-hand side of a plain string assignment.
///
/// Emuera trims ASCII spaces and tabs from the decoded outer text fragments,
/// including whitespace produced by FORM escapes such as `\s` and `\t`.
#[must_use]
pub fn parse_assignment_formatted_at(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<FormattedString> {
    let mut output = parse_formatted_at(source, base, context);
    if let Some(form) = output.value.as_mut() {
        trim_outer_text(form);
    }
    output
}

fn trim_outer_text(form: &mut FormattedString) {
    if let Some(FormPart::Text(text)) = form.parts.first_mut() {
        *text = text.trim_start_matches([' ', '\t']).to_owned();
    }
    if let Some(FormPart::Text(text)) = form.parts.last_mut() {
        *text = text.trim_end_matches([' ', '\t']).to_owned();
    }
    form.parts
        .retain(|part| !matches!(part, FormPart::Text(text) if text.is_empty()));
}

pub(crate) fn lower_formatted(form: &FormattedToken) -> ParseOutput<FormattedString> {
    let mut diagnostics = Vec::new();
    let mut parts = Vec::new();
    for part in &form.parts {
        match part {
            FormattedTokenPart::Text(value) => parts.push(FormPart::Text(value.clone())),
            FormattedTokenPart::Triple { symbol, span } => parts.push(FormPart::Triple {
                symbol: *symbol,
                span: *span,
            }),
            FormattedTokenPart::StringInterpolation { tokens, span }
            | FormattedTokenPart::IntegerInterpolation { tokens, span } => {
                let segments = split_top_level(tokens, ',');
                let expression = parse_token_segment(
                    segments.first().copied().unwrap_or_default(),
                    &mut diagnostics,
                );
                let width = segments
                    .get(1)
                    .and_then(|s| parse_token_segment(s, &mut diagnostics))
                    .map(Box::new);
                let alignment = segments.get(2).and_then(|tokens| tokens.first()).and_then(
                    |token| match &token.kind {
                        TokenKind::Identifier(value) if value.eq_ignore_ascii_case("LEFT") => {
                            Some(Alignment::Left)
                        }
                        TokenKind::Identifier(value) if value.eq_ignore_ascii_case("RIGHT") => {
                            Some(Alignment::Right)
                        }
                        _ => None,
                    },
                );
                let Some(expression) = expression else {
                    continue;
                };
                if matches!(part, FormattedTokenPart::StringInterpolation { .. }) {
                    parts.push(FormPart::StringInterpolation {
                        expression: Box::new(expression),
                        width,
                        alignment,
                        span: *span,
                    });
                } else {
                    parts.push(FormPart::IntegerInterpolation {
                        expression: Box::new(expression),
                        width,
                        alignment,
                        span: *span,
                    });
                }
            }
            FormattedTokenPart::Conditional {
                condition,
                then_value,
                else_value,
                span,
            } => {
                let Some(condition) = parse_token_segment(condition, &mut diagnostics) else {
                    continue;
                };
                let mut then_output = lower_formatted(then_value);
                diagnostics.append(&mut then_output.diagnostics);
                let mut else_output = else_value.as_deref().map(lower_formatted);
                if let Some(output) = else_output.as_mut() {
                    diagnostics.append(&mut output.diagnostics);
                }
                if let Some(then_value) = then_output.value {
                    parts.push(FormPart::Conditional {
                        condition: Box::new(condition),
                        then_value: Box::new(then_value),
                        else_value: else_output.and_then(|o| o.value).map(Box::new),
                        span: *span,
                    });
                }
            }
        }
    }
    ParseOutput {
        value: Some(FormattedString {
            parts,
            span: form.span,
        }),
        diagnostics,
    }
}

fn parse_token_segment(tokens: &[Token], diagnostics: &mut Vec<Diagnostic>) -> Option<Expr> {
    let mut parser = ExpressionParser::new(tokens);
    let value = parser.parse();
    diagnostics.append(&mut parser.diagnostics);
    value
}

pub(crate) fn shift_formatted(output: &mut ParseOutput<FormattedString>, base: usize) {
    for diagnostic in &mut output.diagnostics {
        diagnostic.span = shifted(diagnostic.span, base);
    }
    if let Some(form) = &mut output.value {
        map_formatted_spans(form, &|span| shifted(span, base));
    }
}
