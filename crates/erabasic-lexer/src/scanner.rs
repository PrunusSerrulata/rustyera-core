//! Stateful scanner implementation.
//!
//! `EraBasic` cannot be described as one regular token language: callers choose
//! different terminators, semicolons have debug variants, and formatted strings
//! recursively contain expressions. A small explicit cursor makes those rules
//! visible and keeps error spans exact. This is why the crate does not use
//! `logos` for the stateful outer loop.

use crate::rules::{is_identifier_delimiter, is_identifier_start, operator_at};
use crate::{
    FormattedToken, LexEnd, LexFlags, LexOutput, LexerConfig, MacroTable, Operator, Token,
    TokenKind,
};
use erabasic_ast::{Diagnostic, DiagnosticCode, Span};
use formatted::FormEnd;

mod formatted;

#[must_use]
pub fn lex(source: &str, config: &LexerConfig) -> LexOutput {
    lex_with(
        source,
        config,
        LexEnd::EndOfLine,
        LexFlags::NONE,
        &MacroTable::new(),
    )
}

#[must_use]
pub fn lex_with(
    source: &str,
    config: &LexerConfig,
    end: LexEnd,
    flags: LexFlags,
    macros: &MacroTable,
) -> LexOutput {
    let mut lexer = Lexer::new(source, config, flags, macros);
    lexer.run(end);
    lexer.finish_brackets();
    lexer.output.consumed = lexer.pos;
    lexer.output
}

/// Lex a command-style FORM string that extends to the end of the line.
#[must_use]
pub fn lex_formatted(
    source: &str,
    config: &LexerConfig,
    macros: &MacroTable,
) -> (FormattedToken, Vec<Diagnostic>) {
    let mut lexer = Lexer::new(source, config, LexFlags::NONE, macros);
    let value = lexer.read_formatted_until(FormEnd::EndOfLine, 0);
    (value, lexer.output.diagnostics)
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    config: &'a LexerConfig,
    flags: LexFlags,
    macros: &'a MacroTable,
    paren_depth: usize,
    bracket_depth: usize,
    output: LexOutput,
}

impl<'a> Lexer<'a> {
    fn new(
        source: &'a str,
        config: &'a LexerConfig,
        flags: LexFlags,
        macros: &'a MacroTable,
    ) -> Self {
        Self {
            source,
            pos: 0,
            config,
            flags,
            macros,
            paren_depth: 0,
            bracket_depth: 0,
            output: LexOutput::default(),
        }
    }

