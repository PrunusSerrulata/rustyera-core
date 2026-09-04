use erabasic_ast::{AssignOp, Expr, ExprKind, VariableRef};
use erabasic_lexer::{Operator, Token, TokenKind};

mod spans;

pub(crate) use spans::{
    lines_with_offsets, map_expression_spans, map_formatted_spans, shift_diagnostics, shift_tokens,
    shifted,
};

pub(crate) fn trim_line_start(source: &str, allow_full_width_space: bool) -> &str {
    source.trim_start_matches(|character| {
        matches!(character, ' ' | '\t' | '\r')
            || (allow_full_width_space && character == '\u{3000}')
    })
}

pub(crate) fn split_top_level(tokens: &[Token], separator: char) -> Vec<&[Token]> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Symbol('(' | '[') => depth += 1,
            TokenKind::Symbol(')' | ']') => depth -= 1,
            _ => {}
        }
        if depth == 0 && matches!(token.kind, TokenKind::Symbol(ch) if ch == separator) {
            result.push(&tokens[start..index]);
            start = index + 1;
        }
    }
    result.push(&tokens[start..]);
    result
}

pub(crate) fn top_level_assignment(tokens: &[Token]) -> Option<usize> {
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::Symbol('(' | '[') => depth += 1,
            TokenKind::Symbol(')' | ']') => depth -= 1,
            _ => {}
        }
        if depth == 0
            && matches!(token.kind, TokenKind::Operator(op) if assign_op_opt(op).is_some())
        {
            return Some(index);
        }
    }
    None
}

pub(crate) fn assign_op(operator: Operator) -> AssignOp {
    assign_op_opt(operator).unwrap_or(AssignOp::Assign)
}
pub(crate) fn assign_op_opt(operator: Operator) -> Option<AssignOp> {
    Some(match operator {
        Operator::Assign => AssignOp::Assign,
        Operator::StringAssign => AssignOp::StringAssign,
        Operator::AddAssign => AssignOp::Add,
        Operator::SubtractAssign => AssignOp::Subtract,
        Operator::MultiplyAssign => AssignOp::Multiply,
        Operator::DivideAssign => AssignOp::Divide,
        Operator::ModuloAssign => AssignOp::Modulo,
        Operator::BitAndAssign => AssignOp::BitAnd,
        Operator::BitOrAssign => AssignOp::BitOr,
        Operator::BitXorAssign => AssignOp::BitXor,
        Operator::ShiftLeftAssign => AssignOp::ShiftLeft,
        Operator::ShiftRightAssign => AssignOp::ShiftRight,
        _ => return None,
    })
}

pub(crate) fn expr_to_variable(expr: Expr) -> Option<VariableRef> {
    let span = expr.span;
    match expr.kind {
        ExprKind::Identifier(name) => Some(VariableRef {
            name,
            indices: Vec::new(),
            span,
        }),
        ExprKind::Variable { name, indices } => Some(VariableRef {
            name,
            indices,
            span,
        }),
        _ => None,
    }
}
