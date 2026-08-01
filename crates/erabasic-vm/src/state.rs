use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeFunctionKind, BytecodeStorage,
    BytecodeType, Digest, SourceMapEntry, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;
use serde::{Deserialize, Serialize};

use crate::debug::DebugState;
use crate::regex_compat::RegexCache;
use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostReady, HostRequestId, HostWaitStability,
    Memory, PlaceDescriptor, VariableCell, VmConfig, VmError, VmExecutionOrigin, VmFault, VmValue,
    hot_reload::HotReloadPlan,
};
use crate::{
    PreparedRuntimeState, VmRuntimeFill, VmRuntimeRead, VmRuntimeStatePort,
    VmRuntimeStateTransaction,
};

#[derive(Clone, Debug)]
pub(crate) struct ProgramGeneration {
    pub artifact: Arc<BytecodeArtifact>,
    function_indices: HashMap<SymbolKey, usize>,
    function_name_indices: HashMap<String, usize>,
    // This map is lookup-only; authoritative globals remain canonically ordered
    // in the artifact, so hash iteration can never affect serialized output.
    global_indices: HashMap<SymbolKey, usize>,
    global_name_indices: HashMap<String, usize>,
    target_global_index: Option<usize>,
    native_import_indices: HashMap<SymbolKey, usize>,
    host_import_indices: HashMap<SymbolKey, usize>,
    normalized_native_names: Vec<Arc<str>>,
    function_static_indices: BTreeMap<SymbolKey, Vec<usize>>,
    function_local_indices: BTreeMap<SymbolKey, Vec<usize>>,
    instruction_source_indices: BTreeMap<SymbolKey, Vec<u32>>,
}

const NO_SOURCE_MAP_ENTRY: u32 = u32::MAX;

