//! Non-serialized expression shape used by runtime-form type analysis.
use crate::BytecodeType;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeExpressionShape {
    pub value_type: BytecodeType,
    pub variable: bool,
    pub mutable: bool,
}
