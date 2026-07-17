use serde::{Deserialize, Serialize};

use crate::{BytecodeType, SymbolKey};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapability {
    Text,
    Graphics,
    Audio,
    Input,
    Clock,
    Storage,
    Network,
    System,
    Extension,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct HostEffect {
    pub pure: bool,
    pub may_suspend: bool,
    pub may_error: bool,
    pub mutates_runtime: bool,
}

/// Persisted execution semantics used by the VM, runtime transactions and debugger.
///
/// These fields deliberately describe observable state boundaries instead of Rust
/// implementation details. Keeping them in the artifact makes a missing or stale
/// classification fail validation before execution reaches a candidate save.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationContract {
    pub state: OperationState,
    pub transaction: TransactionPolicy,
    /// Behavior when the operation is reached by an isolated SAVEINFO run.
    ///
    /// This is deliberately independent from `transaction`: a system-flow
    /// command can be transactional during normal execution while still being
    /// forbidden in a save-description candidate.
    pub candidate: CandidatePolicy,
    pub persistence: OperationPersistence,
    pub snapshot: OperationSnapshotPolicy,
    pub hot_reload: OperationHotReloadPolicy,
    pub wait: OperationWaitPolicy,
    pub capability_fallback: CapabilityFallback,
    pub debug: OperationDebugPolicy,
    /// Portability provenance used for deterministic compiler diagnostics.
    pub portability: OperationPortability,
}

impl OperationContract {
    /// Derives the legacy execution flags from the persisted semantic contract.
    /// Keeping one derivation point prevents the scheduler and debugger from
    /// assigning different meanings to the same imported operation.
    #[must_use]
    pub const fn effect(self) -> HostEffect {
        HostEffect {
            pure: matches!(self.state, OperationState::Pure),
            may_suspend: !matches!(self.wait, OperationWaitPolicy::Immediate),
            may_error: true,
            mutates_runtime: matches!(
                self.transaction,
                TransactionPolicy::CloneCommit | TransactionPolicy::BufferedEffect
            ),
        }
    }

    #[must_use]
    pub const fn snapshot_capability(self) -> HostSnapshotCapability {
        match self.wait {
            OperationWaitPolicy::Immediate | OperationWaitPolicy::StableInput => {
                HostSnapshotCapability::StableWait
            }
            OperationWaitPolicy::TransientExternal => HostSnapshotCapability::Never,
        }
    }

    /// Checks combinations that would make rollback or wait eligibility
    /// ambiguous. Artifact validation additionally compares the derived legacy
    /// fields, so untrusted containers cannot smuggle a contradictory policy.
    #[must_use]
    pub const fn is_coherent(self) -> bool {
        if (matches!(self.candidate, CandidatePolicy::ReadOnly)
            && !matches!(self.transaction, TransactionPolicy::ReadOnly))
            || (matches!(self.candidate, CandidatePolicy::CloneCommit)
                && !matches!(self.transaction, TransactionPolicy::CloneCommit))
            || (matches!(self.candidate, CandidatePolicy::BufferedEffect)
                && !matches!(self.transaction, TransactionPolicy::BufferedEffect))
            || (matches!(self.candidate, CandidatePolicy::FrozenClock)
                && (!matches!(self.state, OperationState::External)
                    || !matches!(self.transaction, TransactionPolicy::Forbidden)
                    || !matches!(self.wait, OperationWaitPolicy::TransientExternal)))
        {
            return false;
        }
        if matches!(self.state, OperationState::Pure)
            && (!matches!(self.transaction, TransactionPolicy::ReadOnly)
                || !matches!(self.persistence, OperationPersistence::None)
                || !matches!(self.snapshot, OperationSnapshotPolicy::Included)
                || !matches!(self.hot_reload, OperationHotReloadPolicy::Preserve)
                || !matches!(self.wait, OperationWaitPolicy::Immediate)
                || !matches!(self.capability_fallback, CapabilityFallback::NotApplicable)
                || !matches!(self.debug, OperationDebugPolicy::Pure))
        {
            return false;
        }
        if matches!(self.debug, OperationDebugPolicy::Pure)
            && !matches!(self.transaction, TransactionPolicy::ReadOnly)
        {
            return false;
        }
        if matches!(self.debug, OperationDebugPolicy::Transactional)
            && !matches!(self.transaction, TransactionPolicy::CloneCommit)
        {
            return false;
        }
        if matches!(self.wait, OperationWaitPolicy::TransientExternal)
            && (!matches!(self.snapshot, OperationSnapshotPolicy::PendingBlocks)
                || !matches!(self.hot_reload, OperationHotReloadPolicy::ActiveBlocks))
        {
            return false;
        }
        if matches!(self.wait, OperationWaitPolicy::StableInput)
            && !matches!(self.snapshot, OperationSnapshotPolicy::Included)
        {
            return false;
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePolicy {
    ReadOnly,
    CloneCommit,
    BufferedEffect,
    /// Read the one clock value sampled before candidate execution starts.
    FrozenClock,
    Forbidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Pure,
    Vm,
    Native,
    Presentation,
    Controller,
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionPolicy {
    ReadOnly,
    CloneCommit,
    BufferedEffect,
    Forbidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPersistence {
    None,
    Ordinary,
    Global,
    /// Scope is resolved from the target `EraBasic` variable definition.
    VariableScoped,
    /// Scope is resolved from the project extension declaration for the key.
    ExtensionScoped,
    ProjectDerived,
    RuntimeOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationSnapshotPolicy {
    Included,
    Rebuild,
    Excluded,
    PendingBlocks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationHotReloadPolicy {
    Preserve,
    Rebuild,
    Invalidate,
    ActiveBlocks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationWaitPolicy {
    Immediate,
    StableInput,
    TransientExternal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFallback {
    NotApplicable,
    CanonicalProjection,
    IntentNoOp,
    ScriptResult,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationDebugPolicy {
    Pure,
    Transactional,
    Forbidden,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPortability {
    Portable,
    FrontendObservation,
    PlatformIntent,
    ExtensionDefined,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeImport {
    pub key: SymbolKey,
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub parameters: Vec<BytecodeType>,
    pub result: Option<BytecodeType>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeImportKind {
    Native,
    Host,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeImport {
    pub import: RuntimeImport,
    pub effect: HostEffect,
    pub contract: OperationContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostImport {
    pub import: RuntimeImport,
    pub effect: HostEffect,
    pub capability: HostCapability,
    /// This is the maximum wait stability the host is allowed to report. The
    /// result of an individual call can still downgrade a wait to transient.
    pub snapshot_capability: HostSnapshotCapability,
    pub contract: OperationContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSnapshotCapability {
    Never,
    StableWait,
}
