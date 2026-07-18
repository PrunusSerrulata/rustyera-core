use erabasic_ast::{
    BinaryOp, Diagnostic, DiagnosticCode, Expr, ExprKind, ParseOutput, PostfixOp, Span, UnaryOp,
};
use erabasic_lexer::{LexEnd, LexFlags, Operator, Token, TokenKind, lex_with};

use crate::context::ParserContext;
use crate::formatted::lower_formatted;

#[must_use]
pub fn parse_expression(source: &str, context: &dyn ParserContext) -> ParseOutput<Expr> {
    let lexed = lex_with(
        source,
        context.lexer_config(),
        LexEnd::EndOfLine,
        LexFlags::NONE,
        context.macros(),
    );
    let mut parser = ExpressionParser::new(&lexed.tokens);
    let value = parser.parse();
    let mut diagnostics = lexed.diagnostics;
    diagnostics.append(&mut parser.diagnostics);
    ParseOutput { value, diagnostics }
}

pub(crate) struct ExpressionParser<'a> {
    tokens: &'a [Token],
    pos: usize,
    suppress_indices: bool,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

impl<'a> ExpressionParser<'a> {
    pub(crate) fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            suppress_indices: false,
            diagnostics: Vec::new(),
        }
    }

    pub(crate) fn parse(&mut self) -> Option<Expr> {
        if self.tokens.is_empty() {
            return None;
        }
        let expression = self.parse_bp(0);
        if self.pos < self.tokens.len() {
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::TrailingInput,
                self.tokens[self.pos].span,
                "unexpected token after expression",
            ));
        }
        expression
    }

    fn parse_bp(&mut self, min_bp: u8) -> Option<Expr> {
        let mut left = self.parse_prefix()?;
        loop {
            if !self.suppress_indices
                && let Some(op) = self.postfix()
            {
                let span = left.span.join(self.tokens[self.pos].span);
                self.pos += 1;
                left = Expr {
                    kind: ExprKind::Postfix {
                        op,
                        operand: Box::new(left),
                    },
                    span,
                };
                continue;
            }
            if matches!(
                self.current_kind(),
                Some(TokenKind::Operator(Operator::Question))
            ) {
                if 1 < min_bp {
                    break;
                }
                self.pos += 1;
                let then_expr = self.parse_bp(0)?;
                if !matches!(
                    self.current_kind(),
                    Some(TokenKind::Operator(Operator::TernarySeparator))
                ) {
                    self.diagnostics.push(Diagnostic::error(
                        DiagnosticCode::UnexpectedToken,
                        self.current_span(),
                        "ternary expression requires '#' or ':'",
                    ));
                    return Some(left);
                }
                self.pos += 1;
                let else_expr = self.parse_bp(1)?;
                let span = left.span.join(else_expr.span);
                left = Expr {
                    kind: ExprKind::Ternary {
                        condition: Box::new(left),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    },
                    span,
                };
                continue;
            }
            let Some((op, left_bp, right_bp)) = self.binary() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }
            self.pos += 1;
            let Some(right) = self.parse_bp(right_bp) else {
                self.diagnostics.push(Diagnostic::error(
                    DiagnosticCode::MissingExpression,
                    self.current_span(),
                    "right operand expected",
                ));
                break;
            };
            let span = left.span.join(right.span);
            left = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            };
        }
        Some(left)
    }

    fn parse_prefix(&mut self) -> Option<Expr> {
        let token = self.tokens.get(self.pos)?.clone();
        if let Some(op) = prefix_operator(&token.kind) {
            self.pos += 1;
            let operand = self.parse_bp(25)?;
            return Some(Expr {
                span: token.span.join(operand.span),
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
            });
        }
        self.pos += 1;
        match token.kind {
            TokenKind::Integer(value) => Some(Expr {
                kind: ExprKind::Integer(value),
                span: token.span,
            }),
            TokenKind::String(value) => Some(Expr {
                kind: ExprKind::String(value),
                span: token.span,
            }),
            TokenKind::Formatted(form) => {
                let output = lower_formatted(&form);
                self.diagnostics.extend(output.diagnostics);
                output.value.map(|value| Expr {
                    kind: ExprKind::Formatted(value),
                    span: token.span,
                })
            }
            TokenKind::Identifier(name) => Some(self.parse_identifier(name, token.span)),
            TokenKind::Symbol('(') => {
                let suppress_indices = self.suppress_indices;
                self.suppress_indices = false;
                let inner = self.parse_bp(0);
                self.suppress_indices = suppress_indices;
                let inner = inner?;
                if !matches!(self.current_kind(), Some(TokenKind::Symbol(')'))) {
                    self.diagnostics.push(Diagnostic::error(
                        DiagnosticCode::UnexpectedToken,
                        self.current_span(),
                        "missing ')'",
                    ));
                    return Some(inner);
                }
                let close = self.tokens[self.pos].span;
                self.pos += 1;
                Some(Expr {
                    span: token.span.join(close),
                    kind: ExprKind::Group(Box::new(inner)),
                })
            }
            _ => {
                self.diagnostics.push(Diagnostic::error(
                    DiagnosticCode::UnexpectedToken,
                    token.span,
                    "expression expected",
                ));
                Some(Expr {
                    kind: ExprKind::Error,
                    span: token.span,
                })
            }
        }
    }

    fn parse_identifier(&mut self, name: String, start: Span) -> Expr {
        if matches!(self.current_kind(), Some(TokenKind::Symbol('('))) {
            self.pos += 1;
            let mut args = Vec::new();
            while self.pos < self.tokens.len()
                && !matches!(self.current_kind(), Some(TokenKind::Symbol(')')))
            {
                if matches!(self.current_kind(), Some(TokenKind::Symbol(','))) {
                    args.push(None);
                    self.pos += 1;
                    continue;
                }
                let suppress_indices = self.suppress_indices;
                self.suppress_indices = false;
                let argument = self.parse_bp(0);
                self.suppress_indices = suppress_indices;
                args.push(argument);
                if matches!(self.current_kind(), Some(TokenKind::Symbol(','))) {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let end = if matches!(self.current_kind(), Some(TokenKind::Symbol(')'))) {
                let span = self.tokens[self.pos].span;
                self.pos += 1;
                span
            } else {
                self.current_span()
            };
            return Expr {
                kind: ExprKind::Call { name, args },
                span: start.join(end),
            };
        }
        let mut indices = Vec::new();
        while !self.suppress_indices && matches!(self.current_kind(), Some(TokenKind::Symbol(':')))
        {
            self.pos += 1;
            // `:` binds tighter than every binary operator. Suppressing another
            // index on the root operand makes `CFLAG:TARGET:位置` left-associative,
            // while parentheses and call arguments deliberately re-enable nested
            // variables such as `ARRAY:(LOCAL:1)`.
            let suppress_indices = self.suppress_indices;
            self.suppress_indices = true;
            let index = self.parse_bp(25);
            self.suppress_indices = suppress_indices;
            if let Some(index) = index {
                indices.push(index);
            } else {
                break;
            }
        }
        if indices.is_empty() {
            Expr {
                kind: ExprKind::Identifier(name),
                span: start,
            }
        } else {
            Expr {
                span: start.join(indices.last().map_or(start, |e| e.span)),
                kind: ExprKind::Variable { name, indices },
            }
        }
    }

    fn current_kind(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|token| &token.kind)
    }
    fn current_span(&self) -> Span {
        self.tokens.get(self.pos).map_or_else(
            || {
                self.tokens
                    .last()
                    .map_or(Span::default(), |t| Span::empty(t.span.end))
            },
            |t| t.span,
        )
    }
    fn postfix(&self) -> Option<PostfixOp> {
        match self.current_kind()? {
            TokenKind::Operator(Operator::Increment) => Some(PostfixOp::Increment),
            TokenKind::Operator(Operator::Decrement) => Some(PostfixOp::Decrement),
            _ => None,
        }
    }
    fn binary(&self) -> Option<(BinaryOp, u8, u8)> {
        binary_binding(self.current_kind()?)
    }
}

