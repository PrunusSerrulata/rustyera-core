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
    BytecodeArtifact, BytecodeConstant, BytecodeFunction, BytecodeFunctionKind, BytecodeGlobal,
    BytecodeStorage, BytecodeType, Digest, ImportKind, Opcode, SourceMapEntry, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;

mod derived_cache;
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

impl ProgramGeneration {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn new(artifact: Arc<BytecodeArtifact>) -> Self {
        Self::new_with_progress(artifact, None)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn new_with_progress(
        artifact: Arc<BytecodeArtifact>,
        mut progress: Option<&mut dyn FnMut(VmPreparationProgress)>,
    ) -> Self {
        let function_work = u64::try_from(artifact.functions.len()).unwrap_or(u64::MAX);
        let global_work = u64::try_from(artifact.globals.len()).unwrap_or(u64::MAX);
        let total_work = function_work
            .saturating_mul(10)
            .saturating_add(global_work.saturating_mul(4))
            .max(1);
        let mut completed_work = 0;
        let mut next_checkpoint = 1;
        report_vm_preparation(
            &mut progress,
            VmPreparationStage::IndexingProgram,
            0,
            total_work,
        );
        // Era projects commonly contain tens of thousands of functions. Resolving the
        // active function with a linear scan for every instruction makes otherwise
        // lightweight EraBasic execution quadratic in the project size.
        let mut function_indices = SymbolMap::with_capacity_and_hasher(
            artifact.functions.len(),
            BuildHasherDefault::default(),
        );
        for (index, function) in artifact.functions.iter().enumerate() {
            function_indices.insert(function.key, index);
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut function_name_indices = HashMap::new();
        for (index, function) in artifact.functions.iter().enumerate() {
            // Dynamic lookup follows the artifact order when duplicate declarations
            // are permitted by the selected compatibility mode.
            function_name_indices
                .entry(function.name.to_ascii_uppercase())
                .or_insert(index);
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut global_indices = SymbolMap::with_capacity_and_hasher(
            artifact.globals.len(),
            BuildHasherDefault::default(),
        );
        for (index, global) in artifact.globals.iter().enumerate() {
            global_indices.insert(global.key, index);
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut variable_global_indices = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            variable_global_indices.push(
                function
                    .code
                    .iter()
                    .map(|instruction| {
                        if matches!(
                            Opcode::try_from(instruction.opcode),
                            Ok(Opcode::LoadVariable | Opcode::StoreVariable | Opcode::MakePlace)
                        ) {
                            instruction
                                .payload
                                .get(..16)
                                .and_then(|bytes| bytes.try_into().ok())
                                .map(SymbolKey)
                                .and_then(|key| global_indices.get(&key).copied())
                                .and_then(|index| u32::try_from(index).ok())
                                .unwrap_or(NO_GLOBAL_INDEX)
                        } else {
                            NO_GLOBAL_INDEX
                        }
                    })
                    .collect(),
            );
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut runtime_name_fallback_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            runtime_name_fallback_indices
                .entry(global.name.to_ascii_uppercase())
                .or_insert(index);
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut global_name_indices = HashMap::new();
        for (index, global) in artifact.globals.iter().enumerate() {
            if global.owner.is_none() {
                global_name_indices.insert(global.name.to_ascii_uppercase(), index);
            }
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let target_global_index = global_name_indices.get("TARGET").copied();
        let mut native_import_indices = SymbolMap::default();
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
        let mut host_import_indices = SymbolMap::default();
        for (index, import) in artifact.host_imports.iter().enumerate() {
            host_import_indices
                .entry(import.import.key)
                .or_insert(index);
        }
        let normalized_host_names = artifact
            .host_imports
            .iter()
            .map(|import| Arc::<str>::from(import.import.name.to_ascii_uppercase()))
            .collect();
        let mut bulk_fill_loop_plans = Vec::with_capacity(artifact.functions.len());
        for (function_index, function) in artifact.functions.iter().enumerate() {
            bulk_fill_loop_plans.push(
                (0..function.code.len())
                    .filter_map(|instruction| {
                        simple_bulk_fill_loop(
                            &artifact,
                            function_index,
                            instruction,
                            &variable_global_indices,
                        )
                        .zip(u32::try_from(instruction).ok())
                        .map(|(plan, index)| (index, plan))
                    })
                    .collect(),
            );
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut literal_group_match_plans = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            literal_group_match_plans.push(
                (0..function.code.len())
                    .filter_map(|instruction| {
                        literal_group_match(
                            &artifact,
                            function,
                            instruction,
                            &native_import_indices,
                            &normalized_native_names,
                        )
                        .zip(u32::try_from(instruction).ok())
                        .map(|(plan, index)| (index, plan))
                    })
                    .collect(),
            );
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut structured_ranges = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            structured_ranges.push(structured_scope_ranges(function));
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let function_memo_plans = build_function_memo_plans(
            &artifact,
            &variable_global_indices,
            &native_import_indices,
            &host_import_indices,
            &normalized_native_names,
            || {
                advance_vm_preparation(
                    &mut progress,
                    &mut completed_work,
                    total_work,
                    &mut next_checkpoint,
                );
            },
        );
        let mut memoized_indexed_read_plans = Vec::with_capacity(artifact.functions.len());
        let mut path_memo_result_read_plans = Vec::with_capacity(artifact.functions.len());
        for (function_index, function) in artifact.functions.iter().enumerate() {
            memoized_indexed_read_plans.push(memoized_indexed_read(
                &artifact,
                function_index,
                function,
                &variable_global_indices,
                &function_indices,
                &function_memo_plans,
            ));
            path_memo_result_read_plans.push(path_memo_result_reads(
                &artifact,
                function_index,
                function,
                &variable_global_indices,
            ));
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        let mut function_static_indices = SymbolMap::<Vec<usize>>::default();
        let mut function_local_indices = SymbolMap::<Vec<usize>>::default();
        let mut function_names_by_key = BTreeMap::<SymbolKey, String>::new();
        let mut function_keys_by_name = BTreeMap::<String, Vec<SymbolKey>>::new();
        for function in &artifact.functions {
            let normalized = function.name.to_ascii_uppercase();
            function_names_by_key.insert(function.key, normalized.clone());
            function_keys_by_name
                .entry(normalized)
                .or_default()
                .push(function.key);
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
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
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        drop(function_names_by_key);
        drop(function_keys_by_name);
        // Validated artifacts store source entries in canonical function order. Consume each
        // contiguous function range directly so startup never retains a project-wide vector of
        // entry references beside the permanent projection. Filling only empty instruction slots
        // preserves `SourceMap::resolve`'s first match. A u32 sentinel is sufficient for the
        // validator's source-map limit and is one quarter the size of `Option<usize>` on 64-bit.
        let source_entries = &artifact.source_map.entries;
        let mut source_cursor = 0;
        let mut instruction_source_indices = Vec::with_capacity(artifact.functions.len());
        for function in &artifact.functions {
            let source_start = source_cursor;
            while source_cursor < source_entries.len()
                && source_entries[source_cursor].function == function.key
            {
                source_cursor += 1;
            }
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
            instruction_source_indices.push(index_source_entries(
                &offsets,
                source_entries[source_start..source_cursor]
                    .iter()
                    .enumerate()
                    .map(|(offset, entry)| {
                        (
                            u32::try_from(source_start + offset)
                                .expect("validated source-map index fits u32"),
                            entry,
                        )
                    }),
            ));
            advance_vm_preparation(
                &mut progress,
                &mut completed_work,
                total_work,
                &mut next_checkpoint,
            );
        }
        debug_assert_eq!(source_cursor, source_entries.len());
        report_vm_preparation(
            &mut progress,
            VmPreparationStage::IndexingProgram,
            total_work,
            total_work,
        );
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
            path_memo_result_read_plans,
            global_name_indices,
            runtime_name_fallback_indices,
            target_global_index,
            native_import_indices,
            host_import_indices,
            normalized_native_names,
            normalized_host_names,
            function_static_indices,
            function_local_indices,
            instruction_source_indices,
            structured_scope_ranges: structured_ranges,
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

    pub(crate) fn structured_jump_transition(
        &self,
        function: SymbolKey,
        source: usize,
        target: usize,
    ) -> Option<StructuredJumpTransition> {
        let ranges = self
            .structured_scope_ranges
            .get(*self.function_index(function)?)?;
        let mut source_ranges = ranges
            .iter()
            .filter(|range| range.start <= source && source <= range.end);
        let mut target_ranges = ranges
            .iter()
            .filter(|range| range.start <= target && target <= range.end);
        let mut retain_loops = 0;
        let mut retain_selects = 0;
        let entered = loop {
            match (source_ranges.next(), target_ranges.next()) {
                (Some(left), Some(right))
                    if left.kind == right.kind && left.opener == right.opener =>
                {
                    match right.kind {
                        StructuredScopeKind::Loop => retain_loops += 1,
                        StructuredScopeKind::Select => retain_selects += 1,
                    }
                }
                (_, Some(first_entered)) => {
                    break std::iter::once(first_entered)
                        .chain(target_ranges)
                        .map(|range| range.kind)
                        .collect();
                }
                (_, None) => break Vec::new(),
            }
        };
        Some(StructuredJumpTransition {
            retain_loops,
            retain_selects,
            entered,
        })
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
            .filter(|index| *index != NO_GLOBAL_INDEX)
            .map(|index| index as usize)
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

    pub(crate) fn path_memo_result_read_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&PathMemoResultReadPlan> {
        let index = *self.function_index(function)?;
        self.path_memo_result_read_plans
            .get(index)?
            .iter()
            .find(|plan| plan.instruction == instruction)
    }

    pub(crate) fn bulk_fill_loop_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&BulkFillLoopPlan> {
        let index = *self.function_index(function)?;
        sparse_instruction_plan(self.bulk_fill_loop_plans.get(index)?, instruction)
    }

    pub(crate) fn literal_group_match_plan(
        &self,
        function: SymbolKey,
        instruction: usize,
    ) -> Option<&LiteralGroupMatchPlan> {
        let index = *self.function_index(function)?;
        sparse_instruction_plan(self.literal_group_match_plans.get(index)?, instruction)
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

    pub(crate) fn normalized_native_name(&self, index: usize) -> Option<&str> {
        self.normalized_native_names.get(index).map(AsRef::as_ref)
    }

    pub(crate) fn normalized_host_name(&self, index: usize) -> Option<&str> {
        self.normalized_host_names.get(index).map(AsRef::as_ref)
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
        let function_index = *self.function_index(function)?;
        let entry = self
            .instruction_source_indices
            .get(function_index)?
            .get(instruction)
            .copied()
            .filter(|index| *index != NO_SOURCE_MAP_ENTRY)
            .and_then(|index| self.artifact.source_map.entries.get(index as usize))?;
        self.artifact.source_map.resolve_entry(entry)
    }
}

fn report_vm_preparation(
    progress: &mut Option<&mut dyn FnMut(VmPreparationProgress)>,
    stage: VmPreparationStage,
    completed: u64,
    total: u64,
) {
    if let Some(progress) = progress {
        progress(VmPreparationProgress {
            stage,
            completed,
            total,
        });
    }
}

fn advance_vm_preparation(
    progress: &mut Option<&mut dyn FnMut(VmPreparationProgress)>,
    completed: &mut u64,
    total: u64,
    next_checkpoint: &mut u64,
) {
    *completed = completed.saturating_add(1).min(total);
    let checkpoint = completed.saturating_mul(100) / total.max(1);
    if checkpoint >= *next_checkpoint || *completed == total {
        report_vm_preparation(
            progress,
            VmPreparationStage::IndexingProgram,
            *completed,
            total,
        );
        *next_checkpoint = checkpoint.saturating_add(1);
    }
}

fn sparse_instruction_plan<T>(plans: &[(u32, T)], instruction: usize) -> Option<&T> {
    let instruction = u32::try_from(instruction).ok()?;
    let index = plans
        .binary_search_by_key(&instruction, |(instruction, _)| *instruction)
        .ok()?;
    Some(&plans[index].1)
}

#[cfg(test)]
mod compact_generation_index_tests {
    use super::*;
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

    fn compiled_generation_fixture() -> BytecodeArtifact {
        let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .expect("default project data");
        let analysis = analyze_project(
            AnalysisInput {
                project_data,
                sources: vec![ProjectSource {
                    relative_path: "main.erb".into(),
                    payload: SourcePayload::Utf8(
                        "@SYSTEM_TITLE\n#DIMS VALUE\nVALUE '= \"keep\"\n\
                         RESULT = GROUPMATCH(VALUE, \"keep\", \"other\", \"keep\")\n\
                         CALL CLEAR_ROW(2, 0)\nRETURN\n\
                         @CLEAR_ROW(ARG, VALUE)\n#DIM VALUE\n#LOCALSIZE 1\n\
                         FOR LOCAL, 0, 4\nDA:ARG:LOCAL = 0\nNEXT\nRETURN\n"
                            .into(),
                    ),
                }],
            },
            &AnalyzerOptions::default(),
            &ExtensionRegistry::default(),
        );
        assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
        let compile = compile_project(
            analysis.project.as_ref().expect("analyzed project"),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
        );
        assert!(compile.artifact.is_some(), "{:#?}", compile.diagnostics);
        compile.artifact.expect("compiled artifact")
    }

    #[test]
    fn sparse_instruction_lookup_preserves_exact_instruction_identity() {
        let plans = [(2, "first"), (19, "second")];
        assert_eq!(sparse_instruction_plan(&plans, 2), Some(&"first"));
        assert_eq!(sparse_instruction_plan(&plans, 19), Some(&"second"));
        assert_eq!(sparse_instruction_plan(&plans, 18), None);
        assert_eq!(sparse_instruction_plan(&plans, usize::MAX), None);
    }

    #[test]
    fn compact_instruction_indices_use_four_bytes_per_slot() {
        assert_eq!(std::mem::size_of::<u32>(), 4);
        assert!(std::mem::size_of::<u32>() < std::mem::size_of::<Option<usize>>());
    }

    #[test]
    fn real_generation_preserves_sparse_fastpaths_globals_sources_and_progress() {
        let artifact = Arc::new(compiled_generation_fixture());
        let mut progress = Vec::new();
        let generation = ProgramGeneration::new_with_progress(
            Arc::clone(&artifact),
            Some(&mut |event| progress.push(event)),
        );

        assert!(
            generation
                .bulk_fill_loop_plans
                .iter()
                .any(|plans| !plans.is_empty())
        );
        assert!(
            generation
                .literal_group_match_plans
                .iter()
                .any(|plans| !plans.is_empty())
        );
        let mut saw_global = false;
        let mut saw_sentinel = false;
        for (function_index, function) in artifact.functions.iter().enumerate() {
            let mut code_offset = 0;
            for (instruction_index, instruction) in function.code.iter().enumerate() {
                let expected_source = artifact.source_map.resolve(function.key, code_offset);
                assert_eq!(
                    generation.source_location(function.key, instruction_index),
                    expected_source
                );
                code_offset = code_offset.saturating_add(instruction.encoded_len());

                if generation
                    .instruction_global(function_index, instruction_index)
                    .is_some()
                {
                    saw_global = true;
                } else {
                    saw_sentinel = true;
                }
            }
        }
        assert!(saw_global);
        assert!(saw_sentinel);
        assert_eq!(progress.first().map(|event| event.completed), Some(0));
        assert_eq!(
            progress.last().map(|event| (event.completed, event.total)),
            progress.last().map(|event| (event.total, event.total))
        );
        assert!(progress.windows(2).all(|events| {
            events[0].stage == VmPreparationStage::IndexingProgram
                && events[1].stage == VmPreparationStage::IndexingProgram
                && events[0].completed <= events[1].completed
                && events[0].total == events[1].total
        }));
    }
}

mod frames;
mod lifecycle;
mod places;
mod runtime;
mod runtime_types;

pub use runtime_types::Vm;
pub(crate) use runtime_types::{
    ActivePathMemo, BulkFillLoopPlan, EventDispatch, EventDispatchEntry, Fiber, FiberState,
    FindElementCacheKey, FindElementNeedle, ForLoopState, Frame, FunctionMemoEntry,
    FunctionMemoKey, FunctionMemoPlan, LiteralGroupMatchPlan, MemoValue, MemoizedIndexedReadPlan,
    PathMemoBaseKey, PathMemoCache, PathMemoDependency, PathMemoEntry, PathMemoHead,
    PathMemoMutation, PathMemoMutationGroup, PathMemoPlace, PathMemoResultReadPlan, WaitingHost,
};

pub(crate) use frames::{
    PersistentArgumentDestination, bind_persistent_arguments, make_frame,
    persistent_argument_destination, prepare_dynamic_arguments, validate_arguments,
};
use frames::{find_frame, find_frame_mut, find_global};
use runtime::replace_cell_values;
