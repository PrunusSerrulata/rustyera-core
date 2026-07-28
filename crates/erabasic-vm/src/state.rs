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

impl Vm {
    /// Resolve the active shape of a variable name for runtime introspection.
    /// Function-local reference variables report the dimensions of their bound
    /// place rather than the zero-length placeholder stored in bytecode.
    #[must_use]
    pub fn variable_dimensions(&self, fiber: FiberId, name: &str) -> Option<Vec<u64>> {
        let fiber = self.fibers.get(&fiber)?;
        let frame = fiber.frames.last()?;
        let program = self.generations.get(&frame.generation)?;
        if let Some(definition) = program
            .function_locals(frame.function)
            .find(|definition| definition.name.eq_ignore_ascii_case(name))
        {
            let cell = frame.locals.get(&definition.key)?;
            if let Some(VmValue::IntegerPlace(place) | VmValue::StringPlace(place)) = cell.first() {
                return self.place_dimensions(fiber, &place).ok();
            }
            return Some(cell.dimensions.clone());
        }
        if let Some(definition) = program
            .function_statics(frame.function)
            .find(|definition| definition.name.eq_ignore_ascii_case(name))
        {
            return Some(definition.dimensions.clone());
        }
        program
            .artifact
            .globals
            .iter()
            .find(|definition| {
                definition.owner.is_none() && definition.name.eq_ignore_ascii_case(name)
            })
            .map(|definition| definition.dimensions.clone())
    }

    /// Resolve the complete shape of a place supplied by the active Host call.
    ///
    /// # Errors
    ///
    /// The place must belong to the requesting fiber and still refer to an
    /// active variable or bound reference.
    pub fn host_place_dimensions(
        &self,
        fiber: FiberId,
        place: &PlaceDescriptor,
    ) -> Result<Vec<u64>, VmError> {
        let fiber = self
            .fibers
            .get(&fiber)
            .ok_or_else(|| VmError::InvalidState("Host place fiber is missing".into()))?;
        self.place_dimensions(fiber, place)
    }