impl ProgramGeneration {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn new(artifact: Arc<BytecodeArtifact>) -> Self {
        // Era projects commonly contain tens of thousands of functions. Resolving the
        // active function with a linear scan for every instruction makes otherwise
        // lightweight EraBasic execution quadratic in the project size.
        let function_indices = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.key, index))
            .collect();
        let mut function_name_indices = HashMap::new();
        for (index, function) in artifact.functions.iter().enumerate() {
            // Dynamic lookup follows the artifact order when duplicate declarations
            // are permitted by the selected compatibility mode.
            function_name_indices
                .entry(function.name.to_ascii_uppercase())
                .or_insert(index);
        }
        let global_indices = artifact
            .globals
            .iter()
            .enumerate()
            .map(|(index, global)| (global.key, index))
            .collect();
        let mut global_name_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            global_name_indices
                .entry(global.name.to_ascii_uppercase())
                .or_insert(index);
        }
        let target_global_index = global_name_indices.get("TARGET").copied();
        let mut native_import_indices = HashMap::new();
        for (index, import) in artifact.native_imports.iter().enumerate() {
            native_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let normalized_native_names = artifact
            .native_imports
            .iter()
            .map(|import| Arc::<str>::from(import.import.name.to_ascii_lowercase()))
            .collect();
        let mut host_import_indices = HashMap::new();
        for (index, import) in artifact.host_imports.iter().enumerate() {
            host_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let mut function_static_indices = BTreeMap::<SymbolKey, Vec<usize>>::new();
        let mut function_local_indices = BTreeMap::<SymbolKey, Vec<usize>>::new();
        let mut function_names_by_key = BTreeMap::<SymbolKey, String>::new();
        let mut function_keys_by_name = BTreeMap::<String, Vec<SymbolKey>>::new();
        for function in &artifact.functions {
            let normalized = function.name.to_ascii_uppercase();
            function_names_by_key.insert(function.key, normalized.clone());
            function_keys_by_name
                .entry(normalized)
                .or_default()
                .push(function.key);
        }
        for (index, global) in artifact.globals.iter().enumerate() {
            if global.storage == BytecodeStorage::FunctionStatic
                && let Some(owner) = global.owner
            {
                function_static_indices
                    .entry(owner)
                    .or_default()
                    .push(index);
            } else if global.storage == BytecodeStorage::FunctionPersistent
                && let Some(owner) = global.owner
                && let Some(owner_name) = function_names_by_key.get(&owner)
                && let Some(function_keys) = function_keys_by_name.get(owner_name)
            {
                // LOCAL/LOCALS/ARG/ARGS persist per normalized Era function name.
                // Duplicate event handlers therefore share these cells even though a
                // serialized global can name only one function key as its owner.
                for function in function_keys {
                    function_static_indices
                        .entry(*function)
                        .or_default()
                        .push(index);
                }
            } else if global.storage == BytecodeStorage::FunctionLocal
                && let Some(owner) = global.owner
            {
                function_local_indices.entry(owner).or_default().push(index);
            }
        }
        // Resolve serialized source-map precedence once per generation. Filling only empty
        // instruction slots preserves `SourceMap::resolve`'s first-matching-entry behavior.
        // A u32 sentinel is sufficient for the validator's source-map limit and is one quarter
        // the size of `Option<usize>` on 64-bit targets. Offsets are built one function at a time
        // so startup does not retain another project-wide index beside the permanent projection.
        let mut source_entries = BTreeMap::<SymbolKey, Vec<(u32, &SourceMapEntry)>>::new();
        for (index, entry) in artifact.source_map.entries.iter().enumerate() {
            source_entries.entry(entry.function).or_default().push((
                u32::try_from(index).expect("validated source-map index fits u32"),
                entry,
            ));
        }
        let instruction_source_indices = artifact
            .functions
            .iter()
            .map(|function| {
                let mut offset = 0_u64;
                let offsets = function
                    .code
                    .iter()
                    .map(|instruction| {
                        let current = offset;
                        offset = offset.saturating_add(instruction.encoded_len());
                        current
                    })
                    .collect::<Vec<_>>();
                let indices = index_source_entries(
                    &offsets,
                    source_entries.get(&function.key).map_or(&[], Vec::as_slice),
                );
                (function.key, indices)
            })
            .collect();
        Self {
            artifact,
            function_indices,
            function_name_indices,
            global_indices,
            global_name_indices,
            target_global_index,
            native_import_indices,
            host_import_indices,
            normalized_native_names,
            function_static_indices,
            function_local_indices,
            instruction_source_indices,
        }
    }

    pub(crate) fn function(&self, key: SymbolKey) -> Option<&BytecodeFunction> {
        self.function_index(key)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn function_index(&self, key: SymbolKey) -> Option<&usize> {
        self.function_indices.get(&key)
    }

    pub(crate) fn function_by_name(&self, name: &str) -> Option<&BytecodeFunction> {
        case_insensitive_index(&self.function_name_indices, name)
            .and_then(|index| self.artifact.functions.get(*index))
    }

    pub(crate) fn global(&self, key: SymbolKey) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.global_index(key)
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn global_index(&self, key: SymbolKey) -> Option<&usize> {
        self.global_indices.get(&key)
    }

    pub(crate) fn global_by_name(&self, name: &str) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        case_insensitive_index(&self.global_name_indices, name)
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn target_global(&self) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.target_global_index
            .and_then(|index| self.artifact.globals.get(index))
    }

    pub(crate) fn native_import_index(&self, key: SymbolKey) -> Option<usize> {
        self.native_import_indices.get(&key).copied()
    }

    pub(crate) fn host_import_index(&self, key: SymbolKey) -> Option<usize> {
        self.host_import_indices.get(&key).copied()
    }

    pub(crate) fn normalized_native_name(&self, index: usize) -> Option<Arc<str>> {
        self.normalized_native_names.get(index).cloned()
    }

    pub(crate) fn function_statics(
        &self,
        function: SymbolKey,
    ) -> impl Iterator<Item = &erabasic_bytecode::BytecodeGlobal> {
        self.function_static_indices
            .get(&function)
            .into_iter()
            .flatten()
            .filter_map(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn function_locals(
        &self,
        function: SymbolKey,
    ) -> impl Iterator<Item = &erabasic_bytecode::BytecodeGlobal> {
        self.function_local_indices
            .get(&function)
            .into_iter()
            .flatten()
            .filter_map(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn source_location(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<erabasic_bytecode::ResolvedSourceLocation> {
        let entry = self
            .instruction_source_indices
            .get(&function)?
            .get(instruction)
            .copied()
            .filter(|index| *index != NO_SOURCE_MAP_ENTRY)
            .and_then(|index| self.artifact.source_map.entries.get(index as usize))?;
        self.artifact.source_map.resolve_entry(entry)
    }
}

fn index_source_entries(offsets: &[u64], entries: &[(u32, &SourceMapEntry)]) -> Vec<u32> {
    let mut indices = vec![NO_SOURCE_MAP_ENTRY; offsets.len()];
    for (index, entry) in entries {
        let start = offsets.partition_point(|offset| *offset < entry.code_start);
        let end = offsets.partition_point(|offset| *offset < entry.code_end);
        for slot in &mut indices[start..end] {
            if *slot == NO_SOURCE_MAP_ENTRY {
                *slot = *index;
            }
        }
    }
    indices
}

fn case_insensitive_index<'a>(
    indices: &'a HashMap<String, usize>,
    name: &str,
) -> Option<&'a usize> {
    if name.bytes().any(|byte| byte.is_ascii_lowercase()) {
        indices.get(&name.to_ascii_uppercase())
    } else {
        indices.get(name)
    }
}

#[cfg(test)]
mod program_index_tests {
    use super::*;

    #[test]
    fn name_index_preserves_ascii_case_insensitive_lookup() {
        let indices = HashMap::from([("MIXED_NAME".to_owned(), 7)]);
        assert_eq!(case_insensitive_index(&indices, "MIXED_NAME"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "mixed_name"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "Mixed_Name"), Some(&7));
        assert_eq!(case_insensitive_index(&indices, "missing"), None);
    }

    #[test]
    fn compact_source_index_preserves_first_matching_entry() {
        let function = SymbolKey::derive("source-index-test", b"function");
        let broad = SourceMapEntry {
            function,
            code_start: 2,
            code_end: 10,
            byte_start: 1,
            byte_end: 2,
            statement_fingerprint: 0,
            origin_chain: None,
            source_index: 0,
        };
        let overlapping = SourceMapEntry {
            code_start: 5,
            code_end: 7,
            ..broad.clone()
        };
        let indices = index_source_entries(&[0, 2, 5, 7, 10], &[(3, &broad), (4, &overlapping)]);

        assert_eq!(indices, [NO_SOURCE_MAP_ENTRY, 3, 3, 3, NO_SOURCE_MAP_ENTRY]);
        assert_eq!(std::mem::size_of_val(&indices[0]), 4);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub id: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: usize,
    pub stack: Vec<VmValue>,
    pub for_loops: Vec<ForLoopState>,
    pub select_values: Vec<VmValue>,
    pub locals: BTreeMap<SymbolKey, VariableCell>,
    /// Dynamic statement calls discard method results without exposing them to Host code.
    pub return_value_to_caller: bool,
    /// True for an event handler and every ordinary function called beneath it.
    pub event_context: bool,
    /// Nested CALLEVENT handlers are sequenced in the initiating caller frame.
    pub event_dispatch: Option<EventDispatch>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ForLoopState {
    pub counter: PlaceDescriptor,
    pub end: i64,
    pub step: i64,
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
    pub(crate) debug: DebugState,
    pub(crate) regex_cache: RegexCache,
}

mod frames;
mod lifecycle;
mod places;
mod runtime;

pub(crate) use frames::{
    bind_persistent_arguments, make_frame, prepare_dynamic_arguments, validate_arguments,
};
use frames::{find_frame, find_frame_mut, find_global};
use runtime::replace_cell_values;
