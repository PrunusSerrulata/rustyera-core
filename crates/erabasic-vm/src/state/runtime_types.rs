use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use erabasic_bytecode::{BytecodeType, SymbolKey};
use serde::{Deserialize, Serialize};

use super::ProgramGeneration;
use crate::debug::DebugState;
use crate::hot_reload::HotReloadPlan;
use crate::regex_compat::RegexCache;
use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostRequestId, HostWaitStability, Memory,
    PlaceDescriptor, VariableCell, VariableMap, VmConfig, VmExecutionOrigin, VmFault, VmValue,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub id: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: usize,
    pub stack: Vec<VmValue>,
    pub for_loops: Vec<ForLoopState>,
    pub select_values: Vec<VmValue>,
    pub locals: VariableMap,
    /// Dynamic statement calls discard method results without exposing them to Host code.
    pub return_value_to_caller: bool,
    /// Origin-checked return policy for a dynamically bound user call.
    pub user_call: Option<super::user_calls::UserCallFrame>,
    /// True for an event handler and every ordinary function called beneath it.
    pub event_context: bool,
    /// Nested CALLEVENT handlers are sequenced in the initiating caller frame.
    pub event_dispatch: Option<EventDispatch>,
    /// Late-bound STRFORM work owned by this frame and resumed by the scheduler.
    pub runtime_form: Option<crate::interpreter::dynamic_form::RuntimeFormContinuation>,
    /// Opaque method resolution identities are separate from the scalar operand stack.
    pub user_calls: Vec<super::user_calls::PendingUserCall>,
    /// Catch boundaries for the second EXISTVAR source evaluation.
    pub existvar_checks: Vec<crate::interpreter::existvar::ExistVarCheckpoint>,
}