    fn run(&mut self, end: LexEnd) {
        while let Some(ch) = self.current() {
            if ch == '\r' || ch == '\n' {
                break;
            }
            if self.at_terminator(end, ch) {
                break;
            }
            if self.skip_space_or_comment() {
                continue;
            }
            let start = self.pos;
            match ch {
                '0'..='9' => self.read_integer(),
                '"' => self.read_string('"', false),
                '\'' if self.flags.contains(LexFlags::ALLOW_SINGLE_QUOTED_STRING) => {
                    self.read_string('\'', false);
                }
                '\'' if self.flags.contains(LexFlags::ANALYZE_PRINT_V) => {
                    self.read_print_v_string();
                }
                '@' if self.peek() == Some('"') => self.read_formatted_quoted(),
                '\\' if self.peek() == Some('@') => {
                    // The reference expression lexer admits a conditional FORM as
                    // a string-valued term, notably inside `%\@ ... \@%`.
                    let part = self.read_conditional_form();
                    self.push(
                        TokenKind::Formatted(FormattedToken {
                            parts: vec![part],
                            span: Span::new(start, self.pos),
                        }),
                        start,
                        self.pos,
                    );
                }
                c if is_identifier_start(c) => self.read_identifier(),
                '[' if self.peek() == Some('[') => self.read_rename_symbol(),
                '(' | '[' => {
                    if ch == '(' {
                        self.paren_depth += 1;
                    } else {
                        self.bracket_depth += 1;
                    }
                    self.bump();
                    self.push(TokenKind::Symbol(ch), start, self.pos);
                }
                ')' | ']' => {
                    if ch == ')' {
                        if self.paren_depth == 0 {
                            self.output.diagnostics.push(Diagnostic::error(
                                DiagnosticCode::UnexpectedToken,
                                Span::new(start, start + ch.len_utf8()),
                                "closing ')' has no matching '('",
                            ));
                        } else {
                            self.paren_depth -= 1;
                        }
                    } else {
                        if self.bracket_depth == 0 {
                            self.output.diagnostics.push(Diagnostic::error(
                                DiagnosticCode::UnexpectedToken,
                                Span::new(start, start + ch.len_utf8()),
                                "closing ']' has no matching '['",
                            ));
                        } else {
                            self.bracket_depth -= 1;
                        }
                    }
                    self.bump();
                    self.push(TokenKind::Symbol(ch), start, self.pos);
                }
                ',' | ':' | '.' | '{' | '}' | '$' | '@' => {
                    self.bump();
                    self.push(TokenKind::Symbol(ch), start, self.pos);
                }
                _ => {
                    if let Some((op, len)) = operator_at(&self.source[self.pos..]) {
                        if matches!(op, Operator::Assign | Operator::StringAssign)
                            && !self.flags.contains(LexFlags::ALLOW_ASSIGNMENT)
                        {
                            self.output.diagnostics.push(Diagnostic::error(
                                DiagnosticCode::InvalidOperator,
                                Span::new(start, start + len),
                                "assignment '=' is not allowed in this expression context",
                            ));
                        }
                        self.pos += len;
                        self.push(TokenKind::Operator(op), start, self.pos);
                    } else {
                        self.bump();
                        self.output.diagnostics.push(Diagnostic::error(
                            DiagnosticCode::UnexpectedCharacter,
                            Span::new(start, self.pos),
                            format!("unexpected character {ch:?}"),
                        ));
                    }
                }
            }
        }
    }

    fn at_terminator(&self, end: LexEnd, ch: char) -> bool {
        let top = self.paren_depth == 0 && self.bracket_depth == 0;
        match end {
            LexEnd::EndOfLine => false,
            LexEnd::Operator => top && operator_at(&self.source[self.pos..]).is_some(),
            LexEnd::Question => top && ch == '?',
            LexEnd::Percent => top && ch == '%',
            LexEnd::RightCurlyBrace => ch == '}',
            // Emuera historically ignores bracket depth for this mode.
            LexEnd::Comma => self.paren_depth == 0 && ch == ',',
            LexEnd::GreaterThan => ch == '>',
        }
    }

    fn skip_space_or_comment(&mut self) -> bool {
        let Some(ch) = self.current() else {
            return false;
        };
        if ch == ' ' || ch == '\t' || (ch == '\u{3000}' && self.config.allow_full_width_space) {
            self.bump();
            return true;
        }
        if ch != ';' {
            return false;
        }
        if self.source[self.pos..].starts_with(";!;")
            || (self.config.debug_semicolon && self.source[self.pos..].starts_with(";#;"))
        {
            self.pos += 3;
            return true;
        }
        self.pos = self.source.len();
        true
    }

