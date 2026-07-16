//! Deterministic, I/O-free `EraBasic` bytecode execution.
//!
//! The VM owns language state and cooperative fibers. A runtime supplies native
//! services and the single [`VmHost`] boundary, while an application frontend owns
//! files, clocks, rendering and input delivery.

mod config;
mod debug;
mod debug_port;
mod fault;
mod host;
mod hot_reload;
mod interpreter;
mod memory;
mod regex_compat;
mod runtime_port;
mod runtime_vm;
mod save;
mod sfmt;
mod snapshot;
mod state;
mod structured;
mod value;

pub use config::{
    FiberId, FiberStatus, FrameId, GenerationId, HostRequestId, RunBudget, VmBacktraceFrame,
    VmConfig, VmEvent, VmExecutionOrigin, VmRunReport, VmRunStop,
};
pub use debug_port::{
    VmBreakpoint, VmBreakpointBinding, VmBreakpointLocation, VmDebugControl, VmDebugFiber,
    VmDebugFrame, VmDebugInspect, VmDebugOperand, VmDebugPage, VmDebugStop, VmDebugStopReason,
    VmDebugVariable, VmDebugVariableRef, VmDebugVariableWrite, VmResolvedBreakpoint, VmStepKind,
    VmStopToken,
};
pub use fault::{VmError, VmFault, VmFaultCode};
pub use host::{
    HostCallRequest, HostCallResult, HostReady, HostRebindRequest, HostWaitStability,
    NativeCallRequest, NativePlaceView, NativeReady, NativeService, NativeServiceRegistry, VmHost,
};
pub use hot_reload::{HotReloadPlan, HotReloadReport};
pub use runtime_port::{
    PreparedRuntimeState, VmDriveMode, VmHostCompletion, VmHostRequest, VmPortDriveReport,
    VmPortEvent, VmPortStop, VmRestorePort, VmRuntimeFill, VmRuntimePort, VmRuntimeRead,
    VmRuntimeStatePort, VmRuntimeStateTransaction, VmRuntimeWrite, VmWaitRebind,
};
pub use runtime_vm::{PreparedHostCompletion, RuntimeVm};
pub use save::{EraSaveScope, EraState, EraStateReport, EraVariableState};
pub use snapshot::{
    SNAPSHOT_FORMAT_VERSION, SNAPSHOT_MAGIC, SnapshotBlocker, SnapshotEligibility, VmSnapshot,
};
pub use state::Vm;
pub use structured::{StructuredExtension, StructuredScope};
pub use value::{HostWrite, PlaceDescriptor, VmValue};

pub(crate) use memory::{Memory, VariableCell};
pub(crate) use state::{
    Fiber, FiberState, ProgramGeneration, WaitingHost, find_global, make_frame, validate_arguments,
};
