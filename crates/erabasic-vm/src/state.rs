use std::collections::{BTreeMap, BTreeSet, VecDeque};

use erabasic_bytecode::{
    BytecodeArtifact, BytecodeFunction, BytecodeStorage, BytecodeType, Digest, SymbolKey,
};
use erabasic_validator::ValidatedArtifact;
use serde::{Deserialize, Serialize};

use crate::{
    FiberId, FiberStatus, FrameId, GenerationId, HostReady, HostRequestId, HostWaitStability,
    Memory, PlaceDescriptor, VariableCell, VmConfig, VmError, VmFault, VmValue,
    hot_reload::HotReloadPlan,
};
use crate::{PreparedRuntimeState, VmRuntimeRead, VmRuntimeStatePort, VmRuntimeStateTransaction};

#[derive(Clone, Debug)]
pub(crate) struct ProgramGeneration {
    pub artifact: BytecodeArtifact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Frame {
    pub id: FrameId,
    pub generation: GenerationId,
    pub function: SymbolKey,
    pub instruction: usize,
    pub stack: Vec<VmValue>,
    pub locals: BTreeMap<SymbolKey, VariableCell>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct WaitingHost {
    pub request: HostRequestId,
    pub import: erabasic_bytecode::RuntimeImport,
    pub result: Option<BytecodeType>,
    pub stability: HostWaitStability,
    pub rebind_payload: Vec<u8>,
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

pub struct Vm {
    pub(crate) config: VmConfig,
    pub(crate) generations: BTreeMap<GenerationId, ProgramGeneration>,
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
}

impl Vm {
    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        let artifact = artifact.into_inner();
        let memory = Memory::new_game(&artifact);
        let generation = GenerationId(1);
        Self {
            config,
            generations: BTreeMap::from([(generation, ProgramGeneration { artifact })]),
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
        }
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
        let artifact = &self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("current generation is missing".into()))?
            .artifact;
        let function_definition = artifact
            .functions
            .iter()
            .find(|candidate| candidate.key == function)
            .ok_or(VmError::MissingFunction(function))?;
        validate_arguments(function_definition, &arguments)?;
        let fiber_id = FiberId(self.next_fiber);
        self.next_fiber = self.next_fiber.saturating_add(1);
        let frame_id = FrameId(self.next_frame);
        self.next_frame = self.next_frame.saturating_add(1);
        let frame = make_frame(
            frame_id,
            generation,
            function_definition,
            artifact,
            arguments,
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
        fiber.state = FiberState::Cancelled;
        Ok(())
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
        let artifact = self.artifact();
        let definition = find_global(artifact, variable)?;
        if definition.storage == BytecodeStorage::FunctionLocal {
            return Err(VmError::InvalidState(
                "frame-local variables require a place descriptor".into(),
            ));
        }
        let character = character.map_or_else(
            || {
                self.memory
                    .target_character(artifact, self.current_generation)
            },
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
        let definition = find_global(self.artifact(), variable)?.clone();
        if !definition.mutable {
            return Err(VmError::InvalidState("variable is immutable".into()));
        }
        let character = character.map_or_else(
            || self.memory.target_character(self.artifact(), generation),
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell_mut(generation, &definition, character)
            .ok_or_else(|| VmError::InvalidState("variable storage is unavailable".into()))?
            .write(indices, value)
            .map_err(VmError::InvalidState)
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
        if definition.storage == BytecodeStorage::FunctionLocal {
            let frame = find_frame(fiber, place.frame, definition.owner)?;
            return frame
                .locals
                .get(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .read(&place.indices)
                .map_err(VmError::InvalidState);
        }
        let character = place.character.map_or_else(
            || {
                self.generations.get(&generation).map_or(0, |program| {
                    self.memory.target_character(&program.artifact, generation)
                })
            },
            |value| usize::try_from(value).unwrap_or(usize::MAX),
        );
        self.memory
            .cell(generation, definition, character)
            .ok_or_else(|| VmError::InvalidState("place storage is unavailable".into()))?
            .read(&place.indices)
            .map_err(VmError::InvalidState)
    }

    pub(crate) fn write_place(
        &mut self,
        fiber: &mut Fiber,
        place: &PlaceDescriptor,
        value: VmValue,
    ) -> Result<(), VmError> {
        self.write_place_internal(fiber, place, value, false)
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
            let frame = find_frame_mut(fiber, place.frame, definition.owner)?;
            return frame
                .locals
                .get_mut(&definition.key)
                .ok_or_else(|| VmError::InvalidState("local variable is unavailable".into()))?
                .write(&place.indices, value)
                .map_err(VmError::InvalidState);
        }
        let character = place.character.map_or_else(
            || {
                self.generations.get(&generation).map_or(0, |program| {
                    self.memory.target_character(&program.artifact, generation)
                })
            },
            |index| usize::try_from(index).unwrap_or(usize::MAX),
        );
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
        let artifact = &self
            .generations
            .get(&generation)
            .ok_or_else(|| VmError::InvalidState("place generation was reclaimed".into()))?
            .artifact;
        Ok((generation, find_global(artifact, place.variable)?))
    }
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
            VmRuntimeStateTransaction::ResetNewGame | VmRuntimeStateTransaction::RestoreEraState(_)
        );
        let mut memory = match &transaction {
            VmRuntimeStateTransaction::ResetNewGame => Memory::new_game(artifact),
            VmRuntimeStateTransaction::RestoreEraState(state) => {
                crate::save::prepare_era_memory(artifact, state)?.0
            }
            VmRuntimeStateTransaction::Mutate { .. } => self.memory.clone(),
        };
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
                    cell.values.fill(fill.value.clone());
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

pub(crate) fn make_frame(
    id: FrameId,
    generation: GenerationId,
    function: &BytecodeFunction,
    artifact: &BytecodeArtifact,
    arguments: Vec<VmValue>,
) -> Frame {
    let mut locals: BTreeMap<_, _> = artifact
        .globals
        .iter()
        .filter(|definition| {
            definition.storage == BytecodeStorage::FunctionLocal
                && definition.owner == Some(function.key)
        })
        .map(|definition| (definition.key, VariableCell::new(definition)))
        .collect();
    for (parameter, argument) in function.parameters.iter().zip(arguments) {
        if let Some(cell) = locals.get_mut(&parameter.key)
            && let Some(slot) = cell.values.first_mut()
        {
            *slot = argument;
        }
    }
    Frame {
        id,
        generation,
        function: function.key,
        instruction: 0,
        stack: Vec::new(),
        locals,
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