fn prefix_operator(kind: &TokenKind) -> Option<UnaryOp> {
    match kind {
        TokenKind::Operator(Operator::Add) => Some(UnaryOp::Plus),
        TokenKind::Operator(Operator::Subtract) => Some(UnaryOp::Minus),
        TokenKind::Operator(Operator::LogicalNot) => Some(UnaryOp::LogicalNot),
        TokenKind::Operator(Operator::BitNot) => Some(UnaryOp::BitNot),
        TokenKind::Operator(Operator::Increment) => Some(UnaryOp::PreIncrement),
        TokenKind::Operator(Operator::Decrement) => Some(UnaryOp::PreDecrement),
        _ => None,
    }
}

fn binary_binding(kind: &TokenKind) -> Option<(BinaryOp, u8, u8)> {
    let (op, precedence) = match kind {
        TokenKind::Operator(Operator::Multiply) => (BinaryOp::Multiply, 23),
        TokenKind::Operator(Operator::Divide) => (BinaryOp::Divide, 23),
        TokenKind::Operator(Operator::Modulo) => (BinaryOp::Modulo, 23),
        TokenKind::Operator(Operator::Add) => (BinaryOp::Add, 21),
        TokenKind::Operator(Operator::Subtract) => (BinaryOp::Subtract, 21),
        TokenKind::Operator(Operator::ShiftLeft) => (BinaryOp::ShiftLeft, 19),
        TokenKind::Operator(Operator::ShiftRight) => (BinaryOp::ShiftRight, 19),
        TokenKind::Operator(Operator::Less) => (BinaryOp::Less, 17),
        TokenKind::Operator(Operator::LessEqual) => (BinaryOp::LessEqual, 17),
        TokenKind::Operator(Operator::Greater) => (BinaryOp::Greater, 17),
        TokenKind::Operator(Operator::GreaterEqual) => (BinaryOp::GreaterEqual, 17),
        TokenKind::Operator(Operator::Equal) => (BinaryOp::Equal, 15),
        TokenKind::Operator(Operator::NotEqual) => (BinaryOp::NotEqual, 15),
        TokenKind::Operator(Operator::BitAnd) => (BinaryOp::BitAnd, 13),
        TokenKind::Operator(Operator::BitXor) => (BinaryOp::BitXor, 13),
        TokenKind::Operator(Operator::BitOr) => (BinaryOp::BitOr, 13),
        TokenKind::Operator(Operator::LogicalAnd) => (BinaryOp::LogicalAnd, 7),
        TokenKind::Operator(Operator::LogicalXor) => (BinaryOp::LogicalXor, 7),
        TokenKind::Operator(Operator::LogicalOr) => (BinaryOp::LogicalOr, 7),
        TokenKind::Operator(Operator::Nand) => (BinaryOp::Nand, 7),
        TokenKind::Operator(Operator::Nor) => (BinaryOp::Nor, 7),
        _ => return None,
    };
    Some((op, precedence, precedence + 1))
}