impl Frame {
    pub(crate) fn operand_slots(&self) -> Option<usize> {
        self.user_calls.iter().try_fold(
            self.stack.len().checked_add(self.existvar_checks.len())?,
            |slots, call| slots.checked_add(1)?.checked_add(call.call.bindings.len()),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ForLoopState {
    pub counter: PlaceDescriptor,
    pub end: i64,
    pub step: i64,
}

impl ForLoopState {
    pub(crate) fn bypassed() -> Self {
        // Active FOR/REPEAT loops can never have a zero step, so this preserves the
        // snapshot schema while representing a structured scope entered by GOTO.
        Self {
            counter: PlaceDescriptor::default(),
            end: 0,
            step: 0,
        }
    }

    pub(crate) const fn is_bypassed(&self) -> bool {
        self.step == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct EventDispatchEntry {
    pub function: SymbolKey,
    pub single: bool,
    pub group: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct EventDispatch {
    pub active: EventDispatchEntry,
    pub pending: VecDeque<EventDispatchEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct WaitingHost {
    pub request: HostRequestId,
    pub import: erabasic_bytecode::RuntimeImport,
    pub result: Option<BytecodeType>,
    pub stability: HostWaitStability,
    pub rebind_payload: Vec<u8>,
    pub origin: VmExecutionOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum FiberState {
    Runnable,
    WaitingHost(WaitingHost),
    WaitingResume(BytecodeType),
    Completed(Option<VmValue>),
    Faulted(VmFault),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Fiber {
    pub id: FiberId,
    pub frames: Vec<Frame>,
    pub state: FiberState,
    pub backward_branches_without_progress: u64,
    pub consecutive_budget_exhaustions: u32,
}

impl Fiber {
    pub fn public_status(&self) -> FiberStatus {
        match &self.state {
            FiberState::Runnable => FiberStatus::Runnable,
            FiberState::WaitingHost(wait) => FiberStatus::WaitingHost(wait.request),
            FiberState::WaitingResume(_) => FiberStatus::WaitingResume,
            FiberState::Completed(value) => FiberStatus::Completed(value.clone()),
            FiberState::Faulted(fault) => FiberStatus::Faulted(fault.clone()),
            FiberState::Cancelled => FiberStatus::Cancelled,
        }
    }

    pub fn mark_progress(&mut self) {
        self.backward_branches_without_progress = 0;
        self.consecutive_budget_exhaustions = 0;
    }

    pub(crate) fn clear_runtime_forms(&mut self) {
        for frame in &mut self.frames {
            frame.runtime_form = None;
            frame.user_calls.clear();
            frame.existvar_checks.clear();
        }
    }
}

#[derive(Clone)]
pub struct Vm {
    pub(crate) config: VmConfig,
    pub(crate) generations: BTreeMap<GenerationId, Arc<ProgramGeneration>>,
    pub(crate) current_generation: GenerationId,
    pub(crate) memory: Memory,
    pub(crate) fibers: BTreeMap<FiberId, Fiber>,
    pub(crate) runnable: VecDeque<FiberId>,
    pub(crate) primary_fiber: Option<FiberId>,
    pub(crate) next_fiber: u64,
    pub(crate) next_frame: u64,
    pub(crate) next_request: u64,
    pub(crate) next_generation: u64,
    pub(crate) pending_reload: Option<HotReloadPlan>,
    pub(crate) compatibility_warning_sites: BTreeSet<(GenerationId, SymbolKey, usize, u8)>,
    pub(crate) pending_compatibility_warnings:
        Vec<crate::interpreter::compatibility_diagnostics::CompatibilityWarning>,
    pub(crate) debug: DebugState,
    pub(crate) regex_cache: RegexCache,
    pub(crate) find_element_cache: HashMap<FindElementCacheKey, i64>,
    pub(crate) find_element_cache_retained_bytes: usize,
    pub(crate) function_memo_cache: HashMap<FunctionMemoKey, FunctionMemoEntry>,
    pub(crate) function_memo_cache_retained_bytes: usize,
    pub(crate) active_function_memos: HashMap<FrameId, FunctionMemoKey>,
    pub(crate) path_memo_cache: PathMemoCache,
    pub(crate) path_memo_key_count: usize,
    pub(crate) path_memo_retained_bytes: usize,
    pub(crate) active_path_memo_fiber: Cell<Option<FiberId>>,
    pub(crate) active_path_memo: RefCell<Option<ActivePathMemo>>,
    #[cfg(test)]
    pub(crate) path_memo_replays: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FindElementNeedle {
    Integer(i64),
    String(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FindElementCacheKey {
    pub generation: GenerationId,
    pub variable: SymbolKey,
    pub revision: u64,
    pub start: usize,
    pub end: usize,
    pub last: bool,
    pub exact: bool,
    pub needle: FindElementNeedle,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum MemoValue {
    Integer(i64),
    String(String),
}

impl MemoValue {
    pub(crate) fn from_vm(value: &VmValue) -> Option<Self> {
        match value {
            VmValue::Integer(value) => Some(Self::Integer(*value)),
            VmValue::String(value) => Some(Self::String(value.clone())),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct FunctionMemoKey {
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub arguments: Vec<MemoValue>,
    pub dependency_revisions: Vec<u64>,
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionMemoPlan {
    pub dependency_indices: Vec<usize>,
    pub scratch_indices: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct MemoizedIndexedReadPlan {
    pub index_parameter: usize,
    pub selector_parameter: usize,
    pub selector_function: SymbolKey,
    pub selector_prefix: String,
    pub scratch: SymbolKey,
    pub target: SymbolKey,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PathMemoResultReadPlan {
    pub instruction: usize,
    pub variable: SymbolKey,
}

#[derive(Clone, Debug)]
pub(crate) struct BulkFillLoopPlan {
    pub prefix: SymbolKey,
    pub counter: SymbolKey,
    pub target: SymbolKey,
    pub value: VmValue,
    pub after_loop: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct LiteralGroupMatchPlan {
    pub candidates: Vec<Arc<str>>,
    pub after_call: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionMemoEntry {
    pub result: VmValue,
    pub scratch: Vec<(SymbolKey, VmValue)>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PathMemoHead {
    pub generation: GenerationId,
    pub function: SymbolKey,
}

#[derive(Clone, Debug)]
pub(crate) struct PathMemoBaseKey {
    pub head: PathMemoHead,
    pub arguments: Vec<VmValue>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PathMemoPlace {
    pub generation: GenerationId,
    pub variable: SymbolKey,
    /// Resolved character storage index, or zero for non-character storage.
    pub character: usize,
    pub indices: Vec<u64>,
}

#[derive(Clone, Debug)]
pub(crate) enum PathMemoDependency {
    Value {
        place: PathMemoPlace,
        value: VmValue,
    },
    CellRevision {
        generation: GenerationId,
        variable: SymbolKey,
        revision: u64,
    },
    TargetIdentity {
        generation: GenerationId,
        character: usize,
    },
}

impl PathMemoDependency {
    pub(crate) fn observes_cell_revision(
        &self,
        generation: GenerationId,
        variable: SymbolKey,
    ) -> bool {
        matches!(
            self,
            Self::CellRevision {
                generation: observed_generation,
                variable: observed_variable,
                ..
            } if *observed_generation == generation && *observed_variable == variable
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PathMemoMutation {
    Write {
        place: PathMemoPlace,
        value: VmValue,
    },
    Fill {
        generation: GenerationId,
        variable: SymbolKey,
        character: usize,
        start: usize,
        end: usize,
        value: VmValue,
    },
    Replace {
        generation: GenerationId,
        variable: SymbolKey,
        character: usize,
        values: Vec<VmValue>,
    },
}

impl PathMemoMutation {
    pub(crate) const fn cell_key(&self) -> (GenerationId, SymbolKey, usize) {
        match self {
            Self::Write { place, .. } => (place.generation, place.variable, place.character),
            Self::Fill {
                generation,
                variable,
                character,
                ..
            }
            | Self::Replace {
                generation,
                variable,
                character,
                ..
            } => (*generation, *variable, *character),
        }
    }

    pub(crate) fn covers_entire_cell(&self, length: usize) -> bool {
        match self {
            Self::Write { .. } => false,
            Self::Fill { start, end, .. } => *start == 0 && *end == length,
            Self::Replace { values, .. } => values.len() == length,
        }
    }

    pub(crate) fn writes_cell(&self, generation: GenerationId, variable: SymbolKey) -> bool {
        match self {
            Self::Write { place, .. } => {
                place.generation == generation && place.variable == variable
            }
            Self::Fill {
                generation: written_generation,
                variable: written_variable,
                ..
            }
            | Self::Replace {
                generation: written_generation,
                variable: written_variable,
                ..
            } => *written_generation == generation && *written_variable == variable,
        }
    }

    pub(crate) fn writes(&self, place: &PathMemoPlace) -> bool {
        match self {
            Self::Write { place: written, .. } => written == place,
            Self::Fill {
                generation,
                variable,
                character,
                start,
                end,
                ..
            } => {
                place.generation == *generation
                    && place.variable == *variable
                    && place.character == *character
                    && place.indices.len() <= 1
                    && place
                        .indices
                        .first()
                        .copied()
                        .unwrap_or(0)
                        .try_into()
                        .ok()
                        .is_some_and(|index: usize| index >= *start && index < *end)
            }
            Self::Replace {
                generation,
                variable,
                character,
                values,
            } => {
                place.generation == *generation
                    && place.variable == *variable
                    && place.character == *character
                    && place.indices.len() <= 1
                    && place
                        .indices
                        .first()
                        .copied()
                        .unwrap_or(0)
                        .try_into()
                        .ok()
                        .is_some_and(|index: usize| index < values.len())
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PathMemoEntry {
    pub dependencies: Vec<PathMemoDependency>,
    pub safe_natives: Vec<SymbolKey>,
    pub safe_hosts: Vec<SymbolKey>,
    pub mutation_groups: Vec<PathMemoMutationGroup>,
    pub result: VmValue,
    pub result_dependency: Option<usize>,
    pub body_instructions: u64,
    pub backward_branches: u64,
    pub retained_bytes: usize,
}

pub(crate) type PathMemoCache =
    HashMap<PathMemoHead, HashMap<Vec<VmValue>, Vec<Arc<PathMemoEntry>>>>;

#[derive(Clone, Debug)]
pub(crate) struct PathMemoMutationGroup {
    pub generation: GenerationId,
    pub variable: SymbolKey,
    pub character: usize,
    pub mutations: Vec<PathMemoMutation>,
    pub final_cell: Option<VariableCell>,
}

impl PathMemoMutationGroup {
    pub(crate) const fn key(&self) -> (GenerationId, SymbolKey, usize) {
        (self.generation, self.variable, self.character)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ActivePathMemo {
    pub fiber: FiberId,
    pub frame: FrameId,
    pub key: PathMemoBaseKey,
    pub dependencies: Vec<PathMemoDependency>,
    pub repeated_value_dependencies: BTreeSet<usize>,
    pub safe_natives: Vec<SymbolKey>,
    pub safe_hosts: Vec<SymbolKey>,
    pub mutations: Vec<PathMemoMutation>,
    pub pending_result_dependency: Option<(usize, usize)>,
    pub result_dependency: Option<usize>,
    pub retained_bytes: usize,
    pub body_instructions: u64,
    pub maximum_body_instructions: u64,
    pub backward_branches_before: u64,
    pub skip_call_instruction: bool,
    pub valid: bool,
}
