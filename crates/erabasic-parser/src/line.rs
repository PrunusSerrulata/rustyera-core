use erabasic_ast::{
    Argument, AssignOp, Diagnostic, DiagnosticCode, Directive, Expr, ExprKind, ParseOutput,
    PostfixOp, Span, Statement, StatementKind, UnaryOp,
};
use erabasic_lexer::{
    LexEnd, LexFlags, Token, TokenKind, lex_formatted, lex_formatted_until_comma, lex_with,
};

use crate::context::{ArgumentStyle, InstructionSpec, ParserContext};
use crate::expression::ExpressionParser;
use crate::formatted::{lower_formatted, parse_formatted_at, shift_formatted};
use crate::util::{
    assign_op, expr_to_variable, shift_diagnostics, shift_tokens, split_top_level,
    top_level_assignment, trim_line_start,
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
    let line = trim_line_start(source, context.lexer_config().allow_full_width_space);
    let leading = source.len() - line.len();
    let line_base = base + leading;
    if line.is_empty() || line.starts_with(';') {
        return ParseOutput {
            value: None,
            diagnostics: Vec::new(),
        };
    }
    if let Some(label_body) = line.strip_prefix('$') {
        let name = trim_line_start(label_body, context.lexer_config().allow_full_width_space)
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
    let dedicated_instruction_grammar = tokens
        .first()
        .and_then(|token| match &token.kind {
            TokenKind::Identifier(name) => context.instruction(name),
            _ => None,
        })
        .is_some_and(|spec| spec.argument_style != ArgumentStyle::Expressions);
    if !dedicated_instruction_grammar && let Some(index) = top_level_assignment(&tokens) {
        let mut left_parser = ExpressionParser::new(&tokens[..index]);
        let left = left_parser.parse();
        let op_token = &tokens[index];
        // Commit to assignment parsing only when the complete left slice is one variable.
        if left_parser.diagnostics.is_empty()
            && let Some(target) = left.and_then(expr_to_variable)
        {
            if matches!(
                op_token.kind,
                TokenKind::Operator(erabasic_lexer::Operator::Assign)
            ) {
                let right_start = op_token.span.end.saturating_sub(line_base);
                let raw_right = &line[right_start..];
                let whitespace = raw_right.len() - raw_right.trim_start().len();
                let right_source = &raw_right[whitespace..];
                let right_base = line_base + right_start + whitespace;
                let value = if right_source.is_empty() {
                    Expr {
                        kind: ExprKind::String(String::new()),
                        span: Span::empty(right_base),
                    }
                } else {
                    let mut parsed = parse_formatted_at(right_source, right_base, context);
                    diagnostics.append(&mut parsed.diagnostics);
                    Expr {
                        kind: parsed.value.map_or(ExprKind::Error, ExprKind::Formatted),
                        span: Span::new(right_base, line_base + line.len()),
                    }
                };
                // The whole-line expression lexer cannot interpret the type-directed
                // FORM RHS. Its diagnostics are superseded by the dedicated pass.
                diagnostics.retain(|diagnostic| diagnostic.span.end <= op_token.span.start);
                return ParseOutput {
                    value: Some(Statement {
                        span: Span::new(line_base, line_base + line.len()),
                        kind: StatementKind::Assignment {
                            target,
                            op: AssignOp::Assign,
                            value,
                            additional_values: Vec::new(),
                            raw_value: right_source.into(),
                        },
                    }),
                    diagnostics,
                };
            }
            if matches!(
                op_token.kind,
                TokenKind::Operator(erabasic_lexer::Operator::StringAssign)
            ) && split_top_level(&tokens[index + 1..], ',').len() > 1
            {
                let right_start = op_token.span.end.saturating_sub(line_base);
                let raw_right = &line[right_start..];
                let whitespace = raw_right.len() - raw_right.trim_start().len();
                let right_source = &raw_right[whitespace..];
                let right_base = line_base + right_start + whitespace;
                let mut values = parse_arguments(
                    right_source,
                    right_base,
                    ArgumentStyle::Expressions,
                    context,
                );
                diagnostics.append(&mut values.diagnostics);
                let mut values = values
                    .value
                    .unwrap_or_default()
                    .into_iter()
                    .map(|argument| match argument {
                        Argument::Expression(value) => value,
                        Argument::Omitted(span) => Expr {
                            kind: ExprKind::Error,
                            span,
                        },
                        _ => unreachable!("expression argument grammar returned another shape"),
                    });
                let value = values.next().unwrap_or(Expr {
                    kind: ExprKind::Error,
                    span: Span::empty(right_base),
                });
                return ParseOutput {
                    value: Some(Statement {
                        span: Span::new(line_base, line_base + line.len()),
                        kind: StatementKind::Assignment {
                            target,
                            op: assign_op(match op_token.kind {
                                TokenKind::Operator(op) => op,
                                _ => unreachable!("assignment token is an operator"),
                            }),
                            value,
                            additional_values: values.collect(),
                            raw_value: right_source.into(),
                        },
                    }),
                    diagnostics,
                };
            }
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
                            additional_values: Vec::new(),
                            raw_value: assignment_right_source(line, line_base, op_token).into(),
                        },
                    }),
                    diagnostics,
                };
            }
        }
    }

    if !dedicated_instruction_grammar {
        let mut parser = ExpressionParser::new(&tokens);
        let expression = parser.parse();
        if parser.diagnostics.is_empty()
            && let Some((target, op)) = expression.and_then(increment_assignment)
        {
            return ParseOutput {
                value: Some(Statement {
                    span: Span::new(line_base, line_base + line.len()),
                    kind: StatementKind::Assignment {
                        target,
                        op,
                        value: Expr {
                            kind: ExprKind::Integer(1),
                            span: Span::new(line_base, line_base + line.len()),
                        },
                        additional_values: Vec::new(),
                        raw_value: "1".into(),
                    },
                }),
                diagnostics,
            };
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
    let uses_mixed_arguments = matches!(
        name.to_ascii_uppercase().as_str(),
        "PRINT_IMG" | "PRINT_RECT" | "PRINT_SPACE"
    );
    if uses_mixed_arguments
        || matches!(
            spec.argument_style,
            ArgumentStyle::Formatted
                | ArgumentStyle::Raw
                | ArgumentStyle::PrintV
                | ArgumentStyle::Times
                | ArgumentStyle::DynamicCall
                | ArgumentStyle::FormattedFirst
        )
    {
        // The preliminary whole-line lex identifies the instruction name, but
        // non-expression grammars may contain arbitrary punctuation (and Era's
        // `px` mixed unit). Their dedicated pass owns all tail diagnostics.
        diagnostics.retain(|diagnostic| diagnostic.span.end <= name_span.end);
    }
    let mut args_output = if uses_mixed_arguments {
        parse_mixed_arguments(name, &raw, raw_offset, context)
    } else {
        parse_arguments(&raw, raw_offset, spec.argument_style, context)
    };
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

fn assignment_right_source<'a>(line: &'a str, line_base: usize, operator: &Token) -> &'a str {
    let right_start = operator.span.end.saturating_sub(line_base);
    line[right_start..].trim_start_matches([' ', '\t'])
}

