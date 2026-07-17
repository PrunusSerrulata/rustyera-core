//! Serializable, typed high-level representation of an `EraBasic` project.
//!
//! The HIR is intentionally independent from the analyzer's symbol tables. Stable
//! numeric IDs and flat line indices make the result deterministic and suitable for
//! a future compiler or VM without exposing analyzer implementation details.

mod expression;
mod ids;
mod program;
mod source;

pub use expression::{
    CallTarget, ConstantValue, HirCallArgument, HirExpr, HirExprKind, HirFormPart,
    HirFormattedString, HirPlace, SemanticType,
};
pub use ids::{FunctionId, LabelId, LineId, SourceId, VariableId};
pub use program::{
    ControlFlowEdge, ControlFlowKind, EventAttributes, Function, FunctionKind, HirArgument,
    HirStatement, HirStatementKind, InstructionTarget, Parameter, Program, Variable, VariableScope,
};
pub use source::{SourceFile, SourceLocation};

/// Version of the serialized HIR contract.
pub const HIR_FORMAT_VERSION: u32 = 7;