    fn place_dimensions(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<Vec<u64>, VmError> {
        let mut place = place.clone();
        for _ in 0..64 {
            if place.fiber.is_some_and(|owner| owner != fiber.id) {
                return Err(VmError::InvalidState(
                    "place belongs to another fiber".into(),
                ));
            }
            let (_, definition) = self.place_definition(fiber, &place)?;
            if definition.storage != BytecodeStorage::FunctionLocal {
                return Ok(definition.dimensions.clone());
            }
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            match cell.first() {
                Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) => {
                    place = *bound;
                }
                _ => return Ok(cell.dimensions.clone()),
            }
        }
        Err(VmError::InvalidState(
            "variable reference chain is too deep".into(),
        ))
    }

    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        Self::new_with_memory(artifact, config, false)
    }

    #[must_use]
    pub(crate) fn new_for_title(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        Self::new_with_memory(artifact, config, true)
    }

    fn new_with_memory(artifact: ValidatedArtifact, config: VmConfig, title_state: bool) -> Self {
        let artifact = artifact.into_shared();
        let memory = if title_state {
            Memory::title(&artifact)
        } else {
            Memory::new_game(&artifact)
        };
        let generation = GenerationId(1);
        let program = Arc::new(ProgramGeneration::new(artifact));
        Self {
            config,
            generations: BTreeMap::from([(generation, program)]),
            current_generation: generation,
            memory,
            fibers: BTreeMap::new(),
            runnable: VecDeque::new(),
            primary_fiber: None,
            next_fiber: 1,
            next_frame: 1,
            next_request: 1,
            next_generation: 2,
            pending_reload: None,
            debug: DebugState::default(),
            regex_cache: RegexCache::default(),
        }
    }

    pub(crate) fn compile_regex(&mut self, pattern: &str) -> Result<regex::Regex, String> {
        self.regex_cache.get_or_compile(pattern)
    }

    #[must_use]
    pub fn config(&self) -> VmConfig {
        self.config
    }

    #[must_use]
    /// Return the current immutable program artifact.
    ///
    /// # Panics
    ///
    /// Panics only if an internal invariant has removed the current generation.
    pub fn artifact(&self) -> &BytecodeArtifact {
        &self
            .generations
            .get(&self.current_generation)
            .expect("the current generation is always retained")
            .artifact
    }

    pub(crate) fn set_runtime_calculated_string(&mut self, name: &str, value: &str) {
        let program = self
            .generations
            .get(&self.current_generation)
            .expect("the current generation is always retained");
        self.memory
            .set_runtime_calculated_string(&program.artifact, name, value);
    }

    pub(crate) fn target_character_for_generation(&self, generation: GenerationId) -> usize {
        self.generations.get(&generation).map_or(0, |program| {
            self.memory
                .target_character_from_definition(program.target_global(), generation)
        })
    }

    /// Resolve a current-generation global by its `EraBasic` case-insensitive name.
    #[must_use]
    pub fn global_by_name(&self, name: &str) -> Option<&erabasic_bytecode::BytecodeGlobal> {
        self.generations
            .get(&self.current_generation)?
            .global_by_name(name)
    }

    #[must_use]
    pub fn current_generation(&self) -> GenerationId {
        self.current_generation
    }

    #[must_use]
    pub fn artifact_id(&self) -> Digest {
        self.artifact().manifest.artifact_id
    }

    #[must_use]
    pub fn primary_fiber(&self) -> Option<FiberId> {
        self.primary_fiber
    }

    /// Spawned fibers always start in the current program generation. Calls made
    /// by an old frame continue resolving through that frame's pinned generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing entry, invalid arguments, or the fiber limit.
    pub fn spawn_entry(
        &mut self,
        function: SymbolKey,
        arguments: Vec<VmValue>,
    ) -> Result<FiberId, VmError> {
        if self.live_fiber_count() >= self.config.maximum_fibers {
            return Err(VmError::ResourceLimit("fiber count"));
        }
        let generation = self.current_generation;
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        let function_definition = program
            .function(function)
            .ok_or(VmError::MissingFunction(function))?;
        validate_arguments(function_definition, &arguments)?;
        self.memory.ensure_function_statics(
            generation,
            function_definition.key,
            program.function_statics(function_definition.key),
        );
        bind_persistent_arguments(
            &mut self.memory,
            generation,
            function_definition,
            program,
            &arguments,
        )?;
        let fiber_id = self.next_available_fiber_id();
        let frame_id = FrameId(self.next_frame);
        self.next_frame = self.next_frame.saturating_add(1);
        let frame = make_frame(
            frame_id,
            generation,
            function_definition,
            program.function_locals(function_definition.key),
            arguments,
            true,
            function_definition.kind == BytecodeFunctionKind::Event,
        );
        self.fibers.insert(
            fiber_id,
            Fiber {
                id: fiber_id,
                frames: vec![frame],
                state: FiberState::Runnable,
                backward_branches_without_progress: 0,
                consecutive_budget_exhaustions: 0,
            },
        );
        self.next_fiber = self.next_available_fiber_id().0;
        self.runnable.push_back(fiber_id);
        if self.primary_fiber.is_none() {
            self.primary_fiber = Some(fiber_id);
        }
        Ok(fiber_id)
    }

    /// Select the fiber that must be at stable input before snapshots are allowed.
    ///
    /// # Errors
    ///
    /// Returns an error if the fiber does not exist.
    pub fn set_primary_fiber(&mut self, fiber: FiberId) -> Result<(), VmError> {
        if !self.fibers.contains_key(&fiber) {
            return Err(VmError::UnknownFiber(fiber));
        }
        self.primary_fiber = Some(fiber);
        Ok(())
    }

    /// Cancel a fiber and invalidate any outstanding host request it owned.
    ///
    /// # Errors
    ///
    /// Returns an error if the fiber does not exist.
    pub fn cancel_fiber(&mut self, fiber: FiberId) -> Result<(), VmError> {
        let fiber = self
            .fibers
            .get_mut(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?;
        fiber.frames.clear();
        fiber.state = FiberState::Cancelled;
        Ok(())
    }

    /// Retire terminal fibers after their events have been consumed by the caller.
    ///
    /// A terminal fiber selected by the current debugger stop remains available until
    /// that stop is continued. Faulted fibers are retained for diagnosis.
    pub fn retire_terminal_fibers(&mut self) -> usize {
        let protected = self.debug_retained_terminal_fiber();
        let retired = self
            .fibers
            .iter()
            .filter_map(|(id, fiber)| {
                (Some(*id) != protected
                    && matches!(
                        fiber.state,
                        FiberState::Completed(_) | FiberState::Cancelled
                    ))
                .then_some(*id)
            })
            .collect::<BTreeSet<_>>();
        for id in &retired {
            self.fibers.remove(id);
        }
        self.runnable.retain(|id| !retired.contains(id));
        if self.primary_fiber.is_some_and(|id| retired.contains(&id)) {
            self.primary_fiber = None;
        }
        self.next_fiber = self.next_available_fiber_id().0;
        self.reclaim_generations();
        retired.len()
    }

    #[must_use]
    pub fn fiber_status(&self, fiber: FiberId) -> Option<FiberStatus> {
        self.fibers.get(&fiber).map(Fiber::public_status)
    }

    pub fn fiber_ids(&self) -> impl Iterator<Item = FiberId> + '_ {
        self.fibers.keys().copied()
    }

    /// Supply a completed result for one outstanding host request.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale request, result type mismatch, or invalid write.
    pub fn resume_host(
        &mut self,
        request: HostRequestId,
        result: HostReady,
    ) -> Result<FiberId, VmError> {
        let fiber_id = self
            .fibers
            .iter()
            .find_map(|(id, fiber)| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request => Some(*id),
                _ => None,
            })
            .ok_or(VmError::StaleHostRequest(request))?;
        let mut fiber = self
            .fibers
            .remove(&fiber_id)
            .ok_or_else(|| VmError::InvalidState("waiting fiber disappeared".into()))?;
        let expected = match &fiber.state {
            FiberState::WaitingHost(wait) => wait.result,
            _ => {
                return Err(VmError::InvalidState(
                    "host request no longer belongs to a waiting fiber".into(),
                ));
            }
        };
        self.apply_host_ready(&mut fiber, expected, result)?;
        fiber.mark_progress();
        fiber.state = FiberState::Runnable;
        self.fibers.insert(fiber_id, fiber);
        self.runnable.push_back(fiber_id);
        Ok(fiber_id)
    }

    pub(crate) fn return_current_from_host(
        &mut self,
        request: HostRequestId,
        value: Option<&VmValue>,
    ) -> Result<FiberId, VmError> {
        let fiber_id = self
            .fibers
            .iter()
            .find_map(|(id, fiber)| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request => Some(*id),
                _ => None,
            })
            .ok_or(VmError::StaleHostRequest(request))?;
        let mut fiber = self
            .fibers
            .remove(&fiber_id)
            .ok_or_else(|| VmError::InvalidState("waiting fiber disappeared".into()))?;
        if fiber.frames.len() <= 1 {
            self.fibers.insert(fiber_id, fiber);
            return Err(VmError::InvalidState(
                "cannot return the root frame through a host completion".into(),
            ));
        }
        let returned = fiber.frames.pop().expect("checked frame count");
        let caller = fiber.frames.last_mut().expect("checked caller frame");
        if returned.return_value_to_caller
            && let Some(value) = value
        {
            caller.stack.push(value.clone());
        }
        let next_event = caller.event_dispatch.as_mut().and_then(|dispatch| {
            if dispatch.active.single && value == Some(&VmValue::Integer(1)) {
                while dispatch
                    .pending
                    .front()
                    .is_some_and(|entry| entry.group == dispatch.active.group)
                {
                    dispatch.pending.pop_front();
                }
            }
            dispatch.pending.pop_front().inspect(|next| {
                dispatch.active = next.clone();
            })
        });
        if let Some(next) = next_event {
            let generation = caller.generation;
            let frame_id = self.allocate_frame_id();
            let program = self
                .generations
                .get(&generation)
                .ok_or_else(|| VmError::InvalidState("event generation is missing".into()))?;
            let target = program
                .function(next.function)
                .ok_or_else(|| VmError::InvalidState("event function is missing".into()))?;
            self.memory.ensure_function_statics(
                generation,
                target.key,
                program.function_statics(target.key),
            );
            fiber.frames.push(make_frame(
                frame_id,
                generation,
                target,
                program.function_locals(target.key),
                Vec::new(),
                false,
                true,
            ));
        } else if caller.event_dispatch.is_some() {
            caller.event_dispatch = None;
        }
        fiber.mark_progress();
        fiber.state = FiberState::Runnable;
        self.fibers.insert(fiber_id, fiber);
        self.runnable.push_back(fiber_id);
        Ok(fiber_id)
    }

    /// Resume an explicit `AwaitResume` instruction with its typed continuation value.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown/non-waiting fiber or a type mismatch.
    pub fn resume_fiber(&mut self, fiber: FiberId, value: VmValue) -> Result<(), VmError> {
        let target = self
            .fibers
            .get_mut(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?;
        let FiberState::WaitingResume(expected) = target.state else {
            return Err(VmError::InvalidState(
                "fiber is not awaiting a resume value".into(),
            ));
        };
        if value.value_type() != expected {
            return Err(VmError::InvalidArguments(format!(
                "resume expects {expected:?}, found {:?}",
                value.value_type()
            )));
        }
        target
            .frames
            .last_mut()
            .ok_or_else(|| VmError::InvalidState("waiting fiber has no frame".into()))?
            .stack
            .push(value);
        target.mark_progress();
        target.state = FiberState::Runnable;
        self.runnable.push_back(fiber);
        Ok(())
    }

    /// Read non-frame storage in the current generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing variable, frame-local storage, or invalid index.
    pub fn read_variable(
        &self,
        variable: SymbolKey,
        indices: &[u64],
        character: Option<u64>,
    ) -> Result<VmValue, VmError> {
        let program = self
            .generations
            .get(&self.current_generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        let definition = program.global(variable).ok_or_else(|| {
            VmError::InvalidState(format!("variable {variable:?} is not defined"))
        })?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            return Err(VmError::InvalidState(
                "frame-local variables require a place descriptor".into(),
            ));
        }
        let character = character.map_or_else(
            || self.target_character_for_generation(self.current_generation),
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell(self.current_generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .read(indices)
            .map_err(VmError::InvalidState)
    }

    /// Write mutable, non-frame storage in the current generation.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing or immutable variable, unavailable storage,
    /// type mismatch, or invalid index.
    pub fn write_variable(
        &mut self,
        variable: SymbolKey,
        indices: &[u64],
        character: Option<u64>,
        value: VmValue,
    ) -> Result<(), VmError> {
        let generation = self.current_generation;
        let definition = self
            .generations
            .get(&generation)
            .and_then(|program| program.global(variable))
            .cloned()
            .ok_or_else(|| {
                VmError::InvalidState(format!("variable {variable:?} is not defined"))
            })?;
        if !definition.mutable {
            return Err(VmError::InvalidState("variable is immutable".into()));
        }
        let character = character.map_or_else(
            || self.target_character_for_generation(generation),
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .write(indices, value)
            .map_err(VmError::InvalidState)
    }

    /// Validate and fill a batch of runtime-owned variables without cloning VM memory.
    ///
    /// # Errors
    ///
    /// Returns before the first mutation if any variable, character destination, or
    /// value type is invalid.
    ///
    /// # Panics
    ///
    /// Panics only if validated variable storage changes during the commit phase. The
    /// exclusive VM borrow prevents such a change through the public API.
    pub fn fill_runtime_variables(&mut self, fills: &[VmRuntimeFill]) -> Result<(), VmError> {
        struct FillPlan {
            global_index: usize,
            characters: Vec<usize>,
            value: VmValue,
        }

        let generation = self.current_generation;
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        let target_key = program.target_global().map(|definition| definition.key);
        let mut target_character = self.target_character_for_generation(generation);
        let mut plans = Vec::with_capacity(fills.len());
        for fill in fills {
            let global_index = *program.global_index(fill.variable).ok_or_else(|| {
                VmError::InvalidState(format!("variable {:?} is not defined", fill.variable))
            })?;
            let definition = &program.artifact.globals[global_index];
            if !definition.mutable || definition.storage == BytecodeStorage::FunctionLocal {
                return Err(VmError::InvalidState(
                    "runtime state transaction cannot fill this variable".into(),
                ));
            }
            if definition.value_type != fill.value.value_type() {
                return Err(VmError::InvalidArguments(
                    "runtime fill value type differs from its variable".into(),
                ));
            }
            let characters = if definition.storage == BytecodeStorage::Character {
                if fill.all_characters {
                    (0..self.memory.characters.len()).collect()
                } else {
                    vec![target_character]
                }
            } else {
                vec![0]
            };
            for character in &characters {
                let cell = self
                    .memory
                    .cell(generation, definition, *character)
                    .ok_or_else(|| {
                        VmError::InvalidState("variable storage is unavailable".into())
                    })?;
                if cell.value_type != fill.value.value_type() {
                    return Err(VmError::InvalidArguments(
                        "runtime fill value type differs from its variable".into(),
                    ));
                }
            }
            if Some(fill.variable) == target_key
                && let VmValue::Integer(value) = &fill.value
            {
                target_character = usize::try_from(*value).unwrap_or(0);
            }
            plans.push(FillPlan {
                global_index,
                characters,
                value: fill.value.clone(),
            });
        }

        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?;
        for plan in plans {
            let definition = &program.artifact.globals[plan.global_index];
            for character in plan.characters {
                self.memory
                    .cell_mut(generation, definition, character)
                    .expect("the complete fill batch was validated before mutation")
                    .fill(plan.value.clone())
                    .expect("the complete fill batch type was validated before mutation");
            }
        }
        Ok(())
    }

    pub(crate) fn allocate_frame_id(&mut self) -> FrameId {
        let id = FrameId(self.next_frame);
        self.next_frame = self.next_frame.saturating_add(1);
        id
    }

    pub(crate) fn allocate_request_id(&mut self) -> HostRequestId {
        let id = HostRequestId(self.next_request);
        self.next_request = self.next_request.saturating_add(1);
        id
    }

    fn next_available_fiber_id(&self) -> FiberId {
        let mut candidate = 1_u64;
        for id in self.fibers.keys() {
            if id.0 < candidate {
                continue;
            }
            if id.0 != candidate {
                break;
            }
            candidate = candidate
                .checked_add(1)
                .expect("the fiber map cannot contain every positive u64 id");
        }
        FiberId(candidate)
    }

    pub(crate) fn live_fiber_count(&self) -> usize {
        self.fibers
            .values()
            .filter(|fiber| {
                !matches!(
                    fiber.state,
                    FiberState::Completed(_) | FiberState::Cancelled | FiberState::Faulted(_)
                )
            })
            .count()
    }

    pub(crate) fn active_generations(&self) -> BTreeSet<GenerationId> {
        self.fibers
            .values()
            .flat_map(|fiber| fiber.frames.iter().map(|frame| frame.generation))
            .collect()
    }

    pub(crate) fn reclaim_generations(&mut self) {
        let active = self.active_generations();
        let obsolete: Vec<_> = self
            .generations
            .keys()
            .copied()
            .filter(|generation| {
                *generation != self.current_generation && !active.contains(generation)
            })
            .collect();
        for generation in obsolete {
            self.generations.remove(&generation);
            self.memory.reclaim_generation(generation);
        }
    }

    pub(crate) fn apply_host_ready(
        &mut self,
        fiber: &mut Fiber,
        expected: Option<BytecodeType>,
        ready: HostReady,
    ) -> Result<(), VmError> {
        match (expected, ready.value) {
            (None, None) => {}
            (Some(expected), Some(value)) if value.value_type() == expected => fiber
                .frames
                .last_mut()
                .ok_or_else(|| VmError::InvalidState("host fiber has no frame".into()))?
                .stack
                .push(value),
            (expected, value) => {
                return Err(VmError::InvalidArguments(format!(
                    "host result mismatch: expected {expected:?}, found {:?}",
                    value.as_ref().map(VmValue::value_type)
                )));
            }
        }
        for write in ready.writes {
            self.write_place_internal(fiber, &write.target, write.value, true)?;
        }
        Ok(())
    }

    pub(crate) fn read_place(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<VmValue, VmError> {
        let (generation, definition) = self.place_definition(fiber, place)?;
        self.read_place_resolved(fiber, place, generation, definition)
    }

    pub(crate) fn read_place_resolved(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
        generation: GenerationId,
        definition: &erabasic_bytecode::BytecodeGlobal,
    ) -> Result<VmValue, VmError> {
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                let mut target = *bound;
                target.indices.extend_from_slice(&place.indices);
                return self.read_place(fiber, &target);
            }
            return cell.read(&place.indices).map_err(VmError::InvalidState);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .read(&place.indices)
            .map_err(VmError::InvalidState)
    }

    pub(crate) fn read_place_array(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<Vec<VmValue>, VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                return self.read_place_array(fiber, &bound);
            }
            return Ok(cell.to_values());
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell(generation, definition, character)
            .map(VariableCell::to_values)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))
    }

    pub(crate) fn place_array_len(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<usize, VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            let cell = frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            if let Some(VmValue::IntegerPlace(bound) | VmValue::StringPlace(bound)) = cell.first() {
                return self.place_array_len(fiber, &bound);
            }
            return Ok(cell.len());
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell(generation, definition, character)
            .map(VariableCell::len)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))
    }

    pub(crate) fn fill_place_array_range(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        start: usize,
        end: usize,
        value: VmValue,
    ) -> Result<(), VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
        if !definition.mutable {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(bound) = bound {
                return self.fill_place_array_range(fiber, &bound, start, end, value);
            }
            return find_frame_mut(fiber, place.frame, definition.owner)?
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .fill_range(start, end, value)
                .map_err(VmError::InvalidArguments);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .fill_range(start, end, value)
            .map_err(VmError::InvalidArguments)
    }

    pub(crate) fn write_place(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
    ) -> Result<(), VmError> {
        self.write_place_internal(fiber, place, value, false)
    }

    pub(crate) fn write_place_array(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        values: Vec<VmValue>,
    ) -> Result<(), VmError> {
        if !place.indices.is_empty() {
            return Err(VmError::InvalidArguments(
                "array place must be unindexed".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
        if !definition.mutable {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(bound) = bound {
                return self.write_place_array(fiber, &bound, values);
            }
            let cell = find_frame_mut(fiber, place.frame, definition.owner)?
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?;
            return replace_cell_values(cell, values);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |value| usize::try_from(value).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        let cell = self
            .memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?;
        replace_cell_values(cell, values)
    }

    fn write_place_internal(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
        trusted_runtime: bool,
    ) -> Result<(), VmError> {
        if place.fiber.is_some_and(|owner| owner != fiber.id) {
            return Err(VmError::InvalidState(
                "place belongs to another fiber".into(),
            ));
        }
        let (generation, definition) = self.place_definition(fiber, place)?;
        let definition = definition.clone();
        if !definition.mutable && !trusted_runtime {
            return Err(VmError::InvalidState("place is immutable".into()));
        }
        if definition.storage == BytecodeStorage::FunctionLocal {
            let bound = find_frame(fiber, place.frame, definition.owner)?
                .locals
                .get(&definition.key)
                .and_then(VariableCell::first)
                .and_then(|value| match value {
                    VmValue::IntegerPlace(place) | VmValue::StringPlace(place) => Some(*place),
                    VmValue::Integer(_) | VmValue::String(_) => None,
                });
            if let Some(mut target) = bound {
                target.indices.extend_from_slice(&place.indices);
                return self.write_place_internal(fiber, &target, value, trusted_runtime);
            }
            let frame = find_frame_mut(fiber, place.frame, definition.owner)?;
            return frame
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .write(&place.indices, value)
                .map_err(VmError::InvalidState);
        }
        let character = if definition.storage == BytecodeStorage::Character {
            place.character.map_or_else(
                || self.target_character_for_generation(generation),
                |index| usize::try_from(index).unwrap_or(usize::MAX),
            )
        } else {
            0
        };
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .write(&place.indices, value)
            .map_err(VmError::InvalidState)
    }

    fn place_definition<'a>(
        &'a self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<(GenerationId, &'a erabasic_bytecode::BytecodeGlobal), VmError> {
        let generation = place
            .frame
            .and_then(|frame| {
                fiber
                    .frames
                    .iter()
                    .find(|candidate| candidate.id == frame)
                    .map(|frame| frame.generation)
            })
            .or_else(|| fiber.frames.last().map(|frame| frame.generation))
            .ok_or_else(|| VmError::InvalidState("place fiber has no frames".into()))?;
        let program = self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("place generation was reclaimed".into()))?;
        Ok((
            generation,
            program.global(place.variable).ok_or_else(|| {
                VmError::InvalidState(format!("variable {:?} is not defined", place.variable))
            })?,
        ))
    }
}

fn replace_cell_values(cell: &mut VariableCell, values: Vec<VmValue>) -> Result<(), VmError> {
    cell.replace_values(values)
        .map_err(VmError::InvalidArguments)
}

impl VmRuntimeStatePort for Vm {
    fn read_runtime_state(&self, reads: &[VmRuntimeRead]) -> Result<Vec<VmValue>, VmError> {
        reads
            .iter()
            .map(|read| self.read_variable(read.variable, &read.indices, read.character))
            .collect()
    }

    fn prepare_runtime_state(
        &self,
        transaction: VmRuntimeStateTransaction,
    ) -> Result<PreparedRuntimeState, VmError> {
        let artifact = self.artifact();
        let reset_execution = matches!(
            &transaction,
            VmRuntimeStateTransaction::ResetNewGame | VmRuntimeStateTransaction::RestoreOrdinary(_)
        );
        let mut memory = prepare_transaction_memory(artifact, &self.memory, &transaction)?;
        if let VmRuntimeStateTransaction::Mutate {
            writes,
            fills,
            clear_characters,
            add_characters_from_csv,
        } = transaction
        {
            if clear_characters {
                memory.characters.clear();
            }
            for csv_number in add_characters_from_csv {
                let template = artifact
                    .project_data
                    .static_data
                    .characters
                    .iter()
                    .find(|template| template.csv_no == csv_number)
                    .ok_or_else(|| {
                        VmError::InvalidArguments(format!(
                            "character CSV number {csv_number} does not exist"
                        ))
                    })?;
                memory.push_character(artifact, Some(template));
            }
            for fill in fills {
                let definition = find_global(artifact, fill.variable)?;
                if !definition.mutable || definition.storage == BytecodeStorage::FunctionLocal {
                    return Err(VmError::InvalidState(
                        "runtime state transaction cannot fill this variable".into(),
                    ));
                }
                if definition.value_type != fill.value.value_type() {
                    return Err(VmError::InvalidArguments(
                        "runtime fill value type differs from its variable".into(),
                    ));
                }
                let characters: Box<dyn Iterator<Item = usize>> =
                    if definition.storage == BytecodeStorage::Character && fill.all_characters {
                        Box::new(0..memory.characters.len())
                    } else {
                        Box::new(std::iter::once(
                            memory.target_character(artifact, self.current_generation),
                        ))
                    };
                for character in characters {
                    let cell = memory
                        .cell_mut(self.current_generation, definition, character)
                        .ok_or_else(|| {
                            VmError::InvalidState("variable storage is unavailable".into())
                        })?;
                    cell.fill(fill.value.clone())
                        .map_err(VmError::InvalidArguments)?;
                }
            }
            for write in writes {
                let definition = find_global(artifact, write.variable)?;
                if !definition.mutable || definition.storage == BytecodeStorage::FunctionLocal {
                    return Err(VmError::InvalidState(
                        "runtime state transaction cannot write this variable".into(),
                    ));
                }
                let character = write.character.map_or_else(
                    || memory.target_character(artifact, self.current_generation),
                    |value| usize::try_from(value).unwrap_or(usize::MAX),
                );
                memory
                    .cell_mut(self.current_generation, definition, character)
                    .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
                    .write(&write.indices, write.value)
                    .map_err(VmError::InvalidState)?;
            }
        }
        Ok(PreparedRuntimeState {
            generation: self.current_generation,
            memory,
            reset_execution,
            structured_state: None,
        })
    }

    fn commit_runtime_state(&mut self, prepared: PreparedRuntimeState) -> Result<(), VmError> {
        if prepared.generation != self.current_generation {
            return Err(VmError::InvalidState(
                "runtime state transaction belongs to a stale generation".into(),
            ));
        }
        self.memory = prepared.memory;
        if prepared.reset_execution {
            self.clear_execution();
        }
        Ok(())
    }
}

fn prepare_transaction_memory(
    artifact: &erabasic_bytecode::BytecodeArtifact,
    current: &Memory,
    transaction: &VmRuntimeStateTransaction,
) -> Result<Memory, VmError> {
    Ok(match transaction {
        VmRuntimeStateTransaction::ResetNewGame => {
            crate::save::prepare_new_game_memory(artifact, current)
        }
        VmRuntimeStateTransaction::ResetGameData => {
            crate::save::prepare_reset_game_memory(artifact, current)
        }
        VmRuntimeStateTransaction::ResetGlobalData => {
            crate::save::prepare_reset_global_memory(artifact, current)
        }
        VmRuntimeStateTransaction::RestoreOrdinary(state) => {
            crate::save::prepare_era_memory(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::OverlayGlobal(state) => {
            crate::save::prepare_global_memory(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::AppendCharacters(state) => {
            crate::save::prepare_appended_characters(artifact, current, state)?.0
        }
        VmRuntimeStateTransaction::SetLastLoad {
            version,
            slot,
            text,
        } => {
            let mut memory = current.clone();
            memory.set_last_load(artifact, *version, *slot, text);
            memory
        }
        VmRuntimeStateTransaction::Mutate { .. } => current.clone(),
    })
}

pub(crate) fn make_frame<'a>(
    id: FrameId,
    generation: GenerationId,
    function: &BytecodeFunction,
    local_definitions: impl IntoIterator<Item = &'a erabasic_bytecode::BytecodeGlobal>,
    arguments: Vec<VmValue>,
    return_value_to_caller: bool,
    event_context: bool,
) -> Frame {
    let mut locals: BTreeMap<_, _> = local_definitions
        .into_iter()
        .map(|definition| (definition.key, VariableCell::new(definition)))
        .collect();
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        if let Some(cell) = locals.get_mut(&parameter.key) {
            if parameter.by_reference {
                // REF declarations describe the target shape, but their frame
                // storage always contains one opaque alias. This also covers a
                // scalar `#DIM REF value`: its declaration has shape `[1]`, so
                // treating only zero-sized REF arrays specially would attempt
                // to write an IntegerPlace into an Integer cell.
                cell.replace_shape(parameter.value_type, vec![1], vec![argument])
                    .expect("validated reference argument matches its parameter");
            } else {
                cell.write(&parameter.indices, argument)
                    .expect("validated parameter destination fits its local storage");
            }
        }
    }
    Frame {
        id,
        generation,
        function: function.key,
        instruction: 0,
        stack: Vec::new(),
        for_loops: Vec::new(),
        select_values: Vec::new(),
        locals,
        return_value_to_caller,
        event_context,
        event_dispatch: None,
    }
}

pub(crate) fn validate_arguments(
    function: &BytecodeFunction,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    if arguments.len() != function.parameters.len() {
        return Err(VmError::InvalidArguments(format!(
            "function {} expects {} arguments, found {}",
            function.name,
            function.parameters.len(),
            arguments.len()
        )));
    }
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        if parameter.value_type != argument.value_type() {
            return Err(VmError::InvalidArguments(format!(
                "function {} expects {:?}, found {:?}",
                function.name,
                parameter.value_type,
                argument.value_type()
            )));
        }
    }
    Ok(())
}

pub(crate) fn bind_persistent_arguments(
    memory: &mut Memory,
    generation: GenerationId,
    function: &BytecodeFunction,
    program: &ProgramGeneration,
    arguments: &[VmValue],
) -> Result<(), VmError> {
    let artifact = &program.artifact;
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        let Some(definition) = program.global(parameter.key) else {
            return Err(VmError::InvalidState("parameter storage is missing".into()));
        };
        if definition.storage == BytecodeStorage::FunctionLocal {
            continue;
        }
        let (character, indices) = if definition.storage == BytecodeStorage::Character
            && parameter.indices.len() > definition.dimensions.len()
        {
            (
                usize::try_from(parameter.indices[0]).unwrap_or(usize::MAX),
                &parameter.indices[1..],
            )
        } else {
            (
                if definition.storage == BytecodeStorage::Character {
                    memory.target_character(artifact, generation)
                } else {
                    0
                },
                parameter.indices.as_slice(),
            )
        };
        memory
            .cell_mut(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("parameter storage is missing".into()))?
            .write(indices, argument.clone())
            .map_err(VmError::InvalidState)?;
    }
    Ok(())
}

pub(crate) fn prepare_dynamic_arguments(
    function: &BytecodeFunction,
    mut arguments: Vec<VmValue>,
    compatibility: erabasic_bytecode::BytecodeCallCompatibility,
) -> Result<Vec<VmValue>, VmError> {
    if arguments.len() > function.parameters.len() {
        return Err(VmError::InvalidArguments(format!(
            "function {} expects at most {} arguments, found {}",
            function.name,
            function.parameters.len(),
            arguments.len()
        )));
    }
    while arguments.len() < function.parameters.len() {
        arguments.push(VmValue::Integer(i64::MIN));
    }
    for (parameter, argument) in function.parameters.iter().zip(&mut arguments) {
        if matches!(argument, VmValue::Integer(value) if *value == i64::MIN) {
            if parameter.by_reference {
                return Err(VmError::InvalidArguments(format!(
                    "function {} omits a reference argument",
                    function.name
                )));
            }
            *argument = match &parameter.default {
                Some(BytecodeConstant::Integer(value)) => VmValue::Integer(*value),
                Some(BytecodeConstant::String(value)) => VmValue::String(value.clone()),
                None if compatibility.allow_omitted_arguments => match parameter.value_type {
                    BytecodeType::Integer => VmValue::Integer(0),
                    BytecodeType::String => VmValue::String(String::new()),
                    BytecodeType::IntegerPlace | BytecodeType::StringPlace => {
                        return Err(VmError::InvalidArguments(format!(
                            "function {} omits a reference argument",
                            function.name
                        )));
                    }
                },
                None => {
                    return Err(VmError::InvalidArguments(format!(
                        "function {} omits a required argument",
                        function.name
                    )));
                }
            };
        }
        if compatibility.auto_convert_integer_to_string
            && parameter.value_type == BytecodeType::String
            && matches!(argument, VmValue::Integer(_))
            && !parameter.by_reference
        {
            let VmValue::Integer(value) = argument else {
                unreachable!("checked integer argument")
            };
            *argument = VmValue::String(value.to_string());
        }
    }
    validate_arguments(function, &arguments)?;
    Ok(arguments)
}

pub(crate) fn find_global(
    artifact: &BytecodeArtifact,
    key: SymbolKey,
) -> Result<&erabasic_bytecode::BytecodeGlobal, VmError> {
    artifact
        .globals
        .iter()
        .find(|definition| definition.key == key)
        .ok_or_else(|| VmError::InvalidState(format!("variable {key:?} is not defined")))
}

fn find_frame(
    fiber: &Fiber,
    frame: Option<FrameId>,
    owner: Option<SymbolKey>,
) -> Result<&Frame, VmError> {
    fiber
        .frames
        .iter()
        .rev()
        .find(|candidate| {
            frame.is_none_or(|frame| candidate.id == frame)
                && owner.is_none_or(|owner| candidate.function == owner)
        })
        .ok_or_else(|| VmError::InvalidState("place frame is no longer active".into()))
}

fn find_frame_mut(
    fiber: &mut Fiber,
    frame: Option<FrameId>,
    owner: Option<SymbolKey>,
) -> Result<&mut Frame, VmError> {
    fiber
        .frames
        .iter_mut()
        .rev()
        .find(|candidate| {
            frame.is_none_or(|frame| candidate.id == frame)
                && owner.is_none_or(|owner| candidate.function == owner)
        })
        .ok_or_else(|| VmError::InvalidState("place frame is no longer active".into()))
}
