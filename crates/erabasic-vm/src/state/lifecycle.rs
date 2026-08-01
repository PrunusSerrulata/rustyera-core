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
}
