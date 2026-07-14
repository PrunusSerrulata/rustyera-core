//! Syntax tree shared by the `EraBasic` lexer and parser.
//!
//! The tree intentionally omits trivia. Every meaningful node retains a UTF-8
//! byte span, so clients can slice the original source without converting
//! between Emuera's UTF-16 indexing and Rust's string representation.
//! This crate does not implement semantic analysis.

use std::fmt;

/// A half-open UTF-8 byte range in one source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn empty(at: usize) -> Self {
        Self::new(at, at)
    }

    #[must_use]
    pub fn join(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }
}

/// Diagnostic severity follows Emuera's warning levels while exposing names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Warning,
    Error,
}

/// Stable, language-independent diagnostic categories.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    UnexpectedCharacter,
    UnexpectedToken,
    UnterminatedString,
    UnterminatedFormattedString,
    InvalidEscape,
    InvalidInteger,
    IntegerOverflow,
    InvalidOperator,
    MissingExpression,
    InvalidAssignment,
    InvalidDirective,
    UnknownInstruction,
    UnknownIdentifier,
    DuplicateDeclaration,
    UnmatchedBlock,
    InvalidPreprocessor,
    MacroRecursion,
    TrailingInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    #[must_use]
    pub fn error(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            span,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn warning(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            span,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Plus,
    Minus,
    LogicalNot,
    BitNot,
    PreIncrement,
    PreDecrement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostfixOp {
    Increment,
    Decrement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Multiply,
    Divide,
    Modulo,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    BitAnd,
    BitXor,
    BitOr,
    LogicalAnd,
    LogicalXor,
    LogicalOr,
    Nand,
    Nor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignOp {
    Assign,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Integer(i64),
    String(String),
    Identifier(String),
    Variable {
        name: String,
        indices: Vec<Expr>,
    },
    Call {
        name: String,
        args: Vec<Option<Expr>>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Postfix {
        op: PostfixOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Ternary {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    Formatted(FormattedString),
    Group(Box<Expr>),
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormattedString {
    pub parts: Vec<FormPart>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FormPart {
    Text(String),
    StringInterpolation {
        expression: Box<Expr>,
        width: Option<Box<Expr>>,
        alignment: Option<Alignment>,
        span: Span,
    },
    IntegerInterpolation {
        expression: Box<Expr>,
        width: Option<Box<Expr>>,
        alignment: Option<Alignment>,
        span: Span,
    },
    Conditional {
        condition: Box<Expr>,
        then_value: Box<FormattedString>,
        else_value: Option<Box<FormattedString>>,
        span: Span,
    },
    Triple {
        symbol: char,
        span: Span,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariableRef {
    pub name: String,
    pub indices: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatementKind {
    Instruction {
        name: String,
        arguments: Vec<Argument>,
        raw_arguments: String,
    },
    Assignment {
        target: VariableRef,
        op: AssignOp,
        value: Expr,
    },
    GotoLabel {
        name: String,
    },
    Directive(Directive),
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Argument {
    Expression(Expr),
    Formatted(FormattedString),
    Raw(String),
    Omitted(Span),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    pub name: String,
    pub arguments: Vec<Argument>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub attributes: Vec<Directive>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub default: Option<Expr>,
    pub is_reference: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Erb,
    Erh,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub kind: SourceKind,
    pub functions: Vec<Function>,
    pub declarations: Vec<Directive>,
    pub top_level: Vec<Statement>,
    pub span: Span,
}

/// A parser result can contain a value and recoverable diagnostics together.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseOutput<T> {
    pub value: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
}

impl<T> ParseOutput<T> {
    #[must_use]
    pub fn success(value: T) -> Self {
        Self {
            value: Some(value),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?}[{}..{}]: {}",
            self.severity, self.span.start, self.span.end, self.message
        )
    }
}
