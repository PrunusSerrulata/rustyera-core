use erabasic_ast::{AssignOp, Diagnostic, Expr, ExprKind, Span, VariableRef};
use erabasic_lexer::{Operator, Token, TokenKind};

pub(crate) fn shifted(span: Span, base: usize) -> Span {
    Span::new(span.start + base, span.end + base)
}
pub(crate) fn shift_tokens(tokens: Vec<Token>, base: usize) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|mut t| {
            t.span = shifted(t.span, base);
            t
        })
        .collect()
}
pub(crate) fn shift_diagnostics(diagnostics: Vec<Diagnostic>, base: usize) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|mut d| {
            d.span = shifted(d.span, base);
            d
        })
        .collect()
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

pub(crate) fn lines_with_offsets(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    source.split_inclusive('\n').map(move |line| {
        let start = offset;
        offset += line.len();
        (start, line.strip_suffix('\n').unwrap_or(line))
    })
}
