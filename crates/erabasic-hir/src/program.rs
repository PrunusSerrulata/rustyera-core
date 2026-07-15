use erabasic_ast::AssignOp;
use erabasic_data::{Persistence, StorageScope};
use serde::{Deserialize, Serialize};

use crate::{
    ConstantValue, FunctionId, HIR_FORMAT_VERSION, HirExpr, HirFormattedString, HirPlace, LabelId,
    LineId, SemanticType, SourceFile, SourceLocation, VariableId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    Project,
    Function,
    Parameter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Variable {
    pub id: VariableId,
    pub name: String,
    pub value_type: SemanticType,
    pub dimensions: Vec<usize>,
    pub storage: StorageScope,
    pub persistence: Persistence,
    pub mutable: bool,
    pub reference: bool,
    pub static_lifetime: bool,
    pub initial_values: Vec<ConstantValue>,
    pub scope: VariableScope,
    pub owner: Option<FunctionId>,
    pub location: Option<SourceLocation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionKind {
    Normal,
    Event,
    System,
    Method,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub target: HirPlace,
    pub default: Option<HirExpr>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Function {
    pub id: FunctionId,
    pub name: String,
    pub kind: FunctionKind,
    pub return_type: SemanticType,
    pub parameters: Vec<Parameter>,
    pub lines: Vec<HirStatement>,
    pub labels: Vec<(LabelId, String, LineId)>,
    pub control_flow: Vec<ControlFlowEdge>,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HirStatement {
    pub id: LineId,
    pub kind: HirStatementKind,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum InstructionTarget {
    Builtin(String),
    Extension(String),
    Unresolved(String),
}

impl InstructionTarget {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin(name) | Self::Extension(name) | Self::Unresolved(name) => name,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HirStatementKind {
    Instruction {
        target: InstructionTarget,
        arguments: Vec<HirArgument>,
    },
    Assignment {
        target: HirPlace,
        op: AssignOp,
        value: HirExpr,
    },
    Label {
        label: LabelId,
        name: String,
    },
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum HirArgument {
    Expression(HirExpr),
    /// A mutable argument retains identity instead of being lowered to its current value.
    Place(HirPlace),
    Formatted(HirFormattedString),
    Raw(String),
    Omitted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFlowKind {
    Next,
    Branch,
    LoopBack,
    Break,
    Continue,
    Goto,
    Call,
    Jump,
    Return,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlFlowEdge {
    pub kind: ControlFlowKind,
    pub from: LineId,
    pub to: Option<LineId>,
    pub function: Option<FunctionId>,
    pub label: Option<LabelId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub format_version: u32,
    pub sources: Vec<SourceFile>,
    pub variables: Vec<Variable>,
    pub functions: Vec<Function>,
}

impl Program {
    #[must_use]
    pub fn new(sources: Vec<SourceFile>) -> Self {
        Self {
            format_version: HIR_FORMAT_VERSION,
            sources,
            variables: Vec::new(),
            functions: Vec::new(),
        }
    }
}
