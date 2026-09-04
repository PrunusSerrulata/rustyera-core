use era_debug_protocol::{
    AuthorizedDebugRequest, Breakpoint, BreakpointBinding, BreakpointLocation, CallStack,
    ConsoleCommand, DEBUG_PROTOCOL_VERSION, DebugCommand, DebugDiagnostic, DebugError,
    DebugErrorCode, DebugGrant, DebugHello, DebugMessage, DebugResponse, DebugScope,
    DebugSourceLocation, DebugStop, DebugValue, FiberPage, FiberState, FiberSummary,
    FieldMutability, FrameSummary, GameFieldDescriptor, GameFieldPage, GameFieldValue,
    GameFieldWriteOutcome, GrantToken, OperandStackPage, OperandValue, ResolvedBreakpoint,
    ScriptOutputChunk, StepKind, StopReason, StopToken, ValueKind, VariableDescriptor,
    VariablePage, VariableReference, VariableStorage, VariableValue, VariableWriteOutcome,
    grant_scopes,
};
use era_protocol::{ProtocolBytes, SessionId, VersionRange, encode_envelope, negotiate_version};
use erabasic_ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use erabasic_bytecode::{BytecodeStorage, Digest, SymbolKey};
use erabasic_parser::{DefaultParserContext, ParserContext, parse_expression};
use erabasic_vm::{
    FiberId, FiberStatus, FrameId, GenerationId, PlaceDescriptor, VmBreakpoint,
    VmBreakpointBinding, VmBreakpointLocation, VmDebugControl, VmDebugInspect, VmDebugStop,
    VmDebugStopReason, VmDebugVariable, VmDebugVariableRef, VmDebugVariableWrite, VmError,
    VmResolvedBreakpoint, VmRuntimePort, VmStepKind, VmStopToken, VmValue,
    evaluate_pure_native_with_compatibility,
};

use super::{ActiveDebugGrant, RuntimeError, RuntimeLogLevel, RuntimePhase, RuntimeSession};

const DEBUG_REQUEST_REJECTED: &str = "debug request rejected";


include!("dispatch/protocol.rs");
include!("dispatch/commands.rs");
include!("dispatch/lifecycle.rs");