    fn finish_brackets(&mut self) {
        if self.paren_depth > 0 {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::empty(self.pos),
                "missing closing ')'",
            ));
        }
        if self.bracket_depth > 0 {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::empty(self.pos),
                "missing closing ']'",
            ));
        }
    }

    fn read_identifier(&mut self) {
        let start = self.pos;
        while let Some(ch) = self.current() {
            if is_identifier_delimiter(ch) {
                break;
            }
            self.bump();
        }
        let value = self.source[start..self.pos].to_string();
        let key = value.to_uppercase();
        if self.config.expand_macros
            && let Some(replacement) = self.macros.get(&key)
        {
            if replacement.iter().any(|t| matches!(&t.kind, TokenKind::Identifier(name) if name.eq_ignore_ascii_case(&value))) {
                    self.output.diagnostics.push(Diagnostic::error(
                        DiagnosticCode::MacroRecursion, Span::new(start, self.pos), "recursive macro expansion",
                    ));
                    return;
                }
            self.output
                .tokens
                .extend(replacement.iter().cloned().map(|mut token| {
                    token.from_macro = true;
                    token
                }));
            return;
        }
        self.push(TokenKind::Identifier(value), start, self.pos);
    }

    fn read_rename_symbol(&mut self) {
        let start = self.pos;
        self.pos += 2;
        let Some(relative_end) = self.source[self.pos..].find("]]") else {
            self.pos = self.source.len();
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::new(start, self.pos),
                "rename symbol is missing closing ']]'",
            ));
            return;
        };
        if relative_end == 0 {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::new(start, self.pos + 2),
                "rename symbol cannot be empty",
            ));
        }
        self.pos += relative_end + 2;
        self.push(
            TokenKind::Identifier(self.source[start..self.pos].to_owned()),
            start,
            self.pos,
        );
    }

    fn read_integer(&mut self) {
        let start = self.pos;
        let (radix, prefix_len) = if self.source[self.pos..].starts_with("0x")
            || self.source[self.pos..].starts_with("0X")
        {
            (16, 2)
        } else if self.source[self.pos..].starts_with("0b")
            || self.source[self.pos..].starts_with("0B")
        {
            (2, 2)
        } else {
            (10, 0)
        };
        self.pos += prefix_len;
        let digits_start = self.pos;
        while self.current().is_some_and(|c| c.is_digit(radix)) {
            self.bump();
        }
        if self.pos == digits_start {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::InvalidInteger,
                Span::new(start, self.pos),
                "integer prefix is not followed by digits",
            ));
            return;
        }
        let digits = &self.source[digits_start..self.pos];
        let Ok(mut value) = i64::from_str_radix(digits, radix) else {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::IntegerOverflow,
                Span::new(start, self.pos),
                "integer literal does not fit in i64",
            ));
            return;
        };
        if self
            .current()
            .is_some_and(|c| matches!(c, 'e' | 'E' | 'p' | 'P'))
        {
            let marker = self.current().unwrap_or('e');
            self.bump();
            let negative = if self.current() == Some('-') {
                self.bump();
                true
            } else {
                if self.current() == Some('+') {
                    self.bump();
                }
                false
            };
            let exp_start = self.pos;
            while self.current().is_some_and(|c| c.is_digit(radix)) {
                self.bump();
            }
            let exponent = u32::from_str_radix(&self.source[exp_start..self.pos], radix);
            let Ok(exponent) = exponent else {
                self.output.diagnostics.push(Diagnostic::error(
                    DiagnosticCode::InvalidInteger,
                    Span::new(start, self.pos),
                    "invalid integer exponent",
                ));
                return;
            };
            if negative {
                let base: i64 = if matches!(marker, 'p' | 'P') { 2 } else { 10 };
                value = base
                    .checked_pow(exponent)
                    .map_or(0, |factor| value / factor);
            } else {
                let base: u128 = if matches!(marker, 'p' | 'P') { 2 } else { 10 };
                let Some(factor) = base.checked_pow(exponent) else {
                    self.integer_overflow(start);
                    return;
                };
                let Ok(unsigned_value) = u128::try_from(value) else {
                    self.integer_overflow(start);
                    return;
                };
                let Some(next) = unsigned_value.checked_mul(factor) else {
                    self.integer_overflow(start);
                    return;
                };
                // The pinned .NET conversion admits exactly 2^63 and converts it to
                // the sign bit. eraTW relies on `1p63` as an i64 mask constant.
                if next == 1_u128 << 63 {
                    value = i64::MIN;
                } else if let Ok(next) = i64::try_from(next) {
                    value = next;
                } else {
                    self.integer_overflow(start);
                    return;
                }
            }
        }
        self.push(TokenKind::Integer(value), start, self.pos);
    }

    fn integer_overflow(&mut self, start: usize) {
        self.output.diagnostics.push(Diagnostic::error(
            DiagnosticCode::IntegerOverflow,
            Span::new(start, self.pos),
            "integer literal does not fit in i64",
        ));
    }

    fn read_string(&mut self, quote: char, stop_at_comma: bool) {
        let start = self.pos;
        self.bump();
        let mut value = String::new();
        let mut closed = false;
        while let Some(ch) = self.current() {
            if ch == quote {
                self.bump();
                closed = true;
                break;
            }
            if stop_at_comma && ch == ',' {
                closed = true;
                break;
            }
            if matches!(ch, '\r' | '\n') {
                break;
            }
            if ch == '\\' {
                self.bump();
                let Some(escaped) = self.current() else {
                    self.output.diagnostics.push(Diagnostic::error(
                        DiagnosticCode::InvalidEscape,
                        Span::new(self.pos, self.pos),
                        "escape is missing its following character",
                    ));
                    break;
                };
                self.bump();
                match escaped {
                    's' => value.push(' '),
                    'S' => value.push('\u{3000}'),
                    't' => value.push('\t'),
                    'n' => value.push('\n'),
                    '\r' => {
                        if self.current() == Some('\n') {
                            self.bump();
                        }
                    }
                    '\n' => {}
                    other => value.push(other),
                }
            } else {
                value.push(ch);
                self.bump();
            }
        }
        if !closed {
            self.output.diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnterminatedString,
                Span::new(start, self.pos),
                "unterminated string literal",
            ));
        }
        self.push(TokenKind::String(value), start, self.pos);
    }

    fn read_print_v_string(&mut self) {
        let start = self.pos;
        self.bump();
        let content_start = self.pos;
        while self
            .current()
            .is_some_and(|c| !matches!(c, ',' | '\r' | '\n'))
        {
            self.bump();
        }
        let value = self.source[content_start..self.pos].to_string();
        self.push(TokenKind::String(value), start, self.pos);
    }

    fn current(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().nth(1)
    }

    fn bump(&mut self) {
        if let Some(ch) = self.current() {
            self.pos += ch.len_utf8();
        }
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.output.tokens.push(Token {
            kind,
            span: Span::new(start, end),
            from_macro: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_unicode_and_longest_operators() {
        let output = lex("変数:TARGET += 0x10", &LexerConfig::default());
        assert!(output.diagnostics.is_empty());
        assert!(matches!(&output.tokens[0].kind, TokenKind::Identifier(s) if s == "変数"));
        assert!(matches!(
            output.tokens[3].kind,
            TokenKind::Operator(Operator::AddAssign)
        ));
    }

    #[test]
    fn handles_emuera_string_escapes() {
        let output = lex("\"a\\sb\\Sc\\n\"", &LexerConfig::default());
        assert_eq!(
            output.tokens[0].kind,
            TokenKind::String("a b\u{3000}c\n".into())
        );
    }

    #[test]
    fn reports_utf8_spans_and_unmatched_brackets() {
        let output = lex("変数 + (1", &LexerConfig::default());
        assert_eq!(output.tokens[0].span, Span::new(0, 6));
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::UnexpectedToken)
        );
    }

    #[test]
    fn honors_comment_escape_and_full_width_space_config() {
        let mut config = LexerConfig::default();
        let output = lex("A;!;+B; ignored", &config);
        assert_eq!(output.tokens.len(), 3);
        config.allow_full_width_space = true;
        assert!(lex("A　+ B", &config).diagnostics.is_empty());
    }

    #[test]
    fn preserves_reference_sign_bit_exponent_literal() {
        for source in ["1p63", "0x1p3f"] {
            let output = lex(source, &LexerConfig::default());
            assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
            assert!(matches!(
                output.tokens.as_slice(),
                [Token {
                    kind: TokenKind::Integer(i64::MIN),
                    ..
                }]
            ));
        }
    }
}