fn increment_assignment(expression: Expr) -> Option<(erabasic_ast::VariableRef, AssignOp)> {
    let (operand, increment) = match expression.kind {
        ExprKind::Postfix { op, operand } => (operand, matches!(op, PostfixOp::Increment)),
        ExprKind::Unary { op, operand }
            if matches!(op, UnaryOp::PreIncrement | UnaryOp::PreDecrement) =>
        {
            (operand, matches!(op, UnaryOp::PreIncrement))
        }
        _ => return None,
    };
    expr_to_variable(*operand).map(|target| {
        (
            target,
            if increment {
                AssignOp::Add
            } else {
                AssignOp::Subtract
            },
        )
    })
}

fn parse_mixed_arguments(
    name: &str,
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    let mut arguments = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, (start, end)) in split_argument_text(source).into_iter().enumerate() {
        let raw = source[start..end].trim();
        let whitespace = source[start..end].len() - source[start..end].trim_start().len();
        let argument_base = base + start + whitespace;
        if raw.is_empty() {
            arguments.push(Argument::Omitted(Span::empty(argument_base)));
            continue;
        }
        let bytes = raw.as_bytes();
        let has_px_suffix = bytes.len() >= 2
            && bytes[bytes.len() - 2].eq_ignore_ascii_case(&b'p')
            && bytes[bytes.len() - 1].eq_ignore_ascii_case(&b'x');
        let (expression_source, is_px) = if has_px_suffix {
            (raw[..raw.len() - 2].trim_end(), true)
        } else {
            (raw, false)
        };
        let mut parsed = parse_arguments(
            expression_source,
            argument_base,
            ArgumentStyle::Expressions,
            context,
        );
        diagnostics.append(&mut parsed.diagnostics);
        let Some(argument) = parsed.value.and_then(|mut values| values.pop()) else {
            continue;
        };
        match argument {
            Argument::Expression(expression)
                if !name.eq_ignore_ascii_case("PRINT_IMG") || index != 0 =>
            {
                arguments.push(Argument::MixedExpression { expression, is_px });
            }
            other => arguments.push(other),
        }
    }
    ParseOutput {
        value: Some(arguments),
        diagnostics,
    }
}

