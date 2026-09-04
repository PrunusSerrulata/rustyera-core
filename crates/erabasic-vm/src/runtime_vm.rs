use erabasic_bytecode::{Digest, HostImport, HostSnapshotCapability, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::structured::{ColumnIdentityStamp, StructuredExtension, StructuredScope};
use crate::{
    EraState, EraStateReport, FiberId, FiberState, FiberStatus, GenerationId, HostCallRequest,
    HostCallResult, HostReady, HostRequestId, HostWaitStability, HotReloadReport,
    ImmediateHostCall, ImmediateHostCallResult, NativeServiceRegistry, PlaceDescriptor,
    PreparedRuntimeState, RunBudget, SnapshotEligibility, Vm, VmConfig, VmDriveMode, VmError,
    VmHost, VmHostCompletion, VmHostRequest, VmPortDriveReport, VmPortEvent, VmPortStop,
    VmRestorePort, VmRuntimePort, VmRuntimeRead, VmRuntimeStatePort, VmRuntimeStateTransaction,
    VmSnapshot, VmValue, VmWaitRebind,
};
use std::collections::BTreeSet;

use crate::debug::DebugState;

/// Runtime-facing VM owner. It keeps native services beside the interpreter so the
/// caller-pumped runtime port never needs a callback parameter.
pub struct RuntimeVm {
    vm: Vm,
    natives: NativeServiceRegistry,
    pending_natives: Option<(
        NativeServiceRegistry,
        Option<ColumnIdentityStamp>,
        Option<crate::structured::MapLeaseStamp>,
    )>,
    candidate_base_column_stamp: CandidateColumnBase,
    candidate_base_array_stamp: Option<crate::state::array_leases::ArrayLeaseStamp>,
    line_columns: u32,
    pending_completion_events: Vec<VmPortEvent>,
}

/// Distinguish an unforked runtime from a fork whose artifact has no structured services.
#[derive(Clone, Copy)]
enum CandidateColumnBase {
    Unforked,
    Forked(
        Option<ColumnIdentityStamp>,
        Option<crate::structured::MapLeaseStamp>,
    ),
}

/// The immutable program index retained while a runtime obtains title entropy.
///
/// Consuming a [`RuntimeVm`] into this type releases game memory, fibers, scheduler
/// state, derived caches, Native services, layout and VM configuration without
/// rebuilding the program index when the title timeline starts.
pub struct RetainedProgramIndex {
    program: std::sync::Arc<crate::ProgramGeneration>,
}

impl RetainedProgramIndex {
    /// Identify the exact artifact whose immutable index is retained.
    #[must_use]
    pub fn artifact_id(&self) -> Digest {
        self.program.artifact.manifest.artifact_id
    }
}

/// Stable logical width used until a frontend reports its projection dimensions.
pub const DEFAULT_LINE_COLUMNS: u32 = 75;

/// Opaque candidate state prepared against one exact artifact generation.
/// It intentionally excludes fibers, frames and scheduler counters.
pub struct PreparedCandidateState {
    artifact_id: Digest,
    base_column_stamp: CandidateColumnBase,
    base_array_stamp: Option<crate::state::array_leases::ArrayLeaseStamp>,
    memory: crate::Memory,
    natives: NativeServiceRegistry,
}

mod candidate;
mod debug;
mod ports;
mod restore;
#[cfg(test)]
mod tests;

pub use ports::PreparedHostCompletion;
