use erabasic_ast::{Diagnostic, DiagnosticCode, Span};

use crate::context::ParserContext;

#[derive(Clone, Copy)]
pub(crate) struct PreprocessorFrame {
    pub(crate) parent_active: bool,
    pub(crate) active: bool,
    pub(crate) branch_taken: bool,
}

pub(crate) fn handle_preprocessor(
    line: &str,
    base: usize,
    context: &dyn ParserContext,
    stack: &mut Vec<PreprocessorFrame>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if !(line.starts_with('[') && line.ends_with(']')) {
        return false;
    }
    let directive_body = &line[1..line.len() - 1];
    let mut fields = directive_body.split_whitespace();
    let command = fields.next().unwrap_or_default().to_uppercase();
    let parent_active = stack.iter().all(|frame| frame.active);
    match command.as_str() {
        "IF" | "IF_DEBUG" | "IF_NDEBUG" => {
            let condition = match command.as_str() {
                "IF_DEBUG" => context.lexer_config().debug_semicolon,
                "IF_NDEBUG" => !context.lexer_config().debug_semicolon,
                _ => eval_preprocessor(fields.collect::<Vec<_>>().join(" ").as_str(), context),
            };
            stack.push(PreprocessorFrame {
                parent_active,
                active: parent_active && condition,
                branch_taken: condition,
            });
        }
        "ELSEIF" => {
            let Some(frame) = stack.last_mut() else {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::InvalidPreprocessor,
                    Span::new(base, base + line.len()),
                    "ELSEIF without IF",
                ));
                return true;
            };
            let condition =
                eval_preprocessor(fields.collect::<Vec<_>>().join(" ").as_str(), context);
            frame.active = frame.parent_active && !frame.branch_taken && condition;
            frame.branch_taken |= condition;
        }
        "ELSE" => {
            let Some(frame) = stack.last_mut() else {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::InvalidPreprocessor,
                    Span::new(base, base + line.len()),
                    "ELSE without IF",
                ));
                return true;
            };
            frame.active = frame.parent_active && !frame.branch_taken;
            frame.branch_taken = true;
        }
        "ENDIF" => {
            if stack.pop().is_none() {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::InvalidPreprocessor,
                    Span::new(base, base + line.len()),
                    "ENDIF without IF",
                ));
            }
        }
        "SKIPSTART" => stack.push(PreprocessorFrame {
            parent_active,
            active: false,
            branch_taken: true,
        }),
        "SKIPEND" => {
            if stack.pop().is_none() {
                diagnostics.push(Diagnostic::error(
                    DiagnosticCode::InvalidPreprocessor,
                    Span::new(base, base + line.len()),
                    "SKIPEND without SKIPSTART",
                ));
            }
        }
        _ => return false,
    }
    true
}

fn eval_preprocessor(expression: &str, context: &dyn ParserContext) -> bool {
    let expression = expression.trim();
    if let Some(rest) = expression.strip_prefix('!') {
        return !eval_preprocessor(rest, context);
    }
    expression.parse::<i64>().map_or_else(
        |_| context.preprocessor_symbol(expression).unwrap_or(0) != 0,
        |value| value != 0,
    )
}
