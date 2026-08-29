use erabasic_bytecode::ResolvedSourceLocation;
use serde::{Deserialize, Serialize};

use crate::{VmFault, VmValue};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(FiberId);
id_type!(FrameId);
id_type!(GenerationId);
id_type!(HostRequestId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmConfig {
    pub maximum_fibers: usize,
    pub maximum_call_depth: usize,
    pub maximum_operand_stack: usize,
    pub maximum_retained_generations: usize,
    pub maximum_backward_branches_without_progress: u64,
    pub maximum_consecutive_budget_exhaustions: u32,
    pub maximum_snapshot_bytes: usize,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            maximum_fibers: 1024,
            maximum_call_depth: 4096,
            maximum_operand_stack: 1_000_000,
            maximum_retained_generations: 8,
            maximum_backward_branches_without_progress: 10_000_000,
            maximum_consecutive_budget_exhaustions: 128,
            maximum_snapshot_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunBudget {
    pub maximum_instructions: u64,
    pub maximum_host_calls: u32,
    pub fiber_quantum: u32,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            maximum_instructions: 100_000,
            maximum_host_calls: 1024,
            fiber_quantum: 4096,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmRunStop {
    Idle,
    BudgetExhausted,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VmDiagnosticNotification {
    #[default]
    Default,
    LogOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmEvent {
    Diagnostic {
        fiber: FiberId,
        code: String,
        message: String,
        origin: VmExecutionOrigin,
        notification: VmDiagnosticNotification,
    },
    HostPending {
        fiber: FiberId,
        request: HostRequestId,
    },
    FiberYielded {
        fiber: FiberId,
    },
    FiberCompleted {
        fiber: FiberId,
        value: Option<VmValue>,
    },
    FiberFaulted {
        fiber: FiberId,
        fault: VmFault,
    },
    DebugStopped(crate::VmDebugStop),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmRunReport {
    pub stop: VmRunStop,
    pub instructions: u64,
    pub host_calls: u32,
    pub events: Vec<VmEvent>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FiberStatus {
    Runnable,
    WaitingHost(HostRequestId),
    WaitingResume,
    Completed(Option<VmValue>),
    Faulted(VmFault),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VmBacktraceFrame {
    pub function: String,
    pub source: Option<ResolvedSourceLocation>,
}

/// Immutable source identity captured before an instruction crosses the
/// caller-pumped Host boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VmExecutionOrigin {
    pub generation: GenerationId,
    pub function: erabasic_bytecode::SymbolKey,
    pub function_name: String,
    pub instruction: u32,
    pub command: String,
    pub source: Option<ResolvedSourceLocation>,
}

/// VM-owned occurrence of a direct Host expression. This identity is never supplied
/// by a frontend; runtime uses it only to retain or discard its own staged resources.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeHostScope {
    pub fiber: crate::FiberId,
    pub frame: crate::FrameId,
    pub generation: GenerationId,
    pub function: erabasic_bytecode::SymbolKey,
    pub instruction: u32,
    pub occurrence: u64,
}
