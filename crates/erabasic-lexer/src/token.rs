use erabasic_ast::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operator {
    Assign,
    StringAssign,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    AddAssign,
    SubtractAssign,
    MultiplyAssign,
    DivideAssign,
    ModuloAssign,
    LogicalAnd,
    LogicalOr,
    LogicalXor,
    Nand,
    Nor,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    BitAndAssign,
    BitOrAssign,
    BitXorAssign,
    ShiftLeft,
    ShiftRight,
    ShiftLeftAssign,
    ShiftRightAssign,
    LogicalNot,
    Increment,
    Decrement,
    Question,
    TernarySeparator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub from_macro: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Integer(i64),
    String(String),
    Operator(Operator),
    Symbol(char),
    Formatted(FormattedToken),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormattedToken {
    pub parts: Vec<FormattedTokenPart>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FormattedTokenPart {
    Text(String),
    StringInterpolation {
        tokens: Vec<Token>,
        span: Span,
    },
    IntegerInterpolation {
        tokens: Vec<Token>,
        span: Span,
    },
    Conditional {
        condition: Vec<Token>,
        then_value: Box<FormattedToken>,
        else_value: Option<Box<FormattedToken>>,
        span: Span,
    },
    Triple {
        symbol: char,
        span: Span,
    },
}
