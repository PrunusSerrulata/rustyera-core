use std::fmt;

use erabasic_bytecode::{ResolvedSourceLocation, SymbolKey};
use serde::{Deserialize, Serialize};

use crate::{FiberId, GenerationId};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmFaultCode {
    InvalidInstruction,
    StackUnderflow,
    TypeMismatch,
    Bounds,
    DivideByZero,
    MissingSymbol,
    Host,
    Native,
    Trap,
    ResourceLimit,
    RunawayExecution,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VmFault {
    pub code: VmFaultCode,
    pub message: String,
    pub fiber: FiberId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: u32,
    pub source: Option<ResolvedSourceLocation>,
}

impl fmt::Display for VmFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for VmFault {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmError {
    MissingFunction(SymbolKey),
    InvalidArguments(String),
    ResourceLimit(&'static str),
    InvalidState(String),
    UnknownFiber(FiberId),
    StaleHostRequest(crate::HostRequestId),
    HotReload(String),
    Snapshot(String),
    Save(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for VmError {}
