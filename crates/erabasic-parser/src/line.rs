mod arguments;

use erabasic_ast::{
    Argument, AssignOp, Diagnostic, DiagnosticCode, Directive, Expr, ExprKind, ParseOutput,
    PostfixOp, Span, Statement, StatementKind, UnaryOp,
};
use erabasic_lexer::{LexEnd, LexFlags, Token, TokenKind, lex_with};

use crate::context::{ArgumentStyle, InstructionSpec, ParserContext};
use crate::expression::ExpressionParser;
use crate::formatted::parse_formatted_at;
use crate::util::{
    assign_op, expr_to_variable, shift_diagnostics, shift_tokens, split_top_level,
    top_level_assignment, trim_line_start,
};

use arguments::{OutputMap, parse_arguments, parse_assignment_right, parse_mixed_arguments};

pub fn parse_line(source: &str, context: &dyn ParserContext) -> ParseOutput<Statement> {
    parse_line_at(source, 0, context)
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
    arguments::parse_expression_list_at_impl(source, base, context)
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
    let assignment_index = top_level_assignment(&tokens);
    // VARI/VARS declarations require a name after the keyword. When an assignment
    // operator follows immediately, Emuera instead treats the keyword as a variable
    // name; real projects use `#DIMS VARS` followed by `VARS = ...`.
    let scoped_keyword_assignment = assignment_index == Some(1)
        && tokens.first().is_some_and(|token| {
            matches!(
                &token.kind,
                TokenKind::Identifier(name)
                    if matches!(name.to_ascii_uppercase().as_str(), "VARI" | "VARS")
            )
        });
    let dedicated_instruction_grammar = tokens
        .first()
        .and_then(|token| match &token.kind {
            TokenKind::Identifier(name) => context.instruction(name),
            _ => None,
        })
        .is_some_and(|spec| spec.argument_style != ArgumentStyle::Expressions)
        && !scoped_keyword_assignment;
    if !dedicated_instruction_grammar && let Some(index) = assignment_index {
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

pub(crate) fn parse_directive(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Directive> {
    let allow_full_width_space = context.lexer_config().allow_full_width_space;
    let rest = trim_line_start(
        source.strip_prefix('#').unwrap_or(source),
        allow_full_width_space,
    );
    let name_end = rest
        .find(|character| {
            matches!(character, ' ' | '\t') || (allow_full_width_space && character == '\u{3000}')
        })
        .unwrap_or(rest.len());
    let name = rest[..name_end].to_uppercase();
    let args_text = trim_line_start(&rest[name_end..], allow_full_width_space);
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
