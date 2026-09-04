//! Runtime CALLSTR text parsing, without symbol lookup or argument evaluation.

use erabasic_ast::{Argument, Diagnostic, DiagnosticCode, Severity, Span};
use erabasic_lexer::{LexEnd, LexFlags, Token, TokenKind, lex_with};

use crate::context::ParserContext;
use crate::expression::ExpressionParser;
use crate::util::{shift_diagnostics, shift_tokens, split_top_level};

/// The reference lexes the entire tail before its TRY-protected argument pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallTextParseStage {
    /// Includes reading/decoding the target field; TRY does not catch this stage.
    Lexical,
    /// Argument syntax only. Name resolution and evaluation happen elsewhere.
    Arguments,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallTextParseError {
    pub stage: CallTextParseStage,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCallText {
    /// The trimmed, escape-decoded target, not a FORM name template.
    pub target: String,
    /// The original target field, before trimming and escape decoding.
    pub target_span: Span,
    pub arguments: Vec<Argument>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallTextParseOutput {
    /// None means the entire source was whitespace, hence no lookup or call.
    pub call: Option<ParsedCallText>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse an already evaluated CALLSTR-family string using the caller's lexer.
///
/// Spans are UTF-8 byte offsets in `source`, shifted by `base`. The caller must
/// keep this runtime-text source separate from the outer instruction's source.
/// This entry point neither resolves names nor evaluates or truncates arguments.
/// In particular, it does not decide which failures a TRY instruction catches.
///
/// # Errors
/// Returns the failing parse stage and diagnostics for malformed targets,
/// lexical input, or argument syntax.
pub fn parse_call_text_at(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> Result<CallTextParseOutput, CallTextParseError> {
    if source.trim().is_empty() {
        return Ok(CallTextParseOutput {
            call: None,
            diagnostics: Vec::new(),
        });
    }

    let (target, target_end) = read_target(source, base)?;
    let delimiter = source[target_end..].chars().next();
    // Include the opening delimiter in the lexer input so bracket diagnostics
    // stay lexical, as in CALLS_Instruction. Even ignored suffix text is lexed.
    let lexed = lex_with(
        &source[target_end..],
        context.lexer_config(),
        LexEnd::EndOfLine,
        LexFlags::NONE,
        context.macros(),
    );
    let mut diagnostics = shift_diagnostics(lexed.diagnostics, base + target_end);
    reject_errors(CallTextParseStage::Lexical, &mut diagnostics)?;
    let tokens = shift_tokens(lexed.tokens, base + target_end);
    // The fixed reference skips the first token whenever the tail is nonempty.
    // The supported spellings place '(' or ',' there; '[' is not subname syntax
    // for CALLSTR, unlike SP_CALL's independent subname grammar.
    let argument_tokens = tokens.get(1..).unwrap_or_default();
    let argument_base = tokens
        .first()
        .map_or(base + target_end, |token| token.span.end);
    let argument_tokens = if delimiter == Some('(') {
        parenthesized_arguments(argument_tokens, base + source.len(), &mut diagnostics)
    } else {
        argument_tokens
    };
    reject_errors(CallTextParseStage::Arguments, &mut diagnostics)?;
    let arguments = reduce_arguments(argument_tokens, argument_base, &mut diagnostics);
    reject_errors(CallTextParseStage::Arguments, &mut diagnostics)?;

    Ok(CallTextParseOutput {
        call: Some(ParsedCallText {
            target,
            target_span: Span::new(base, base + target_end),
            arguments,
            span: Span::new(base, base + source.len()),
        }),
        diagnostics,
    })
}

fn reject_errors(
    stage: CallTextParseStage,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CallTextParseError> {
    if diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        return Err(CallTextParseError {
            stage,
            diagnostics: std::mem::take(diagnostics),
        });
    }
    Ok(())
}

fn read_target(source: &str, base: usize) -> Result<(String, usize), CallTextParseError> {
    let mut target = String::new();
    let mut characters = source.char_indices();
    while let Some((offset, character)) = characters.next() {
        match character {
            '(' | '[' | ',' | ';' | '\0' => return Ok((target.trim().to_owned(), offset)),
            '\\' => {
                let Some((_, escaped)) = characters.next() else {
                    return Err(CallTextParseError {
                        stage: CallTextParseStage::Lexical,
                        diagnostics: vec![Diagnostic::error(
                            DiagnosticCode::InvalidEscape,
                            Span::new(base + offset, base + source.len()),
                            "call target escape is missing its following character",
                        )],
                    });
                };
                // ReadString uses these escapes before Trim, without treating
                // quotes or FORM interpolation as target-name syntax.
                match escaped {
                    '\n' => {}
                    's' => target.push(' '),
                    'S' => target.push('\u{3000}'),
                    't' => target.push('\t'),
                    'n' => target.push('\n'),
                    'e' => target.push_str("\\e"),
                    other => target.push(other),
                }
            }
            other => target.push(other),
        }
    }
    Ok((target.trim().to_owned(), source.len()))
}

fn parenthesized_arguments<'a>(
    tokens: &'a [Token],
    end: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> &'a [Token] {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Symbol('(') => depth += 1,
            TokenKind::Symbol(')') if depth == 0 => {
                // CALLS_Instruction does not check wc.EOL after ReduceArguments.
                // Keep that boundary instead of applying SP_CALL's suffix rule.
                return &tokens[..index];
            }
            TokenKind::Symbol(')') => depth -= 1,
            _ => {}
        }
    }
    // Usually the lexer reports this first; macro expansion can also change the
    // token delimiters after that scan, so the argument pass remains defensive.
    diagnostics.push(Diagnostic::error(
        DiagnosticCode::UnexpectedToken,
        Span::empty(end),
        "call arguments require a closing ')'",
    ));
    tokens
}

fn reduce_arguments(
    tokens: &[Token],
    base: usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Argument> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let segments = split_top_level(tokens, ',');
    let mut arguments = Vec::with_capacity(segments.len());
    let mut token_index = 0_usize;
    for (index, segment) in segments.iter().enumerate() {
        // ReduceArguments checks EOL / ')' before adding another term. A final
        // comma ends the preceding slot, without inventing one more omission.
        if index + 1 == segments.len() && segment.is_empty() {
            break;
        }
        if segment.is_empty() {
            let omitted_at = token_index
                .checked_sub(1)
                .map_or(base, |index| tokens[index].span.end);
            arguments.push(Argument::Omitted(Span::empty(omitted_at)));
        } else {
            let mut parser = ExpressionParser::new(segment);
            if let Some(expression) = parser.parse() {
                arguments.push(Argument::Expression(expression));
            } else if parser.diagnostics.is_empty() {
                parser.diagnostics.push(Diagnostic::error(
                    DiagnosticCode::MissingExpression,
                    segment[0].span,
                    "call argument expression is incomplete",
                ));
            }
            diagnostics.append(&mut parser.diagnostics);
        }
        // Anchor omissions to actual token spans, including whitespace between
        // commas; byte arithmetic here could split a multibyte whitespace char.
        token_index += segment.len() + 1;
    }
    arguments
}
