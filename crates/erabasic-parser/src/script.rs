use erabasic_ast::{
    Argument, Diagnostic, DiagnosticCode, Directive, Expr, ExprKind, Function, Parameter,
    ParseOutput, Script, SourceKind, Span, Statement, StatementKind,
};
use erabasic_lexer::{LexEnd, LexFlags, MacroTable, Operator, Token, TokenKind, lex_with};

use crate::context::ParserContext;
use crate::expression::ExpressionParser;
use crate::line::{parse_directive, parse_line_at};
use crate::preprocessor::{PreprocessorFrame, handle_preprocessor};
use crate::util::{lines_with_offsets, shift_diagnostics, shift_tokens, split_top_level};

pub fn parse_erh(source: &str, context: &mut dyn ParserContext) -> ParseOutput<Script> {
    parse_script(source, SourceKind::Erh, context)
}

/// Parse one ERB source using symbols previously loaded from ERH files.
pub fn parse_erb(source: &str, context: &mut dyn ParserContext) -> ParseOutput<Script> {
    parse_script(source, SourceKind::Erb, context)
}

fn parse_script(
    source: &str,
    kind: SourceKind,
    context: &mut dyn ParserContext,
) -> ParseOutput<Script> {
    let mut diagnostics = Vec::new();
    let mut functions = Vec::new();
    let mut declarations = Vec::new();
    let mut top_level = Vec::new();
    let mut current: Option<Function> = None;
    let mut blocks: Vec<(String, Span)> = Vec::new();
    let mut preprocessor: Vec<PreprocessorFrame> = Vec::new();

    for (offset, raw_line) in lines_with_offsets(source) {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim_start_matches([' ', '\t']);
        let leading = line.len() - trimmed.len();
        let base = offset + leading;
        if handle_preprocessor(trimmed, base, context, &mut preprocessor, &mut diagnostics) {
            continue;
        }
        if preprocessor.iter().any(|frame| !frame.active) {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }

        if kind == SourceKind::Erb && trimmed.starts_with('@') {
            if let Some(function) = current.take() {
                functions.push(function);
            }
            let mut header = parse_function_header(trimmed, base, context);
            diagnostics.append(&mut header.diagnostics);
            current = header.value;
            blocks.clear();
            continue;
        }

        if trimmed.starts_with('#') {
            let mut directive_output = parse_directive(trimmed, base, context);
            diagnostics.append(&mut directive_output.diagnostics);
            if let Some(directive) = directive_output.value {
                if kind == SourceKind::Erh {
                    apply_erh_directive(trimmed, &directive, context, &mut diagnostics);
                }
                if let Some(function) = current.as_mut() {
                    function.attributes.push(directive);
                } else {
                    declarations.push(directive);
                }
            }
            continue;
        }

        let mut parsed = parse_line_at(trimmed, base, context);
        diagnostics.append(&mut parsed.diagnostics);
        if let Some(statement) = parsed.value {
            check_structure(&statement, &mut blocks, &mut diagnostics);
            if let Some(function) = current.as_mut() {
                function.body.push(statement);
            } else {
                top_level.push(statement);
            }
        }
    }
    if let Some(function) = current {
        functions.push(function);
    }
    for (name, span) in blocks {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnmatchedBlock,
            span,
            format!("unclosed {name} block"),
        ));
    }
    if !preprocessor.is_empty() {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::InvalidPreprocessor,
            Span::empty(source.len()),
            "unclosed preprocessor block",
        ));
    }
    ParseOutput {
        value: Some(Script {
            kind,
            functions,
            declarations,
            top_level,
            span: Span::new(0, source.len()),
        }),
        diagnostics,
    }
}