fn split_argument_text(source: &str) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quote = None;
    for (index, character) in source.char_indices() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push((start, index));
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push((start, source.len()));
    result
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
    if right_source.is_empty() {
        // The reference's generic SET grammar uses an empty string when a bare
        // string assignment ends immediately after `=` (for example `RESULTS =`).
        // Semantic analysis still rejects the same spelling for integer places.
        return Some(Expr {
            kind: ExprKind::String(String::new()),
            span: Span::empty(right_base),
        });
    }
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
    let tokens = if matches!(
        operator.kind,
        TokenKind::Operator(
            erabasic_lexer::Operator::AddAssign
                | erabasic_lexer::Operator::SubtractAssign
                | erabasic_lexer::Operator::MultiplyAssign
                | erabasic_lexer::Operator::DivideAssign
                | erabasic_lexer::Operator::ModuloAssign
                | erabasic_lexer::Operator::BitAndAssign
                | erabasic_lexer::Operator::BitOrAssign
                | erabasic_lexer::Operator::BitXorAssign
                | erabasic_lexer::Operator::ShiftLeftAssign
                | erabasic_lexer::Operator::ShiftRightAssign
        )
    ) && matches!(
        tokens.last(),
        Some(Token {
            kind: TokenKind::Symbol(','),
            ..
        })
    ) {
        &tokens[..tokens.len() - 1]
    } else {
        tokens
    };
    let mut parser = ExpressionParser::new(tokens);
    let right = parser.parse();
    if !parser.diagnostics.is_empty() {
        // String SET accepts unquoted FORM text (`LOCALS = HP(%NAME%)`). Try the
        // ordinary expression grammar first so integer modulo remains unambiguous,
        // then recover through FORM only when that grammar cannot consume the RHS.
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
        return ParseOutput::success(vec![Argument::Raw(raw_argument(source, context))]);
    }
    if style == ArgumentStyle::Times {
        return parse_times_arguments(source, base, context);
    }
    if style == ArgumentStyle::DynamicCall {
        return parse_dynamic_call_arguments(source, base, context);
    }
    if style == ArgumentStyle::FormattedFirst {
        return parse_formatted_first_arguments(source, base, context);
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

    let flags = if style == ArgumentStyle::PrintV {
        LexFlags::ANALYZE_PRINT_V
    } else {
        LexFlags::NONE
    };
    let lexed = lex_with(
        source,
        context.lexer_config(),
        LexEnd::EndOfLine,
        flags,
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

fn parse_formatted_first_arguments(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    let (form, consumed, lex_diagnostics) =
        lex_formatted_until_comma(source, context.lexer_config(), context.macros());
    let mut formatted = lower_formatted(&form);
    formatted.diagnostics.splice(0..0, lex_diagnostics);
    shift_formatted(&mut formatted, base);
    let mut diagnostics = formatted.diagnostics;
    let mut arguments = formatted
        .value
        .map_or_else(Vec::new, |value| vec![Argument::Formatted(value)]);
    if source.as_bytes().get(consumed) == Some(&b',') {
        let tail_start = consumed + 1;
        let mut tail = parse_arguments(
            &source[tail_start..],
            base + tail_start,
            ArgumentStyle::Expressions,
            context,
        );
        diagnostics.append(&mut tail.diagnostics);
        arguments.extend(tail.value.unwrap_or_default());
    }
    ParseOutput {
        value: Some(arguments),
        diagnostics,
    }
}

/// Parses generic comma-separated expressions at an existing source offset.
///
/// Plain `=` delays this pass until semantic analysis knows whether its
/// destination is numeric (a SET list) or string (one FORM value with commas).
pub fn parse_expression_list_at(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Expr>> {
    parse_arguments(source, base, ArgumentStyle::Expressions, context).map(|arguments| {
        arguments
            .into_iter()
            .map(|argument| match argument {
                Argument::Expression(expression) => expression,
                Argument::Omitted(span) => Expr {
                    kind: ExprKind::Error,
                    span,
                },
                _ => unreachable!("expression grammar produced a non-expression argument"),
            })
            .collect()
    })
}

fn parse_times_arguments(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    let segments = split_argument_text(source);
    if segments.len() != 2 {
        return ParseOutput {
            value: Some(Vec::new()),
            diagnostics: vec![Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::new(base, base + source.len()),
                "TIMES requires a variable and one real literal",
            )],
        };
    }
    let (place_start, place_end) = segments[0];
    let place_source = source[place_start..place_end].trim();
    let place_whitespace =
        source[place_start..place_end].len() - source[place_start..place_end].trim_start().len();
    let mut place = parse_arguments(
        place_source,
        base + place_start + place_whitespace,
        ArgumentStyle::Expressions,
        context,
    );
    let mut diagnostics = std::mem::take(&mut place.diagnostics);
    let Some(mut arguments) = place.value else {
        return ParseOutput {
            value: None,
            diagnostics,
        };
    };
    if arguments.len() != 1 {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnexpectedToken,
            Span::new(base + place_start, base + place_end),
            "TIMES first argument must be one expression",
        ));
        return ParseOutput {
            value: Some(arguments),
            diagnostics,
        };
    }
    let (real_start, real_end) = segments[1];
    let real_source = source[real_start..real_end].trim();
    let real_whitespace =
        source[real_start..real_end].len() - source[real_start..real_end].trim_start().len();
    let real_base = base + real_start + real_whitespace;
    match decimal_ratio(real_source) {
        Some((numerator, denominator)) => {
            let span = Span::new(real_base, real_base + real_source.len());
            arguments.push(Argument::Expression(Expr {
                kind: ExprKind::Integer(numerator),
                span,
            }));
            arguments.push(Argument::Expression(Expr {
                kind: ExprKind::Integer(denominator),
                span,
            }));
        }
        None => diagnostics.push(Diagnostic::error(
            DiagnosticCode::InvalidInteger,
            Span::new(real_base, real_base + real_source.len()),
            "TIMES second argument must be a finite real literal",
        )),
    }
    ParseOutput {
        value: Some(arguments),
        diagnostics,
    }
}

