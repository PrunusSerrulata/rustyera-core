#[allow(clippy::wildcard_imports)]
use super::*;

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
            if program.is_reference_variable(definition.key) {
                return None;
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
        if place.backing.is_some() {
            return Err(VmError::InvalidState(
                "Host cannot inject an array backing identity".into(),
            ));
        }
        let fiber = self
            .fibers
            .get(&fiber)
            .ok_or_else(|| VmError::InvalidState("Host place fiber is missing".into()))?;
        self.place_dimensions(fiber, place)
    }

    pub(crate) fn place_dimensions(
        &self,
        fiber: &Fiber,
        place: &PlaceDescriptor,
    ) -> Result<Vec<u64>, VmError> {
        let mut place = place.clone();
        for _ in 0..64 {
            if place.backing.is_some() {
                return Ok(self
                    .checked_array_backing(fiber, &place)?
                    .1
                    .dimensions
                    .clone());
            }
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

    pub(crate) fn new_for_title_with_progress(
        artifact: ValidatedArtifact,
        config: VmConfig,
        progress: &mut dyn FnMut(VmPreparationProgress),
    ) -> Self {
        Self::new_with_memory_and_progress(artifact, config, true, Some(progress))
    }

    pub(crate) fn new_for_title_from_program_with_progress(
        program: Arc<ProgramGeneration>,
        config: VmConfig,
        progress: &mut dyn FnMut(VmPreparationProgress),
    ) -> Self {
        let mut progress = Some(progress);
        let memory = initialize_title_memory(&program.artifact, &mut progress);
        Self::from_program_and_memory(program, config, memory)
    }

    fn new_with_memory(artifact: ValidatedArtifact, config: VmConfig, title_state: bool) -> Self {
        Self::new_with_memory_and_progress(artifact, config, title_state, None)
    }

    fn new_with_memory_and_progress(
        artifact: ValidatedArtifact,
        config: VmConfig,
        title_state: bool,
        mut progress: Option<&mut dyn FnMut(VmPreparationProgress)>,
    ) -> Self {
        let artifact = artifact.into_shared();
        let memory = if title_state {
            initialize_title_memory(&artifact, &mut progress)
        } else {
            report_vm_preparation(&mut progress, VmPreparationStage::InitializingMemory, 0, 1);
            Memory::new_game(&artifact)
        };
        if !title_state {
            report_vm_preparation(&mut progress, VmPreparationStage::InitializingMemory, 1, 1);
        }
        let program = Arc::new(ProgramGeneration::new_with_progress(artifact, progress));
        Self::from_program_and_memory(program, config, memory)
    }

    fn from_program_and_memory(
        program: Arc<ProgramGeneration>,
        config: VmConfig,
        memory: Memory,
    ) -> Self {
        let generation = GenerationId(1);
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
            compatibility_warning_sites: BTreeSet::new(),
            pending_compatibility_warnings: Vec::new(),
            debug: DebugState::default(),
            regex_cache: RegexCache::default(),
            find_element_cache: HashMap::new(),
            find_element_cache_retained_bytes: 0,
            function_memo_cache: HashMap::new(),
            function_memo_cache_retained_bytes: 0,
            active_function_memos: HashMap::new(),
            path_memo_cache: HashMap::new(),
            path_memo_key_count: 0,
            path_memo_retained_bytes: 0,
            active_path_memo_fiber: std::cell::Cell::new(None),
            active_path_memo: std::cell::RefCell::new(None),
            #[cfg(test)]
            path_memo_replays: 0,
        }
    }

    pub(crate) fn into_current_program(mut self) -> Arc<ProgramGeneration> {
        self.generations
            .remove(&self.current_generation)
            .expect("the current generation is always retained")
    }

    pub(crate) fn compile_regex(
        &mut self,
        pattern: &str,
    ) -> Result<regex::Regex, crate::ExecutionFailure> {
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
                fault_hook: None,
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
        let mut state = self
            .fibers
            .remove(&fiber)
            .ok_or(VmError::UnknownFiber(fiber))?;
        if let Some(hook) = state.fault_hook.as_ref() {
            let failure = crate::ExecutionFailure::classified(
                crate::FaultCategory::Cancellation,
                crate::VmFaultCode::Host,
                "final-fault hook was cancelled",
            );
            let mut origin = hook.original.origin();
            origin.command = "CANCEL".into();
            let cancellation = crate::VmFault::from_origin(fiber, origin, failure);
            let transition = self.transition_fault(&mut state, cancellation);
            debug_assert!(matches!(
                transition,
                crate::interpreter::fault_hooks::FaultTransition::Published(_)
            ));
        } else {
            for frame in &state.frames {
                self.active_function_memos.remove(&frame.id);
            }
            state.frames.clear();
            state.state = FiberState::Cancelled;
        }
        self.fibers.insert(fiber, state);
        self.prune_bit_leases();
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
        if retired.is_empty() {
            return 0;
        }
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
        let outcome = self.return_frame(&mut fiber, value.cloned(), None, false);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                self.fibers.insert(fiber_id, fiber);
                return Err(error);
            }
        };
        fiber.mark_progress();
        if matches!(outcome, super::FrameReturn::Continue) {
            fiber.state = FiberState::Runnable;
            self.runnable.push_back(fiber_id);
        }
        self.fibers.insert(fiber_id, fiber);
        self.prune_bit_leases();
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
}

fn initialize_title_memory(
    artifact: &BytecodeArtifact,
    progress: &mut Option<&mut dyn FnMut(VmPreparationProgress)>,
) -> Memory {
    let total = u64::try_from(artifact.globals.len())
        .unwrap_or(u64::MAX - 1)
        .saturating_add(1)
        .max(1);
    report_vm_preparation(progress, VmPreparationStage::InitializingMemory, 0, total);
    Memory::title_with_progress(artifact, &mut |completed, total| {
        report_vm_preparation(
            progress,
            VmPreparationStage::InitializingMemory,
            completed,
            total,
        );
    })
}