fn parse_function_header(
    source: &str,
    base: usize,
    context: &dyn ParserContext,
) -> ParseOutput<Function> {
    let header_body = &source[1..];
    let lexed = lex_with(
        header_body,
        context.lexer_config(),
        LexEnd::EndOfLine,
        LexFlags::ALLOW_ASSIGNMENT,
        context.macros(),
    );
    let diagnostics = shift_diagnostics(lexed.diagnostics, base + 1);
    let tokens = shift_tokens(lexed.tokens, base + 1);
    let name = match tokens.first() {
        Some(Token {
            kind: TokenKind::Identifier(name),
            ..
        }) => name.clone(),
        _ => {
            return ParseOutput {
                value: None,
                diagnostics: vec![Diagnostic::error(
                    DiagnosticCode::UnexpectedToken,
                    Span::new(base, base + source.len()),
                    "function name expected after '@'",
                )],
            };
        }
    };
    let mut parameters = Vec::new();
    for segment in split_top_level(tokens.get(1..).unwrap_or_default(), ',') {
        let Some(Token {
            kind: TokenKind::Identifier(param_name),
            span,
            ..
        }) = segment.first()
        else {
            continue;
        };
        let default = segment
            .iter()
            .position(|token| matches!(token.kind, TokenKind::Operator(Operator::Assign)))
            .and_then(|index| ExpressionParser::new(&segment[index + 1..]).parse());
        parameters.push(Parameter {
            name: param_name.clone(),
            default,
            is_reference: false,
            span: *span,
        });
    }
    ParseOutput {
        value: Some(Function {
            name,
            parameters,
            attributes: Vec::new(),
            body: Vec::new(),
            span: Span::new(base, base + source.len()),
        }),
        diagnostics,
    }
}

fn apply_erh_directive(
    source: &str,
    directive: &Directive,
    context: &mut dyn ParserContext,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match directive.name.as_str() {
        "DEFINE" => {
            let rest = source.trim_start_matches('#').trim_start();
            let rest = rest
                .strip_prefix("DEFINE")
                .or_else(|| rest.strip_prefix("define"))
                .unwrap_or(rest)
                .trim_start();
            let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = rest[..split].to_uppercase();
            let replacement = rest[split..].trim_start();
            if name.is_empty() {
                return;
            }
            let lexed = lex_with(
                replacement,
                context.lexer_config(),
                LexEnd::EndOfLine,
                LexFlags::ALLOW_ASSIGNMENT,
                &MacroTable::new(),
            );
            diagnostics.extend(lexed.diagnostics);
            context.macros_mut().insert(name, lexed.tokens);
        }
        "DIM" | "DIMS" => {
            if let Some(name) = directive.arguments.first().and_then(argument_identifier)
                && !context.register_variable(name)
            {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::DuplicateDeclaration,
                    directive.span,
                    format!("variable {name} is already declared"),
                ));
            }
        }
        _ => {}
    }
}

fn argument_identifier(argument: &Argument) -> Option<&str> {
    match argument {
        Argument::Expression(Expr {
            kind: ExprKind::Identifier(name) | ExprKind::Variable { name, .. },
            ..
        }) => Some(name),
        _ => None,
    }
}

fn check_structure(
    statement: &Statement,
    blocks: &mut Vec<(String, Span)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let StatementKind::Instruction { name, .. } = &statement.kind else {
        return;
    };
    let opener = match name.as_str() {
        "IF" => Some("IF"),
        "FOR" => Some("FOR"),
        "REPEAT" => Some("REPEAT"),
        "WHILE" => Some("WHILE"),
        "DO" => Some("DO"),
        "SELECTCASE" => Some("SELECTCASE"),
        "TRYC" => Some("TRYC"),
        "PRINTDATA" | "PRINTDATAL" | "PRINTDATAW" => Some("PRINTDATA"),
        "DATALIST" => Some("DATALIST"),
        "TRYLIST" => Some("TRYLIST"),
        "FUNC" => Some("FUNC"),
        _ => None,
    };
    if let Some(opener) = opener {
        blocks.push((opener.to_string(), statement.span));
        return;
    }
    let expected = match name.as_str() {
        "ENDIF" => Some("IF"),
        "NEXT" => Some("FOR"),
        "REND" => Some("REPEAT"),
        "WEND" => Some("WHILE"),
        "LOOP" => Some("DO"),
        "ENDSELECT" => Some("SELECTCASE"),
        "ENDCATCH" => Some("TRYC"),
        "ENDDATA" => Some("PRINTDATA"),
        "ENDLIST" => Some("DATALIST"),
        "ENDFUNC" => Some("FUNC"),
        _ => None,
    };
    if let Some(expected) = expected
        && !matches!(blocks.pop(), Some((name, _)) if name == expected)
    {
        diagnostics.push(Diagnostic::error(
            DiagnosticCode::UnmatchedBlock,
            statement.span,
            format!("{name} does not match an open {expected} block"),
        ));
    }
}
