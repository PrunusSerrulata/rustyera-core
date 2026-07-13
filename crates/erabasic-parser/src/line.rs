use erabasic_ast::{
    Argument, Diagnostic, DiagnosticCode, Directive, ParseOutput, Span, Statement, StatementKind,
};
use erabasic_lexer::{LexEnd, LexFlags, Token, TokenKind, lex_formatted, lex_with};

use crate::context::{ArgumentStyle, InstructionSpec, ParserContext};
use crate::expression::ExpressionParser;
use crate::formatted::{lower_formatted, shift_formatted};
use crate::util::{
    assign_op, expr_to_variable, shift_diagnostics, shift_tokens, split_top_level,
    top_level_assignment,
};

pub fn parse_line(source: &str, context: &dyn ParserContext) -> ParseOutput<Statement> {
    parse_line_at(source, 0, context)
}

pub(crate) fn parse_line_at(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Statement> {
    let leading = source.len() - source.trim_start_matches([' ', '\t']).len();
    let line = &source[leading..];
    let line_base = base + leading;
    if line.is_empty() || line.starts_with(';') {
        return ParseOutput {
            value: None,
            diagnostics: Vec::new(),
        };
    }
    if let Some(label_body) = line.strip_prefix('$') {
        let name = label_body
            .split([' ', '\t', ';'])
            .next()
            .unwrap_or_default()
            .to_string();
        return ParseOutput::success(Statement {
            kind: StatementKind::GotoLabel { name },
            span: Span::new(line_base, line_base + line.len()),
        });
    }
    if line.starts_with('#') {
        let directive = parse_directive(line, line_base, context);
        return directive.map(|d| Statement {
            span: d.span,
            kind: StatementKind::Directive(d),
        });
    }

    let lexed = lex_with(
        line,
        context.lexer_config(),
        LexEnd::EndOfLine,
        LexFlags::ALLOW_ASSIGNMENT,
        context.macros(),
    );
    let mut diagnostics = shift_diagnostics(lexed.diagnostics, line_base);
    let tokens = shift_tokens(lexed.tokens, line_base);
    if let Some(index) = top_level_assignment(&tokens) {
        let mut left_parser = ExpressionParser::new(&tokens[..index]);
        let left = left_parser.parse();
        let op_token = &tokens[index];
        // A formatted PRINT argument may contain a top-level '='. Emuera first
        // recognizes known instruction names, so only commit to assignment
        // parsing when the complete left token slice is one variable.
        if left_parser.diagnostics.is_empty()
            && let Some(target) = left.and_then(expr_to_variable)
        {
            let mut right_parser = ExpressionParser::new(&tokens[index + 1..]);
            let right = right_parser.parse();
            diagnostics.append(&mut right_parser.diagnostics);
            if let (Some(value), TokenKind::Operator(op)) = (right, &op_token.kind) {
                return ParseOutput {
                    value: Some(Statement {
                        span: Span::new(line_base, line_base + line.len()),
                        kind: StatementKind::Assignment {
                            target,
                            op: assign_op(*op),
                            value,
                        },
                    }),
                    diagnostics,
                };
            }
        }
    }

    let Some(Token {
        kind: TokenKind::Identifier(name),
        span: name_span,
        ..
    }) = tokens.first()
    else {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnexpectedToken,
            Span::new(line_base, line_base + line.len()),
            "instruction name expected",
        ));
        return ParseOutput {
            value: None,
            diagnostics,
        };
    };
    let raw_start = name_span.end - line_base;
    let raw = line[raw_start..].trim_start().to_string();
    let raw_offset =
        line_base + line[raw_start..].len() - line[raw_start..].trim_start().len() + raw_start;
    let spec = context.instruction(name).unwrap_or(InstructionSpec {
        argument_style: ArgumentStyle::Raw,
    });
    let mut args_output = parse_arguments(&raw, raw_offset, spec.argument_style, context);
    diagnostics.append(&mut args_output.diagnostics);
    ParseOutput {
        value: Some(Statement {
            kind: StatementKind::Instruction {
                name: name.to_uppercase(),
                arguments: args_output.value.unwrap_or_default(),
                raw_arguments: raw,
            },
            span: Span::new(line_base, line_base + line.len()),
        }),
        diagnostics,
    }
}

trait OutputMap<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> ParseOutput<U>;
}

impl<T> OutputMap<T> for ParseOutput<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> ParseOutput<U> {
        ParseOutput {
            value: self.value.map(f),
            diagnostics: self.diagnostics,
        }
    }
}

fn parse_arguments(
    source: &str,
    base: usize,
    style: ArgumentStyle,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    if source.is_empty() || style == ArgumentStyle::None {
        return ParseOutput::success(Vec::new());
    }
    if style == ArgumentStyle::Raw {
        return ParseOutput::success(vec![Argument::Raw(source.to_string())]);
    }
    if style == ArgumentStyle::Formatted {
        if source.starts_with("@\"") {
            let parsed = lex_with(
                source,
                context.lexer_config(),
                LexEnd::EndOfLine,
                LexFlags::NONE,
                context.macros(),
            );
            let mut diagnostics = shift_diagnostics(parsed.diagnostics, base);
            if let Some(Token {
                kind: TokenKind::Formatted(form),
                ..
            }) = parsed.tokens.first()
            {
                let mut form_output = lower_formatted(form);
                shift_formatted(&mut form_output, base);
                diagnostics.append(&mut form_output.diagnostics);
                return ParseOutput {
                    value: form_output.value.map(|f| vec![Argument::Formatted(f)]),
                    diagnostics,
                };
            }
        }
        let (form, lex_diagnostics) =
            lex_formatted(source, context.lexer_config(), context.macros());
        let mut output = lower_formatted(&form);
        output.diagnostics.splice(0..0, lex_diagnostics);
        shift_formatted(&mut output, base);
        return ParseOutput {
            value: output.value.map(|value| vec![Argument::Formatted(value)]),
            diagnostics: output.diagnostics,
        };
    }

    let lexed = lex_with(
        source,
        context.lexer_config(),
        LexEnd::EndOfLine,
        LexFlags::NONE,
        context.macros(),
    );
    let mut diagnostics = shift_diagnostics(lexed.diagnostics, base);
    let tokens = shift_tokens(lexed.tokens, base);
    let mut arguments = Vec::new();
    for segment in split_top_level(&tokens, ',') {
        if segment.is_empty() {
            arguments.push(Argument::Omitted(Span::empty(base)));
            continue;
        }
        let mut parser = ExpressionParser::new(segment);
        if let Some(expr) = parser.parse() {
            arguments.push(Argument::Expression(expr));
        }
        diagnostics.append(&mut parser.diagnostics);
    }
    ParseOutput {
        value: Some(arguments),
        diagnostics,
    }
}

pub(crate) fn parse_directive(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Directive> {
    let rest = source.strip_prefix('#').unwrap_or(source).trim_start();
    let name_end = rest.find([' ', '\t']).unwrap_or(rest.len());
    let name = rest[..name_end].to_uppercase();
    let args_text = rest[name_end..].trim_start();
    let offset = base + source.find(args_text).unwrap_or(source.len());
    let style = if matches!(name.as_str(), "DEFINE") {
        ArgumentStyle::Raw
    } else {
        ArgumentStyle::Expressions
    };
    let output = parse_arguments(args_text, offset, style, context);
    ParseOutput {
        value: Some(Directive {
            name,
            arguments: output.value.unwrap_or_default(),
            span: Span::new(base, base + source.len()),
        }),
        diagnostics: output.diagnostics,
    }
}
