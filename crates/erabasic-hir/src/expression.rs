use erabasic_ast::{Alignment, BinaryOp, PostfixOp, UnaryOp};
use serde::{Deserialize, Serialize};

use crate::{FunctionId, SourceLocation, VariableId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticType {
    Integer,
    String,
    Void,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ConstantValue {
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallTarget {
    Builtin { name: String },
    User { function: FunctionId },
    Extension { name: String },
    Unresolved { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HirPlace {
    pub variable: VariableId,
    pub indices: Vec<HirExpr>,
    pub value_type: SemanticType,
    pub mutable: bool,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HirExpr {
    pub kind: HirExprKind,
    pub value_type: SemanticType,
    pub constant: Option<ConstantValue>,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HirExprKind {
    Integer {
        value: i64,
    },
    String {
        value: String,
    },
    Variable {
        place: HirPlace,
    },
    Call {
        target: CallTarget,
        arguments: Vec<Option<HirExpr>>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<HirExpr>,
    },
    Postfix {
        op: PostfixOp,
        operand: Box<HirExpr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<HirExpr>,
        right: Box<HirExpr>,
    },
    Ternary {
        condition: Box<HirExpr>,
        then_expr: Box<HirExpr>,
        else_expr: Box<HirExpr>,
    },
    Formatted {
        value: HirFormattedString,
    },
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HirFormattedString {
    pub parts: Vec<HirFormPart>,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HirFormPart {
    Text {
        value: String,
    },
    Interpolation {
        expression: Box<HirExpr>,
        width: Option<Box<HirExpr>>,
        alignment: Option<Alignment>,
        integer: bool,
        location: SourceLocation,
    },
    Conditional {
        condition: Box<HirExpr>,
        then_value: Box<HirFormattedString>,
        else_value: Option<Box<HirFormattedString>>,
        location: SourceLocation,
    },
    Triple {
        symbol: char,
        location: SourceLocation,
    },
}
