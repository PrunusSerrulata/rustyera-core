use erabasic_ast::{
    Argument, Diagnostic, DiagnosticCode, Directive, Expr, ExprKind, ParseOutput, Span, Statement,
    StatementKind,
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

#[allow(clippy::too_many_lines)]
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
        // Commit to assignment parsing only when the complete left slice is one variable.
        if left_parser.diagnostics.is_empty()
            && let Some(target) = left.and_then(expr_to_variable)
        {
            let right = parse_assignment_right(
                line,
                line_base,
                op_token,
                &tokens[index + 1..],
                context,
                &mut diagnostics,
            );
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

fn parse_assignment_right(
    line: &str,
    line_base: usize,
    operator: &Token,
    tokens: &[Token],
    context: &dyn ParserContext,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Expr> {
    let right_start = operator.span.end.saturating_sub(line_base);
    let raw_right = &line[right_start..];
    let whitespace = raw_right.len() - raw_right.trim_start().len();
    let right_source = &raw_right[whitespace..];
    let right_base = line_base + right_start + whitespace;
    if right_source.starts_with('%') {
        // String assignments use FORM interpolation syntax even though '%' is
        // the modulo operator in ordinary expressions.
        let (form, lex_diagnostics) =
            lex_formatted(right_source, context.lexer_config(), context.macros());
        let mut output = lower_formatted(&form);
        output.diagnostics.splice(0..0, lex_diagnostics);
        shift_formatted(&mut output, right_base);
        diagnostics.append(&mut output.diagnostics);
        return output.value.map(|value| Expr {
            kind: ExprKind::Formatted(value),
            span: Span::new(right_base, line_base + line.len()),
        });
    }
    let mut parser = ExpressionParser::new(tokens);
    let right = parser.parse();
    diagnostics.append(&mut parser.diagnostics);
    right
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
    if style == ArgumentStyle::DynamicCall {
        return parse_dynamic_call_arguments(source, base, context);
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

fn parse_dynamic_call_arguments(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    let split = dynamic_call_separator(source);
    let (target, arguments) = split.map_or((source.trim(), None), |(index, separator)| {
        let end = if separator == '(' {
            source.rfind(')').unwrap_or(source.len())
        } else {
            source.len()
        };
        (source[..index].trim(), Some(&source[index + 1..end]))
    });
    let target_offset = source.find(target).unwrap_or(0);
    let (form, lex_diagnostics) = lex_formatted(target, context.lexer_config(), context.macros());
    let mut target_output = lower_formatted(&form);
    target_output.diagnostics.splice(0..0, lex_diagnostics);
    shift_formatted(&mut target_output, base + target_offset);
    let mut diagnostics = target_output.diagnostics;
    let mut values = target_output
        .value
        .map_or_else(Vec::new, |value| vec![Argument::Formatted(value)]);
    if let Some(arguments) = arguments {
        let argument_offset = source.find(arguments).unwrap_or(source.len());
        let mut parsed = parse_arguments(
            arguments,
            base + argument_offset,
            ArgumentStyle::Expressions,
            context,
        );
        diagnostics.append(&mut parsed.diagnostics);
        values.extend(parsed.value.unwrap_or_default());
    }
    ParseOutput {
        value: Some(values),
        diagnostics,
    }
}

fn dynamic_call_separator(source: &str) -> Option<(usize, char)> {
    let mut braces = 0_u32;
    let mut brackets = 0_u32;
    let mut percent = false;
    let mut quoted = false;
    for (index, character) in source.char_indices() {
        match character {
            '"' if !percent => quoted = !quoted,
            '%' if !quoted => percent = !percent,
            '{' if !quoted && !percent => braces = braces.saturating_add(1),
            '}' if !quoted && !percent => braces = braces.saturating_sub(1),
            '[' if !quoted && !percent && braces == 0 => brackets = brackets.saturating_add(1),
            ']' if !quoted && !percent && braces == 0 => brackets = brackets.saturating_sub(1),
            '(' | ',' if !quoted && !percent && braces == 0 && brackets == 0 => {
                return Some((index, character));
            }
            _ => {}
        }
    }
    None
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
    // Declaration grammars contain keywords, dimensions and initializers that are
    // not one normal expression. Preserve them verbatim for the semantic pass.
    let style = if matches!(
        name.as_str(),
        "DEFINE" | "DIM" | "DIMS" | "FUNCTION" | "FUNCTIONS"
    ) {
        ArgumentStyle::Raw
    } else {
        ArgumentStyle::Expressions
    };
    let output = parse_arguments(args_text, offset, style, context);
    ParseOutput {
        value: Some(Directive {
            name,
            arguments: output.value.unwrap_or_default(),
            raw_arguments: args_text.to_string(),
            span: Span::new(base, base + source.len()),
        }),
        diagnostics: output.diagnostics,
    }
}
