use erabasic_bytecode::{Digest, ResolvedSourceLocation, SymbolKey};

use crate::{FiberId, FiberStatus, FrameId, GenerationId, PlaceDescriptor, VmError, VmValue};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmStopToken {
    pub pause_epoch: u64,
    pub generation: GenerationId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmStepKind {
    Instruction,
    SourceLine,
    Into,
    Over,
    Out,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmDebugStopReason {
    PauseRequested,
    Breakpoint(u64),
    StepCompleted,
    HostWait,
    FiberCompleted,
    Fault(crate::VmFault),
    Reload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugStop {
    pub token: VmStopToken,
    pub reason: VmDebugStopReason,
    pub selected_fiber: Option<FiberId>,
    pub source: Option<ResolvedSourceLocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugFiber {
    pub id: FiberId,
    pub status: FiberStatus,
    pub primary: bool,
    pub frame_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugFrame {
    pub id: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub function_name: String,
    pub instruction: u32,
    pub source: Option<ResolvedSourceLocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugOperand {
    pub offset: usize,
    pub value: VmValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugPage<T> {
    pub values: Vec<T>,
    pub next_cursor: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugVariableRef {
    pub target: PlaceDescriptor,
    pub generation: GenerationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugVariable {
    pub target: VmDebugVariableRef,
    pub name: String,
    pub mutable: bool,
    pub value: VmValue,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmDebugVariableWrite {
    pub target: VmDebugVariableRef,
    pub value: VmValue,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmBreakpointLocation {
    Source {
        relative_path: String,
        content_hash: Digest,
        byte_offset: u64,
    },
    Function(SymbolKey),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmBreakpoint {
    pub id: u64,
    pub enabled: bool,
    pub hit_count: u64,
    pub location: VmBreakpointLocation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmBreakpointBinding {
    Verified,
    Moved,
    Unbound,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmResolvedBreakpoint {
    pub id: u64,
    pub generation: GenerationId,
    pub binding: VmBreakpointBinding,
    pub source: Option<ResolvedSourceLocation>,
    pub message: Option<String>,
    pub hit_count: u64,
}

/// Read-only inspection interface. The absence of operand/frame mutation methods is
/// intentional and forms part of the debugger security contract.
pub trait VmDebugInspect {
    fn stop_token(&self) -> Option<VmStopToken>;
    /// # Errors
    ///
    /// Returns an error for stale stop tokens or invalid pagination limits.
    fn fibers(
        &self,
        stop: VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugFiber>, VmError>;
    /// # Errors
    ///
    /// Returns an error for a stale stop token or unknown fiber.
    fn call_stack(&self, stop: VmStopToken, fiber: FiberId) -> Result<Vec<VmDebugFrame>, VmError>;
    /// # Errors
    ///
    /// Returns an error for a stale stop token, unknown frame or invalid page.
    fn operand_stack(
        &self,
        stop: VmStopToken,
        fiber: FiberId,
        frame: FrameId,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugOperand>, VmError>;
    /// # Errors
    ///
    /// Returns an error for stale stop tokens or invalid pagination limits.
    fn variables(
        &self,
        stop: VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<VmDebugPage<VmDebugVariable>, VmError>;
    /// # Errors
    ///
    /// Returns an error for stale stop tokens or unavailable variable storage.
    fn read_variable(
        &self,
        stop: VmStopToken,
        target: &VmDebugVariableRef,
    ) -> Result<VmDebugVariable, VmError>;
}

/// Controlled VM mutations requested by the runtime. Runtime-owned game fields and
/// debugger authorization remain outside this interface.
pub trait VmDebugControl {
    /// # Errors
    ///
    /// Returns an error if a pause is already pending or execution cannot reach a safe point.
    fn request_pause(&mut self) -> Result<VmDebugStop, VmError>;
    /// # Errors
    ///
    /// Returns an error for a stale stop token or invalid execution state.
    fn continue_execution(&mut self, stop: VmStopToken) -> Result<(), VmError>;
    /// # Errors
    ///
    /// Returns an error for a stale stop token, unknown fiber or invalid step target.
    fn step(&mut self, stop: VmStopToken, fiber: FiberId, kind: VmStepKind) -> Result<(), VmError>;
    /// # Errors
    ///
    /// Returns an error if any target, type, index or expected revision is invalid;
    /// implementations must leave every variable unchanged in that case.
    fn write_variables(
        &mut self,
        stop: VmStopToken,
        writes: &[VmDebugVariableWrite],
    ) -> Result<Vec<VmDebugVariable>, VmError>;
    /// # Errors
    ///
    /// Returns an error for invalid identifiers, locations or source-map resolution.
    fn update_breakpoints(
        &mut self,
        breakpoints: &[VmBreakpoint],
        remove: &[u64],
    ) -> Result<Vec<VmResolvedBreakpoint>, VmError>;
}
