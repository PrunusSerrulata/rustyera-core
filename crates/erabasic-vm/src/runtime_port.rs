use erabasic_bytecode::{Digest, HostImport, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::{
    EraState, EraStateReport, FiberId, FiberStatus, GenerationId, HostReady, HostRequestId,
    HostWaitStability, HotReloadReport, RunBudget, SnapshotEligibility, VmConfig, VmError,
    VmSnapshot, VmValue,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmRuntimeRead {
    pub variable: SymbolKey,
    pub indices: Vec<u64>,
    pub character: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmRuntimeWrite {
    pub variable: SymbolKey,
    pub indices: Vec<u64>,
    pub character: Option<u64>,
    pub value: VmValue,
}

/// Mutations needed by the reference system controller. Script instructions keep
/// using regular bytecode operations; this transaction is only legal between slices.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmRuntimeStateTransaction {
    ResetNewGame,
    RestoreEraState(Box<EraState>),
    Mutate {
        writes: Vec<VmRuntimeWrite>,
        clear_characters: bool,
        add_characters_from_csv: Vec<i64>,
    },
}

/// Opaque commit token containing a fully validated candidate memory image.
pub struct PreparedRuntimeState {
    pub(crate) generation: GenerationId,
    pub(crate) memory: crate::Memory,
    pub(crate) reset_execution: bool,
}

/// Transactional state access used by the runtime's built-in system controller.
/// It intentionally excludes frame-local places and exposes no VM-owned references.
pub trait VmRuntimeStatePort {
    /// # Errors
    ///
    /// Returns an error for frame-local, missing, or out-of-bounds storage.
    fn read_runtime_state(&self, reads: &[VmRuntimeRead]) -> Result<Vec<VmValue>, VmError>;

    /// Validate the entire operation against a cloned memory image.
    ///
    /// # Errors
    ///
    /// Returns an error without changing the VM when any mutation is invalid.
    fn prepare_runtime_state(
        &self,
        transaction: VmRuntimeStateTransaction,
    ) -> Result<PreparedRuntimeState, VmError>;

    /// # Errors
    ///
    /// Returns an error if the program generation changed after preparation.
    fn commit_runtime_state(&mut self, prepared: PreparedRuntimeState) -> Result<(), VmError>;
}

/// Prospective execution mode used by a runtime actor or debugger. Normal scheduling
/// remains fair; selected-fiber mode is only valid while every other fiber is paused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmDriveMode {
    Normal,
    SelectedFiber(FiberId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmPortStop {
    Idle,
    BudgetExhausted,
    DebugStopped,
}

/// A `CallHost` request captured after the interpreter has left its instruction
/// dispatch stack. Runtime code must never be invoked from instruction execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmHostRequest {
    pub id: HostRequestId,
    pub fiber: FiberId,
    pub import: HostImport,
    pub arguments: Vec<VmValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmHostCompletion {
    Ready(HostReady),
    Pending {
        stability: HostWaitStability,
        rebind_payload: Vec<u8>,
    },
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmPortEvent {
    HostCall(VmHostRequest),
    FiberYielded(FiberId),
    FiberCompleted(FiberId, Option<VmValue>),
    FiberFaulted(FiberId, crate::VmFault),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmPortDriveReport {
    pub stop: VmPortStop,
    pub instructions: u64,
    pub events: Vec<VmPortEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmWaitRebind {
    pub request: HostRequestId,
    pub fiber: FiberId,
    pub import: erabasic_bytecode::RuntimeImport,
    pub payload: Vec<u8>,
}

/// Runtime/VM port used by the caller-pumped runtime actor.
///
/// This trait intentionally contains no runtime callback. `drive` first returns host
/// requests; the runtime stages its own state transition and asks the VM to validate a
/// completion. With one serialized owner, committing the returned token cannot observe
/// an intervening VM mutation.
///
/// [`crate::RuntimeVm`] adapts the interpreter to this contract. The lower-level `Vm`
/// callback API remains available for embedders that do not use the runtime actor.
pub trait VmRuntimePort {
    type PreparedCompletion;

    fn artifact_id(&self) -> Digest;
    fn current_generation(&self) -> GenerationId;
    /// Spawn a root entry in the current generation.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown function, invalid arguments or fiber limits.
    fn spawn_entry(
        &mut self,
        function: SymbolKey,
        arguments: Vec<VmValue>,
    ) -> Result<FiberId, VmError>;
    fn fiber_status(&self, fiber: FiberId) -> Option<FiberStatus>;
    fn drive(&mut self, budget: RunBudget, mode: VmDriveMode) -> VmPortDriveReport;

    /// Validate types, places and request freshness without mutating VM state.
    ///
    /// # Errors
    ///
    /// Returns an error for stale requests, invalid waits, values or writes.
    fn validate_host_completion(
        &self,
        request: HostRequestId,
        completion: VmHostCompletion,
    ) -> Result<Self::PreparedCompletion, VmError>;

    /// Commit a token produced by `validate_host_completion`. A future implementation
    /// must consume tokens exactly once and reject stale program generations.
    ///
    /// # Errors
    ///
    /// Returns an error if the prepared token is stale or has already been consumed.
    fn commit_host_completion(
        &mut self,
        completion: Self::PreparedCompletion,
    ) -> Result<FiberId, VmError>;

    /// # Errors
    ///
    /// Returns an error if the fiber does not exist.
    fn cancel_fiber(&mut self, fiber: FiberId) -> Result<(), VmError>;
    fn export_era_state(&self) -> EraState;
    /// # Errors
    ///
    /// Returns an error when saved data is incompatible with the active project.
    fn restore_era_state(&mut self, state: &EraState) -> Result<EraStateReport, VmError>;
    fn snapshot_eligibility(&self) -> SnapshotEligibility;
    /// # Errors
    ///
    /// Returns an error unless the VM is at a stable, snapshot-capable input wait.
    fn snapshot(&self) -> Result<VmSnapshot, VmError>;
    /// # Errors
    ///
    /// Returns an error for incompatible storage or generation/resource limits.
    fn prepare_hot_reload(&mut self, target: ValidatedArtifact) -> Result<(), VmError>;
    /// # Errors
    ///
    /// Returns an error if no valid plan is pending or the base generation changed.
    fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError>;
}

/// Snapshot restore is also split into preparation and commit. A runtime recreates
/// canonical waits from `waits` after preparation and before it consumes the plan.
pub trait VmRestorePort: Sized {
    type PreparedRestore;

    /// # Errors
    ///
    /// Returns an error for format, artifact, version or VM-invariant mismatches.
    fn prepare_restore(
        artifact: ValidatedArtifact,
        config: VmConfig,
        snapshot: VmSnapshot,
    ) -> Result<Self::PreparedRestore, VmError>;
    fn restore_waits(plan: &Self::PreparedRestore) -> &[VmWaitRebind];
    /// # Errors
    ///
    /// Returns an error if the prepared restore plan can no longer be committed.
    fn commit_restore(plan: Self::PreparedRestore) -> Result<Self, VmError>;
}
