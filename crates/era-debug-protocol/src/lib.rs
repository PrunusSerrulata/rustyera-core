//! Independent, capability-gated debugger protocol for a future Era runtime.
//!
//! Nothing in this crate can inspect a VM by itself. It defines versioned requests,
//! coherent stop tokens and typed responses for a later runtime implementation.

mod authorization;
mod breakpoint;
mod console;
mod execution;
mod message;
mod value;
mod variable;

pub use authorization::{DebugGrant, DebugHello, DebugRevoke, DebugScope, grant_scopes};
pub use breakpoint::{
    Breakpoint, BreakpointBinding, BreakpointLocation, BreakpointUpdate, ResolvedBreakpoint,
};
pub use console::{ConsoleCommand, ConsoleOutcome};
pub use execution::{
    CallStack, DebugSourceLocation, DebugStop, FiberPage, FiberState, FiberSummary, FrameSummary,
    OperandStackPage, OperandValue, StepKind, StopReason, StopToken,
};
pub use message::{
    AuthorizedDebugRequest, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugError, DebugErrorCode,
    DebugMessage, DebugResponse,
};
pub use value::{DebugPlace, DebugValue, ValueKind};
pub use variable::{
    FieldMutability, GameFieldDescriptor, GameFieldPage, GameFieldValue, GameFieldWrite,
    VariableDescriptor, VariablePage, VariableReference, VariableStorage, VariableValue,
    VariableWrite,
};
