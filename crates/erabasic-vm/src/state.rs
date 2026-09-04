use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::hash::BuildHasherDefault;
use std::sync::Arc;

use crate::debug::DebugState;
use crate::memory::SymbolKeyHasher;
use crate::regex_compat::RegexCache;
use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostReady, HostRequestId, Memory,
    NativeServiceRegistry, PlaceDescriptor, VariableCell, VariableMap, VmConfig, VmError, VmValue,
};
use crate::{
    PreparedRuntimeState, VmRuntimeFill, VmRuntimeRead, VmRuntimeStatePort,
    VmRuntimeStateTransaction,
};
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeFunction, BytecodeFunctionKind, BytecodeGlobal, BytecodeStorage,
    BytecodeType, Digest, ImportKind, Opcode, SourceMapEntry, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;

pub(crate) mod array_leases;
pub(crate) mod bit_calls;
mod derived_cache;
pub(crate) mod references;
mod returns;
pub(crate) use returns::FrameReturn;
mod path_memo;
mod planning;

pub(crate) use path_memo::path_memo_cache_usage;

use planning::{
    build_function_memo_plans, case_insensitive_index, index_source_entries, literal_group_match,
    memoized_indexed_read, path_memo_result_reads, simple_bulk_fill_loop, structured_scope_ranges,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StructuredScopeKind {
    Loop,
    Select,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StructuredScopeRange {
    kind: StructuredScopeKind,
    opener: usize,
    start: usize,
    end: usize,
}

pub(crate) struct StructuredJumpTransition {
    pub retain_loops: usize,
    pub retain_selects: usize,
    pub entered: Vec<StructuredScopeKind>,
}

type SymbolMap<T> = HashMap<SymbolKey, T, BuildHasherDefault<SymbolKeyHasher>>;

#[derive(Clone, Debug)]
pub(crate) struct ProgramGeneration {
    pub artifact: Arc<BytecodeArtifact>,
    function_indices: SymbolMap<usize>,
    function_name_indices: HashMap<String, usize>,
    // This map is lookup-only; authoritative globals remain canonically ordered
    // in the artifact, so hash iteration can never affect serialized output.
    global_indices: SymbolMap<usize>,
    variable_global_indices: Vec<Vec<u32>>,
    bulk_fill_loop_plans: Vec<Vec<(u32, BulkFillLoopPlan)>>,
    literal_group_match_plans: Vec<Vec<(u32, LiteralGroupMatchPlan)>>,
    function_memo_plans: Vec<Option<FunctionMemoPlan>>,
    memoized_indexed_read_plans: Vec<Option<MemoizedIndexedReadPlan>>,
    path_memo_result_read_plans: Vec<Vec<PathMemoResultReadPlan>>,
    // Canonical owner-free definitions always win system-name lookup.
    global_name_indices: HashMap<String, usize>,
    // Runtime inspection historically exposes otherwise unique function variables.
    runtime_name_fallback_indices: HashMap<String, usize>,
    target_global_index: Option<usize>,
    native_import_indices: SymbolMap<usize>,
    host_import_indices: SymbolMap<usize>,
    normalized_native_names: Vec<Arc<str>>,
    normalized_host_names: Vec<Arc<str>>,
    function_static_indices: SymbolMap<Vec<usize>>,
    function_local_indices: SymbolMap<Vec<usize>>,
    instruction_source_indices: Vec<Vec<u32>>,
    structured_scope_ranges: Vec<Vec<StructuredScopeRange>>,
}

const NO_SOURCE_MAP_ENTRY: u32 = u32::MAX;
const NO_GLOBAL_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmPreparationStage {
    InitializingMemory,
    IndexingProgram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmPreparationProgress {
    pub stage: VmPreparationStage,
    pub completed: u64,
    pub total: u64,
}

mod frames;
mod lifecycle;
mod places;
mod runtime;
mod runtime_types;
pub(crate) mod user_calls;

pub use runtime_types::Vm;
pub(crate) use runtime_types::{
    ActivePathMemo, BulkFillLoopPlan, EventDispatch, EventDispatchEntry, Fiber, FiberState,
    FindElementCacheKey, FindElementNeedle, ForLoopState, Frame, FunctionMemoEntry,
    FunctionMemoKey, FunctionMemoPlan, LiteralGroupMatchPlan, MemoValue, MemoizedIndexedReadPlan,
    PathMemoBaseKey, PathMemoCache, PathMemoDependency, PathMemoEntry, PathMemoHead,
    PathMemoMutation, PathMemoMutationGroup, PathMemoPlace, PathMemoResultReadPlan,
    PendingFaultHook, WaitingHost,
};

pub(crate) use frames::{
    PersistentArgumentDestination, bind_persistent_arguments, make_frame,
    persistent_argument_destination, validate_arguments,
};
use frames::{find_frame, find_frame_mut, find_global};
use runtime::replace_cell_values;

mod generation;
use generation::report_vm_preparation;
