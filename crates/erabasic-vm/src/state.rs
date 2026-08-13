use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::debug::DebugState;
use crate::regex_compat::RegexCache;
use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostReady, HostRequestId, Memory, PlaceDescriptor,
    VariableCell, VariableMap, VmConfig, VmError, VmValue,
};
use crate::{
    PreparedRuntimeState, VmRuntimeFill, VmRuntimeRead, VmRuntimeStatePort,
    VmRuntimeStateTransaction,
};
use erabasic_bytecode::{
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeFunctionKind, BytecodeStorage,
    BytecodeType, Digest, ImportKind, Opcode, SourceMapEntry, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;

mod planning;

use planning::{
    build_function_memo_plans, case_insensitive_index, index_source_entries, literal_group_match,
    memoized_indexed_read, simple_bulk_fill_loop,
};

#[derive(Clone, Debug)]
pub(crate) struct ProgramGeneration {
    pub artifact: Arc<BytecodeArtifact>,
    function_indices: HashMap<SymbolKey, usize>,
    function_name_indices: HashMap<String, usize>,
    // This map is lookup-only; authoritative globals remain canonically ordered
    // in the artifact, so hash iteration can never affect serialized output.
    global_indices: HashMap<SymbolKey, usize>,
    variable_global_indices: Vec<Vec<Option<usize>>>,
    bulk_fill_loop_plans: Vec<Vec<Option<BulkFillLoopPlan>>>,
    literal_group_match_plans: Vec<Vec<Option<LiteralGroupMatchPlan>>>,
    function_memo_plans: Vec<Option<FunctionMemoPlan>>,
    memoized_indexed_read_plans: Vec<Option<MemoizedIndexedReadPlan>>,
    // Canonical owner-free definitions always win system-name lookup.
    global_name_indices: HashMap<String, usize>,
    // Runtime inspection historically exposes otherwise unique function variables.
    runtime_name_fallback_indices: HashMap<String, usize>,
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
        let global_indices: HashMap<SymbolKey, usize> = artifact
            .globals
            .iter()
            .enumerate()
            .map(|(index, global)| (global.key, index))
            .collect();
        let variable_global_indices: Vec<Vec<Option<usize>>> = artifact
            .functions
            .iter()
            .map(|function| {
                function
                    .code
                    .iter()
                    .map(|instruction| {
                        matches!(
                            Opcode::try_from(instruction.opcode),
                            Ok(Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace)
                        )
                        .then(|| {
                            instruction
                                .payload
                                .get(..16)?
                                .try_into()
                                .ok()
                                .map(SymbolKey)
                                .and_then(|key| global_indices.get(&key).copied())
                        })
                        .flatten()
                    })
                    .collect()
            })
            .collect();
        let mut runtime_name_fallback_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            runtime_name_fallback_indices
                .entry(global.name.to_ascii_uppercase())
                .or_insert(index);
        }
        let mut global_name_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            if global.owner.is_none() {
                global_name_indices.insert(global.name.to_ascii_uppercase(), index);
            }
        }
        let target_global_index = global_name_indices.get("TARGET").copied();
        let mut native_import_indices = HashMap::new();
        for (index, import) in artifact.native_imports.iter().enumerate() {
            native_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let normalized_native_names: Vec<Arc<str>> = artifact
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
        let bulk_fill_loop_plans = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(function_index, function)| {
                (0..function.code.len())
                    .map(|instruction| {
                        simple_bulk_fill_loop(
                            &artifact,
                            function_index,
                            instruction,
                            &variable_global_indices,
                        )
                    })
                    .collect()
            })
            .collect();
        let literal_group_match_plans = artifact
            .functions
            .iter()
            .map(|function| {
                (0..function.code.len())
                    .map(|instruction| {
                        literal_group_match(
                            &artifact,
                            function,
                            instruction,
                            &native_import_indices,
                            &normalized_native_names,
                        )
                    })
                    .collect()
            })
            .collect();
        let function_memo_plans = build_function_memo_plans(
            &artifact,
            &variable_global_indices,
            &native_import_indices,
            &host_import_indices,
            &normalized_native_names,
        );
        let memoized_indexed_read_plans = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(function_index, function)| {
                memoized_indexed_read(
                    &artifact,
                    function_index,
                    function,
                    &variable_global_indices,
                    &function_indices,
                    &function_memo_plans,
                )
            })
            .collect();
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
            variable_global_indices,
            bulk_fill_loop_plans,
            literal_group_match_plans,
            function_memo_plans,
            memoized_indexed_read_plans,
            global_name_indices,
            runtime_name_fallback_indices,
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

    pub(crate) fn instruction_global(
        &self,
        function_index: usize,
        instruction: usize,
    ) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.variable_global_indices
            .get(function_index)?
            .get(instruction)
            .copied()
            .flatten()
            .and_then(|index| self.artifact.globals.get(index))
    }

    pub(crate) fn function_memo_plan(&self, function: SymbolKey) -> Option<&FunctionMemoPlan> {
        let index = *self.function_index(function)?;
        self.function_memo_plans.get(index)?.as_ref()
    }

    pub(crate) fn memoized_indexed_read_plan(
        &self,
        function: SymbolKey,
    ) -> Option<&MemoizedIndexedReadPlan> {
        let index = *self.function_index(function)?;
        self.memoized_indexed_read_plans.get(index)?.as_ref()
    }

    pub(crate) fn bulk_fill_loop_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&BulkFillLoopPlan> {
        let index = *self.function_index(function)?;
        self.bulk_fill_loop_plans
            .get(index)?
            .get(instruction)?
            .as_ref()
    }

    pub(crate) fn literal_group_match_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&LiteralGroupMatchPlan> {
        let index = *self.function_index(function)?;
        self.literal_group_match_plans
            .get(index)?
            .get(instruction)?
            .as_ref()
    }

    pub(crate) fn global_by_name(&self, name: &str) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        case_insensitive_index(&self.global_name_indices, name)
            .or_else(|| case_insensitive_index(&self.runtime_name_fallback_indices, name))
            .and_then(|index| self.artifact.globals.get(*index))
    }

    pub(crate) fn scoped_variable(
        &self,
        function: SymbolKey,
        name: &str,
    ) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.function_locals(function)
            .chain(self.function_statics(function))
            .chain(
                self.artifact
                    .globals
                    .iter()
                    .filter(|global| global.owner.is_none()),
            )
            .find(|global| global.name.eq_ignore_ascii_case(name))
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

mod frames;
mod lifecycle;
mod places;
mod runtime;
mod runtime_types;

pub use runtime_types::Vm;
pub(crate) use runtime_types::{
    BulkFillLoopPlan, EventDispatch, EventDispatchEntry, Fiber, FiberState, FindElementCacheKey,
    FindElementNeedle, ForLoopState, Frame, FunctionMemoEntry, FunctionMemoKey, FunctionMemoPlan,
    LiteralGroupMatchPlan, MemoValue, MemoizedIndexedReadPlan, WaitingHost,
};

pub(crate) use frames::{
    bind_persistent_arguments, make_frame, prepare_dynamic_arguments, validate_arguments,
};
use frames::{find_frame, find_frame_mut, find_global};
use runtime::replace_cell_values;
