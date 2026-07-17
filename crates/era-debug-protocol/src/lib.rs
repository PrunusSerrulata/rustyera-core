//! Independent, capability-gated debugger protocol for the Era runtime.
//!
//! Nothing in this crate can inspect a VM by itself. It defines versioned requests,
//! coherent stop tokens and typed responses dispatched by `era-runtime`.

mod authorization;
mod breakpoint;
mod console;
mod execution;
mod message;
mod value;
mod variable;

pub use authorization::{
    DebugGrant, DebugHello, DebugRevoke, DebugScope, GrantToken, grant_scopes,
};
pub use breakpoint::{
    Breakpoint, BreakpointBinding, BreakpointLocation, BreakpointUpdate, ResolvedBreakpoint,
};
pub use console::{ConsoleCommand, ConsoleOutcome, DebugDiagnostic};
pub use execution::{
    CallStack, DebugSourceLocation, DebugStop, FiberPage, FiberState, FiberSummary, FrameSummary,
    OperandStackPage, OperandValue, StepKind, StopReason, StopToken,
};
pub use message::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugError, DebugErrorCode,
    DebugMessage, DebugResponse, ScriptOutputChunk,
};
pub use value::{DebugPlace, DebugValue, ValueKind};
pub use variable::{
    FieldMutability, GameFieldDescriptor, GameFieldPage, GameFieldValue, GameFieldWrite,
    GameFieldWriteOutcome, VariableDescriptor, VariablePage, VariableReference, VariableStorage,
    VariableValue, VariableWrite, VariableWriteOutcome,
};
