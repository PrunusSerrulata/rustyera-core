use erabasic_ast::{Diagnostic, DiagnosticCode, Span};

use super::{Lexer, lex_with};
use crate::{FormattedToken, FormattedTokenPart, LexEnd, LexFlags, Token};

impl Lexer<'_> {
    pub(super) fn read_formatted_quoted(&mut self) {
        let start = self.pos;
        self.pos += 2;
        let formatted = self.read_formatted_until(FormEnd::Quote, start);
        self.push(crate::TokenKind::Formatted(formatted), start, self.pos);
    }

    pub(super) fn read_formatted_until(&mut self, end: FormEnd, start: usize) -> FormattedToken {
        let mut parts = Vec::new();
        let mut text = String::new();
        let mut closed = false;
        while let Some(ch) = self.current() {
            if end.matches(self) {
                if !text.is_empty() {
                    parts.push(FormattedTokenPart::Text(std::mem::take(&mut text)));
                }
                end.consume(self);
                closed = true;
                break;
            }
            if matches!(ch, '\r' | '\n') {
                break;
            }
            if "*+=/$".contains(ch)
                && self.source[self.pos..].chars().take(3).all(|c| c == ch)
                && self.source[self.pos..].chars().take(3).count() == 3
            {
                if !text.is_empty() {
                    parts.push(FormattedTokenPart::Text(std::mem::take(&mut text)));
                }
                let item_start = self.pos;
                for _ in 0..3 {
                    self.bump();
                }
                parts.push(FormattedTokenPart::Triple {
                    symbol: ch,
                    span: Span::new(item_start, self.pos),
                });
                continue;
            }
            if ch == '%' {
                if !text.is_empty() {
                    parts.push(FormattedTokenPart::Text(std::mem::take(&mut text)));
                }
                let item_start = self.pos;
                self.bump();
                let nested = self.lex_nested(LexEnd::Percent);
                if self.current() == Some('%') {
                    self.bump();
                } else {
                    self.unterminated_form(item_start, "missing closing '%' in formatted string");
                }
                parts.push(FormattedTokenPart::StringInterpolation {
                    tokens: nested,
                    span: Span::new(item_start, self.pos),
                });
                continue;
            }
            if ch == '{' {
                if !text.is_empty() {
                    parts.push(FormattedTokenPart::Text(std::mem::take(&mut text)));
                }
                let item_start = self.pos;
                self.bump();
                let nested = self.lex_nested(LexEnd::RightCurlyBrace);
                if self.current() == Some('}') {
                    self.bump();
                } else {
                    self.unterminated_form(item_start, "missing closing '}' in formatted string");
                }
                parts.push(FormattedTokenPart::IntegerInterpolation {
                    tokens: nested,
                    span: Span::new(item_start, self.pos),
                });
                continue;
            }
            if ch == '\\' && self.peek() == Some('@') {
                if !text.is_empty() {
                    parts.push(FormattedTokenPart::Text(std::mem::take(&mut text)));
                }
                parts.push(self.read_conditional_form());
                continue;
            }
            if ch == '\\' {
                self.bump();
                if let Some(escaped) = self.current() {
                    text.push(escaped);
                    self.bump();
                }
            } else {
                text.push(ch);
                self.bump();
            }
        }
        if !text.is_empty() {
            parts.push(FormattedTokenPart::Text(text));
        }
        if !closed && !matches!(end, FormEnd::EndOfLine) {
            self.unterminated_form(start, "unterminated formatted string");
        }
        FormattedToken {
            parts,
            span: Span::new(start, self.pos),
        }
    }

    pub(super) fn read_conditional_form(&mut self) -> FormattedTokenPart {
        let start = self.pos;
        self.pos += 2;
        let condition = self.lex_nested(LexEnd::Question);
        if self.current() == Some('?') {
            self.bump();
        }
        let then_start = self.pos;
        let then_value = self.read_formatted_until(FormEnd::SharpOrYenAt, then_start);
        let else_value = if self.source[..self.pos].ends_with('#') {
            let else_start = self.pos;
            Some(Box::new(
                self.read_formatted_until(FormEnd::YenAt, else_start),
            ))
        } else {
            None
        };
        FormattedTokenPart::Conditional {
            condition,
            then_value: Box::new(then_value),
            else_value,
            span: Span::new(start, self.pos),
        }
    }

    fn lex_nested(&mut self, end: LexEnd) -> Vec<Token> {
        let suffix = &self.source[self.pos..];
        let nested = lex_with(suffix, self.config, end, LexFlags::NONE, self.macros);
        let consumed = nested.consumed;
        let base = self.pos;
        self.output
            .diagnostics
            .extend(nested.diagnostics.into_iter().map(|mut diagnostic| {
                diagnostic.span =
                    Span::new(diagnostic.span.start + base, diagnostic.span.end + base);
                diagnostic
            }));
        self.pos += consumed;
        nested
            .tokens
            .into_iter()
            .map(|mut token| {
                token.span = Span::new(token.span.start + base, token.span.end + base);
                token
            })
            .collect()
    }

    fn unterminated_form(&mut self, start: usize, message: &'static str) {
        self.output.diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnterminatedFormattedString,
            Span::new(start, self.pos),
            message,
        ));
    }
}

#[derive(Clone, Copy)]
pub(super) enum FormEnd {
    Quote,
    SharpOrYenAt,
    YenAt,
    EndOfLine,
}

impl FormEnd {
    fn matches(self, lexer: &Lexer<'_>) -> bool {
        match self {
            Self::Quote => lexer.current() == Some('"'),
            Self::SharpOrYenAt => {
                lexer.current() == Some('#') || lexer.source[lexer.pos..].starts_with("\\@")
            }
            Self::YenAt => lexer.source[lexer.pos..].starts_with("\\@"),
            Self::EndOfLine => false,
        }
    }
    fn consume(self, lexer: &mut Lexer<'_>) {
        match self {
            Self::Quote => lexer.bump(),
            Self::SharpOrYenAt if lexer.current() == Some('#') => lexer.bump(),
            Self::SharpOrYenAt | Self::YenAt => lexer.pos += 2,
            Self::EndOfLine => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LexerConfig, TokenKind, lex};

    #[test]
    fn lexes_formatted_string_parts() {
        let output = lex("@\"value={X} name=%NAME% ***\"", &LexerConfig::default());
        let TokenKind::Formatted(form) = &output.tokens[0].kind else {
            panic!("expected form")
        };
        assert_eq!(form.parts.len(), 6);
    }

    #[test]
    fn lexes_conditional_form_as_an_expression_term() {
        let output = lex("\\@ FLAG ? yes # no \\@", &LexerConfig::default());
        assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
        assert!(matches!(
            output.tokens.as_slice(),
            [crate::Token {
                kind: TokenKind::Formatted(_),
                ..
            }]
        ));
    }
}