fn decimal_ratio(source: &str) -> Option<(i64, i64)> {
    let (negative, unsigned) = match source.as_bytes().first() {
        Some(b'+') => (false, &source[1..]),
        Some(b'-') => (true, &source[1..]),
        _ => (false, source),
    };
    let (mantissa, exponent) = if let Some(index) = unsigned.find(['e', 'E']) {
        (
            &unsigned[..index],
            unsigned[index + 1..].parse::<i32>().ok()?,
        )
    } else {
        (unsigned, 0_i32)
    };
    let mut digits = String::with_capacity(mantissa.len());
    let mut fractional = 0_i32;
    let mut seen_dot = false;
    for character in mantissa.chars() {
        match character {
            '0'..='9' => {
                digits.push(character);
                fractional += i32::from(seen_dot);
            }
            '.' if !seen_dot => seen_dot = true,
            _ => return None,
        }
    }
    if digits.is_empty() {
        return None;
    }
    let mut numerator = digits.parse::<i128>().ok()?;
    if negative {
        numerator = -numerator;
    }
    let scale = fractional.checked_sub(exponent)?;
    let mut denominator = 1_i128;
    if scale >= 0 {
        denominator = 10_i128.checked_pow(u32::try_from(scale).ok()?)?;
    } else {
        numerator = numerator.checked_mul(10_i128.checked_pow(scale.unsigned_abs())?)?;
    }
    let divisor = i128::try_from(gcd_i128(
        numerator.unsigned_abs(),
        denominator.cast_unsigned(),
    ))
    .ok()?;
    i64::try_from(numerator / divisor)
        .ok()
        .zip(i64::try_from(denominator / divisor).ok())
}

fn gcd_i128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.max(1)
}

fn raw_argument(source: &str, context: &dyn ParserContext) -> String {
    let mut result = String::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        let remaining = &source[index..];
        if remaining.starts_with(";!;")
            || (context.lexer_config().debug_semicolon && remaining.starts_with(";#;"))
        {
            index += 3;
            continue;
        }
        let character = remaining.chars().next().expect("index is inside source");
        if character == ';' {
            break;
        }
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn parse_dynamic_call_arguments(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Vec<Argument>> {
    // The closing parenthesis must be found before an Era comment. A commented
    // expression may contain arbitrary unmatched parentheses of its own.
    let owned_source = call_source_before_comment(source);
    let source = owned_source.as_str();
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

fn call_source_before_comment(source: &str) -> String {
    let mut quoted = false;
    for (index, character) in source.char_indices() {
        match character {
            '"' => quoted = !quoted,
            ';' if !quoted => return source[..index].trim_end().to_owned(),
            _ => {}
        }
    }
    source.to_owned()
}

fn dynamic_call_separator(source: &str) -> Option<(usize, char)> {
    let mut braces = 0_u32;
    let mut brackets = 0_u32;
    let mut percent = false;
    let mut quoted = false;
    for (index, character) in source.char_indices() {
        match character {
            '"' if !percent => quoted = !quoted,
            // Percent signs inside `{...}` are expression modulo operators, not
            // `%...%` FORM interpolation delimiters. Treating them as delimiters
            // hides the closing brace and makes the target swallow `(arguments)`.
            '%' if !quoted && braces == 0 => percent = !percent,
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
