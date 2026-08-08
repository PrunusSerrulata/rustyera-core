use erabasic_ast::{
    Diagnostic, DiagnosticCode, Directive, Function, Parameter, ParseOutput, Script, SourceKind,
    Span, Statement, StatementKind,
};
use erabasic_lexer::{LexEnd, LexFlags, MacroTable, Operator, Token, TokenKind, lex_with};

use crate::context::ParserContext;
use crate::continuation::{
    ContinuationSourceMap, remap_directive_output, remap_function_output, remap_statement_output,
};
use crate::expression::ExpressionParser;
use crate::line::{parse_directive, parse_line_at};
use crate::preprocessor::{PreprocessorFrame, handle_preprocessor};
use crate::util::{
    expr_to_variable, lines_with_offsets, shift_diagnostics, shift_tokens, split_top_level,
    trim_line_start,
};

pub fn parse_erh(source: &str, context: &mut dyn ParserContext) -> ParseOutput<Script> {
    parse_script(source, SourceKind::Erh, context)
}

/// Parse one ERB source using symbols previously loaded from ERH files.
pub fn parse_erb(source: &str, context: &mut dyn ParserContext) -> ParseOutput<Script> {
    parse_script(source, SourceKind::Erb, context)
}

#[allow(clippy::too_many_lines)]
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
    let mut blocks: Vec<(&'static str, Span)> = Vec::new();
    let mut preprocessor: Vec<PreprocessorFrame> = Vec::new();

    let mut lines = lines_with_offsets(source).peekable();
    while let Some((mut offset, raw_line)) = lines.next() {
        let mut line = raw_line.trim_end_matches('\r');
        // `read_to_string` preserves an UTF-8 BOM whereas StreamReader consumes it.
        // Skip it only at the beginning and keep every reported span byte-accurate.
        if offset == 0
            && let Some(without_bom) = line.strip_prefix('\u{feff}')
        {
            line = without_bom;
            offset += '\u{feff}'.len_utf8();
        }
        let trimmed = trim_line_start(line, context.lexer_config().allow_full_width_space);
        let leading = line.len() - trimmed.len();
        let base = offset + leading;
        if handle_preprocessor(trimmed, base, context, &mut preprocessor, &mut diagnostics) {
            continue;
        }
        if preprocessor.iter().any(|frame| !frame.active) {
            continue;
        }

        let delimiter = trimmed.trim_end_matches([' ', '\t']);
        let mut continued = String::new();
        let mut continuation_source_map = None;
        if delimiter == "{" {
            let opener = Span::new(base, base + 1);
            let mut first_offset = None;
            let mut closed = false;
            let mut source_map = ContinuationSourceMap::default();
            while let Some((part_offset, raw_part)) = lines.next() {
                let part = raw_part.trim_end_matches('\r');
                let part_trimmed =
                    trim_line_start(part, context.lexer_config().allow_full_width_space);
                // EraStreamReader accepts horizontal whitespace after a
                // continuation terminator. Keep the original line intact when
                // joining content, but normalize both ends for delimiter tests.
                let part_delimiter = part_trimmed.trim_end_matches([' ', '\t']);
                if part_delimiter == "}" {
                    closed = true;
                    break;
                }
                if part_delimiter == "{" {
                    diagnostics.push(Diagnostic::error(
                        DiagnosticCode::UnexpectedToken,
                        Span::new(part_offset, part_offset + part.len()),
                        "nested continuation opener",
                    ));
                }
                first_offset.get_or_insert(part_offset);
                let logical_start = continued.len();
                continued.push_str(part);
                source_map.push_source(logical_start, part.len(), part_offset);
                let replacement_start = continued.len();
                continued.push_str(context.continuation_separator());
                let next_offset = lines.peek().map_or(source.len(), |(offset, _)| *offset);
                source_map.push_replacement(
                    replacement_start,
                    context.continuation_separator().len(),
                    part_offset + part.len(),
                    next_offset,
                );
            }
            if !closed {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::UnexpectedToken,
                    opener,
                    "continuation is not closed",
                ));
            }
            offset = first_offset.unwrap_or(offset);
            line = &continued;
            continuation_source_map = Some(source_map);
        } else if delimiter == "}" {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::new(base, base + 1),
                "unexpected continuation terminator",
            ));
            continue;
        }

        let trimmed = trim_line_start(line, context.lexer_config().allow_full_width_space);
        let leading = line.len() - trimmed.len();
        let base = offset + leading;
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }

        if kind == SourceKind::Erb && trimmed.starts_with('@') {
            if let Some(function) = current.take() {
                functions.push(function);
            }
            let parse_base = continuation_source_map.as_ref().map_or(base, |_| 0);
            let mut header = parse_function_header(trimmed, parse_base, context);
            if let Some(source_map) = &continuation_source_map {
                remap_function_output(&mut header, source_map, leading);
            }
            diagnostics.append(&mut header.diagnostics);
            current = header.value;
            blocks.clear();
            continue;
        }

        if trimmed.starts_with('#') {
            let parse_base = continuation_source_map.as_ref().map_or(base, |_| 0);
            let mut directive_output = parse_directive(trimmed, parse_base, context);
            if let Some(source_map) = &continuation_source_map {
                remap_directive_output(&mut directive_output, source_map, leading);
            }
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

        let parse_base = continuation_source_map.as_ref().map_or(base, |_| 0);
        let mut parsed = parse_line_at(trimmed, parse_base, context);
        if let Some(source_map) = &continuation_source_map {
            remap_statement_output(&mut parsed, source_map, leading);
        }
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
    if !matches!(
        tokens.first(),
        Some(Token {
            kind: TokenKind::Identifier(_),
            ..
        })
    ) {
        return ParseOutput {
            value: None,
            diagnostics: vec![Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                Span::new(base, base + source.len()),
                "function name expected after '@'",
            )],
        };
    }
    let mut parameters = Vec::new();
    let tail = tokens.get(1..).unwrap_or_default();
    let parameter_tokens = if matches!(
        tail.first().map(|token| &token.kind),
        Some(TokenKind::Symbol('('))
    ) && matches!(
        tail.last().map(|token| &token.kind),
        Some(TokenKind::Symbol(')'))
    ) {
        &tail[1..tail.len().saturating_sub(1)]
    } else if matches!(
        tail.first().map(|token| &token.kind),
        Some(TokenKind::Symbol(','))
    ) {
        &tail[1..]
    } else {
        &[]
    };
    for segment in split_top_level(parameter_tokens, ',') {
        let assignment = segment
            .iter()
            .position(|token| matches!(token.kind, TokenKind::Operator(Operator::Assign)));
        let left_tokens = assignment.map_or(segment, |index| &segment[..index]);
        let mut left_parser = ExpressionParser::new(left_tokens);
        let target = left_parser.parse().and_then(expr_to_variable);
        let Some(target) = target else {
            continue;
        };
        let default =
            assignment.and_then(|index| ExpressionParser::new(&segment[index + 1..]).parse());
        parameters.push(Parameter {
            name: target.name.clone(),
            span: target.span,
            target: Some(target),
            default,
            is_reference: false,
        });
    }
    let raw_parameters = header_body
        .get(
            tokens
                .first()
                .map_or(0, |token| token.span.end.saturating_sub(base + 1))..,
        )
        .unwrap_or_default()
        .trim_start()
        .to_string();
    let Some(Token {
        kind: TokenKind::Identifier(name),
        ..
    }) = tokens.into_iter().next()
    else {
        unreachable!("function name was validated above");
    };
    ParseOutput {
        value: Some(Function {
            name,
            parameters,
            raw_parameters,
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
            if let Some(name) = declaration_name(&directive.raw_arguments)
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

fn declaration_name(source: &str) -> Option<&str> {
    const KEYWORDS: &[&str] = &[
        "CONST",
        "REF",
        "DYNAMIC",
        "STATIC",
        "GLOBAL",
        "SAVEDATA",
        "CHARADATA",
    ];
    source
        .split(|character: char| character.is_whitespace() || matches!(character, ',' | '='))
        .filter(|part| !part.is_empty())
        .find(|part| {
            !KEYWORDS
                .iter()
                .any(|keyword| part.eq_ignore_ascii_case(keyword))
        })
}

fn check_structure(
    statement: &Statement,
    blocks: &mut Vec<(&'static str, Span)>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let StatementKind::Instruction {
        name, arguments, ..
    } = &statement.kind
    else {
        return;
    };
    let opener = match name.as_str() {
        "IF" => Some("IF"),
        "FOR" => Some("FOR"),
        "REPEAT" => Some("REPEAT"),
        "WHILE" => Some("WHILE"),
        "DO" => Some("DO"),
        "SELECTCASE" => Some("SELECTCASE"),
        "TRYC" | "TRYCCALL" | "TRYCCALLFORM" | "TRYCJUMP" | "TRYCJUMPFORM" | "TRYCGOTO"
        | "TRYCGOTOFORM" => Some("TRYC"),
        "PRINTDATA" | "PRINTDATAL" | "PRINTDATAW" | "PRINTDATAK" | "PRINTDATAKL"
        | "PRINTDATAKW" | "PRINTDATAD" | "PRINTDATADL" | "PRINTDATADW" | "STRDATA" => {
            Some("PRINTDATA")
        }
        "DATALIST" => Some("DATALIST"),
        "TRYCALLLIST" => Some("TRYCALLLIST"),
        "TRYJUMPLIST" => Some("TRYJUMPLIST"),
        "TRYGOTOLIST" => Some("TRYGOTOLIST"),
        _ => None,
    };
    if let Some(opener) = opener {
        if matches!(opener, "TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST")
            && blocks
                .iter()
                .any(|(name, _)| matches!(*name, "TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST"))
        {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnmatchedBlock,
                statement.span,
                "TRY*LIST blocks may not be nested",
            ));
        }
        blocks.push((opener, statement.span));
        return;
    }
    if name == "FUNC" {
        match blocks.last().map(|(name, _)| *name) {
            Some("TRYGOTOLIST") if arguments.len() != 1 => diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnexpectedToken,
                statement.span,
                "TRYGOTOLIST candidates may not have arguments",
            )),
            Some("TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST") => {}
            _ => diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnmatchedBlock,
                statement.span,
                "FUNC is outside a TRY*LIST block",
            )),
        }
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
        _ => None,
    };
    if name == "ENDFUNC" {
        if !matches!(
            blocks.pop().map(|(name, _)| name),
            Some("TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST")
        ) {
            diagnostics.push(Diagnostic::error(
                DiagnosticCode::UnmatchedBlock,
                statement.span,
                "ENDFUNC does not match an open TRY*LIST block",
            ));
        }
        return;
    }
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
