#[allow(clippy::wildcard_imports)]
use super::*;
impl RuntimeVm {
    /// Consume the live VM and retain only its immutable program index for a title restart.
    #[must_use]
    pub fn retain_program_index_for_title(self) -> RetainedProgramIndex {
        let Self {
            vm,
            natives,
            pending_natives,
            candidate_base_column_stamp: _,
            candidate_base_array_stamp: _,
            line_columns: _,
            pending_completion_events: _,
        } = self;
        drop(natives);
        drop(pending_natives);
        let program = vm.into_current_program();
        RetainedProgramIndex { program }
    }

    /// Read a place supplied to a Host extension without exposing VM storage layouts.
    ///
    /// # Errors
    ///
    /// The place must still belong to the requesting fiber and current generation.
    pub fn read_host_place(
        &self,
        fiber: FiberId,
        place: &PlaceDescriptor,
    ) -> Result<VmValue, VmError> {
        let fiber = self
            .vm
            .fibers
            .get(&fiber)
            .ok_or_else(|| VmError::InvalidState("Host place fiber is missing".into()))?;
        self.vm.read_place(fiber, place)
    }
    /// Fork authoritative memory and Native state while discarding every live
    /// fiber. Candidate SAVEINFO execution uses this isolated timeline so a
    /// failure cannot leak stack, scheduler, random or structured state.
    ///
    /// # Errors
    ///
    /// Returns an error when a registered Native service cannot be snapshotted.
    pub fn fork_isolated(&self) -> Result<Self, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::InvalidState(
                "pending Host completion events must be delivered before forking".into(),
            ));
        }
        let base_array_stamp = self.vm.array_lease_stamp()?;
        let mut vm = self.vm.clone();
        let mut array_roots =
            crate::interpreter::bit_calls::live_bit_leases(self.vm.fibers.values());
        array_roots.extend(self.vm.memory.array_leases.protected.iter().copied());
        vm.memory.array_leases.retain(&array_roots);
        vm.memory.array_leases.protected = array_roots;
        vm.fibers.clear();
        vm.runnable.clear();
        vm.primary_fiber = None;
        vm.next_fiber = 1;
        vm.pending_reload = None;
        vm.debug = DebugState::default();
        vm.clear_path_memo_cache();
        vm.active_path_memo_fiber.set(None);
        vm.active_path_memo.borrow_mut().take();
        let roots =
            self.natives
                .candidate_map_roots(&crate::interpreter::map_calls::live_map_leases(
                    self.vm.fibers.values(),
                ));
        self.natives
            .retain_map_leases(&roots)
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        let mut natives = self
            .natives
            .fork_for_artifact(vm.artifact())
            .map_err(VmError::Snapshot)?;
        natives
            .protect_map_roots(roots)
            .map_err(VmError::Snapshot)?;
        Ok(Self {
            vm,
            natives,
            pending_natives: None,
            candidate_base_array_stamp: Some(base_array_stamp),
            candidate_base_column_stamp: CandidateColumnBase::Forked(
                self.natives
                    .column_identity_stamp()
                    .map_err(VmError::Snapshot)?,
                self.natives.map_lease_stamp().map_err(VmError::Snapshot)?,
            ),
            line_columns: self.line_columns,
            pending_completion_events: Vec::new(),
        })
    }

    /// Fork a complete candidate for an atomic state replacement.
    ///
    /// Unlike `fork_isolated`, this candidate does not retain any caller-frame array or MAP
    /// roots: a successful ordinary load discards the current execution timeline. The live VM
    /// remains untouched while traditional variables, structured data, and random state are
    /// validated on the returned candidate.
    ///
    /// # Errors
    /// Returns an error for pending completion events or an already isolated parent.
    pub fn fork_for_state_replacement(&self) -> Result<Self, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::InvalidState(
                "pending Host completion events must be delivered before replacement forking"
                    .into(),
            ));
        }
        if !self.vm.memory.array_leases.protected.is_empty()
            || !self.natives.protected_map_roots().is_empty()
        {
            return Err(VmError::InvalidState(
                "an isolated candidate cannot become a replacement source".into(),
            ));
        }
        let mut vm = self.vm.clone();
        vm.clear_execution();
        let natives = self
            .natives
            .fork_for_artifact(vm.artifact())
            .map_err(VmError::Snapshot)?;
        natives
            .retain_map_leases(&BTreeSet::new())
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        let mut result = Self {
            vm,
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: self.line_columns,
            pending_completion_events: Vec::new(),
        };
        result.refresh_draw_line_string();
        Ok(result)
    }

    /// Replace the complete VM/native state with a previously validated replacement candidate.
    ///
    /// # Errors
    /// Rejects stale artifacts, generations, candidates with execution, or pending completions.
    pub fn commit_state_replacement(&mut self, replacement: Self) -> Result<(), VmError> {
        self.validate_state_replacement(&replacement)?;
        *self = replacement;
        Ok(())
    }

    /// Validate a complete replacement without publishing it.
    ///
    /// # Errors
    /// Rejects the same stale or executable candidates as [`Self::commit_state_replacement`].
    pub fn validate_state_replacement(&self, replacement: &Self) -> Result<(), VmError> {
        if !self.pending_completion_events.is_empty()
            || !replacement.pending_completion_events.is_empty()
        {
            return Err(VmError::InvalidState(
                "state replacement cannot cross pending Host completion events".into(),
            ));
        }
        if self.vm.artifact_id() != replacement.vm.artifact_id()
            || self.vm.current_generation() != replacement.vm.current_generation()
        {
            return Err(VmError::InvalidState(
                "state replacement belongs to a stale artifact generation".into(),
            ));
        }
        if !replacement.vm.fibers.is_empty() || replacement.has_work() {
            return Err(VmError::InvalidState(
                "state replacement candidate unexpectedly contains execution".into(),
            ));
        }
        if !replacement.vm.memory.array_leases.protected.is_empty()
            || !replacement.natives.protected_map_roots().is_empty()
        {
            return Err(VmError::InvalidState(
                "state replacement candidate retained parent roots".into(),
            ));
        }
        Ok(())
    }

    /// Consume a completed candidate after all terminal completion events were delivered.
    ///
    /// # Errors
    /// Rejects a candidate whose Host outcome has not yet been observed by its caller.
    pub fn into_candidate_state(self) -> Result<PreparedCandidateState, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::InvalidState(
                "candidate has undelivered Host completion events".into(),
            ));
        }
        Ok(PreparedCandidateState {
            artifact_id: self.vm.artifact_id(),
            base_column_stamp: self.candidate_base_column_stamp,
            base_array_stamp: self.candidate_base_array_stamp,
            memory: self.vm.memory,
            natives: self.natives,
        })
    }

    /// Atomically install candidate memory and Native services without replacing
    /// the caller's fibers or call stacks.
    ///
    /// # Errors
    ///
    /// Rejects a candidate prepared for another artifact generation.
    pub fn commit_candidate_state(
        &mut self,
        mut candidate: PreparedCandidateState,
    ) -> Result<(), VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::InvalidState(
                "pending Host completion events must be delivered before candidate commit".into(),
            ));
        }
        if candidate.artifact_id != self.vm.artifact_id() {
            return Err(VmError::InvalidState(
                "candidate state belongs to another artifact".into(),
            ));
        }
        let base_array_stamp = candidate.base_array_stamp.as_ref().ok_or_else(|| {
            VmError::InvalidState("candidate has no array lease source guard".into())
        })?;
        self.vm.validate_array_lease_stamp(base_array_stamp)?;
        let mut roots = crate::interpreter::bit_calls::live_bit_leases(self.vm.fibers.values());
        roots.extend(self.vm.memory.array_leases.protected.iter().copied());
        if candidate
            .memory
            .array_leases
            .entries
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != roots
            || roots.iter().any(|id| {
                candidate
                    .memory
                    .array_leases
                    .entries
                    .get(id)
                    .zip(self.vm.memory.array_leases.entries.get(id))
                    .is_none_or(|(candidate, parent)| {
                        candidate.owner != parent.owner
                            || candidate.input != parent.input
                            || candidate.length != parent.length
                    })
            })
        {
            return Err(VmError::InvalidState(
                "candidate array leases differ from protected parent roots".into(),
            ));
        }
        candidate
            .memory
            .array_leases
            .protected
            .clone_from(&self.vm.memory.array_leases.protected);
        // Inherited roots belong to a still-live outer runtime, not this isolated
        // parent's discarded frames. Their exact set/source stamp is checked above;
        // only this parent's own frame roots can be validated against its fibers.
        let expected = roots
            .iter()
            .filter(|id| !self.vm.memory.array_leases.protected.contains(*id))
            .filter_map(|id| {
                self.vm
                    .memory
                    .array_leases
                    .entries
                    .get(id)
                    .map(|lease| (*id, lease.owner))
            })
            .collect();
        candidate.memory.validate_array_leases(
            &self.vm.fibers,
            &expected,
            self.vm.config.maximum_operand_stack,
        )?;
        let CandidateColumnBase::Forked(base, map_base) = candidate.base_column_stamp else {
            return Err(VmError::InvalidState(
                "candidate state was not forked from a live runtime".into(),
            ));
        };
        self.natives
            .validate_column_identity_stamp(base)
            .map_err(VmError::InvalidState)?;
        self.natives
            .validate_map_lease_stamp(map_base)
            .map_err(VmError::InvalidState)?;
        let roots =
            self.natives
                .candidate_map_roots(&crate::interpreter::map_calls::live_map_leases(
                    self.vm.fibers.values(),
                ));
        candidate
            .natives
            .finish_map_candidate(&roots, self.natives.protected_map_roots())
            .map_err(VmError::InvalidState)?;
        self.vm.memory = candidate.memory;
        self.vm.prune_bit_leases();
        self.natives = candidate.natives;
        self.refresh_draw_line_string();
        Ok(())
    }

    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        let natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        let mut runtime = Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    #[must_use]
    pub fn new_with_seed(artifact: ValidatedArtifact, config: VmConfig, seed: u64) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    /// Construct the pre-title state used by the runtime system flow.
    ///
    /// Variable defaults are available to `SYSTEM_TITLE`, while `ResetData` and
    /// initial character insertion remain deferred until the built-in new-game
    /// selection is accepted.
    #[must_use]
    pub fn new_for_title_with_seed(
        artifact: ValidatedArtifact,
        config: VmConfig,
        seed: u64,
    ) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new_for_title(artifact, config),
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    #[must_use]
    pub fn new_for_title_with_seed_and_progress(
        artifact: ValidatedArtifact,
        config: VmConfig,
        seed: u64,
        progress: &mut dyn FnMut(crate::VmPreparationProgress),
    ) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new_for_title_with_progress(artifact, config, progress),
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    /// Build title memory and Native services around a previously retained program index.
    #[must_use]
    pub fn new_for_title_from_retained_program_with_seed_and_progress(
        retained: RetainedProgramIndex,
        config: VmConfig,
        seed: u64,
        progress: &mut dyn FnMut(crate::VmPreparationProgress),
    ) -> Self {
        let RetainedProgramIndex { program } = retained;
        let natives = NativeServiceRegistry::for_artifact_with_seed(&program.artifact, seed);
        let mut runtime = Self {
            vm: Vm::new_for_title_from_program_with_progress(program, config, progress),
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    /// Synchronize calculated line-width state with the current frontend projection.
    pub fn set_line_columns(&mut self, columns: u32) {
        self.line_columns = columns.max(1);
        self.refresh_draw_line_string();
    }

    /// Synchronize runtime formatting and calculated strings with project width policy.
    pub fn set_character_width_mode(&mut self, mode: crate::CharacterWidthMode) {
        let changed = self.natives.character_width_mode() != mode;
        self.natives.set_character_width_mode(mode);
        if let Some(pending) = &mut self.pending_natives {
            pending.0.set_character_width_mode(mode);
        }
        if changed {
            // Width-sensitive compiler natives are memo-safe only within one width policy.
            // Configuration changes happen between VM slices, so discard both completed and
            // in-progress execution memos before subsequent formatting can observe the new mode.
            self.vm.clear_derived_caches();
            self.vm.active_function_memos.clear();
            self.vm.clear_path_memo_cache();
            self.vm.active_path_memo_fiber.set(None);
            self.vm.active_path_memo.borrow_mut().take();
        }
        self.refresh_draw_line_string();
    }

    #[must_use]
    pub fn character_width_mode(&self) -> crate::CharacterWidthMode {
        self.natives.character_width_mode()
    }

    pub(super) fn refresh_draw_line_string(&mut self) {
        let pattern = self
            .vm
            .artifact()
            .project_data
            .static_data
            .replace
            .draw_line_string
            .clone();
        let value = crate::logical_line_string_with_mode(
            &pattern,
            usize::try_from(self.line_columns).unwrap_or(usize::MAX),
            self.character_width_mode(),
        )
        .unwrap_or(pattern);
        self.vm.set_runtime_calculated_string("DRAWLINESTR", &value);
    }

    pub(super) fn waiting_host_import(
        &self,
        fiber_id: FiberId,
        wait: &crate::state::WaitingHost,
    ) -> Option<HostImport> {
        let generation = self.vm.generations.get(&wait.origin.generation)?;
        let import = if let Some(scope) = wait.form_scope {
            if scope.fiber != fiber_id {
                return None;
            }
            let fiber = self.vm.fibers.get(&fiber_id)?;
            let frame = fiber
                .frames
                .last()
                .filter(|frame| frame.id == scope.frame)?;
            frame
                .runtime_form
                .as_ref()?
                .waiting_host_import(scope, &wait.origin, generation)?
        } else {
            let index = generation.host_import_index(wait.import.key)?;
            generation.artifact.host_imports.get(index)?.clone()
        };
        (import.import == wait.import && import.import.result == wait.result).then_some(import)
    }

    /// The origin is taken from the VM's actual waiting continuation, not the frontend request.
    #[must_use]
    pub fn host_request_scope(
        &self,
        request: crate::HostRequestId,
    ) -> Option<crate::RuntimeHostScope> {
        self.vm.fibers.iter().find_map(|(fiber, state)| {
            let FiberState::WaitingHost(wait) = &state.state else {
                return None;
            };
            if wait.request != request {
                return None;
            }
            self.waiting_host_import(*fiber, wait)?;
            wait.form_scope
        })
    }
    #[must_use]
    pub fn host_scope_is_live(&self, scope: crate::RuntimeHostScope) -> bool {
        self.vm
            .fibers
            .get(&scope.fiber)
            .and_then(|fiber| fiber.frames.iter().find(|frame| frame.id == scope.frame))
            .and_then(|frame| frame.runtime_form.as_ref())
            .is_some_and(|form| form.contains_host_scope(scope))
    }
    #[must_use]
    pub fn host_scope_has_html_ticket(&self, scope: crate::RuntimeHostScope, ticket: &str) -> bool {
        self.vm
            .fibers
            .get(&scope.fiber)
            .and_then(|fiber| fiber.frames.iter().find(|frame| frame.id == scope.frame))
            .and_then(|frame| frame.runtime_form.as_ref())
            .is_some_and(|form| form.scope_has_html_ticket(scope, ticket))
    }

    #[must_use]
    pub const fn vm(&self) -> &Vm {
        &self.vm
    }

    pub const fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    #[must_use]
    pub fn fiber_frame_count(&self, fiber: FiberId) -> Option<usize> {
        self.vm.fibers.get(&fiber).map(|fiber| fiber.frames.len())
    }

    /// Read an exact retained caller frame without exposing VM-owned references.
    /// `depth` is zero-based from the root; nested calls preserve the owner's identity.
    #[must_use]
    pub fn host_frame_identity(
        &self,
        fiber: FiberId,
        depth: usize,
    ) -> Option<(
        crate::FrameId,
        crate::GenerationId,
        erabasic_bytecode::SymbolKey,
    )> {
        let frame = self.vm.fibers.get(&fiber)?.frames.get(depth)?;
        Some((frame.id, frame.generation, frame.function))
    }

    /// Only runtime snapshot validation consumes this bounded ownership inventory.
    #[must_use]
    pub fn active_html_line_scopes(&self) -> Vec<(crate::RuntimeHostScope, String)> {
        self.vm
            .fibers
            .iter()
            .flat_map(|(id, fiber)| {
                fiber
                    .frames
                    .iter()
                    .filter_map(|frame| frame.runtime_form.as_ref())
                    .flat_map(|form| form.html_scope_tickets(*id))
            })
            .collect()
    }

    /// Return the active dimensions for a named global, local, or bound
    /// reference variable in the requesting fiber.
    #[must_use]
    pub fn variable_dimensions(&self, fiber: FiberId, name: &str) -> Option<Vec<u64>> {
        self.vm.variable_dimensions(fiber, name)
    }

    /// Return the active dimensions for a place supplied by a Host call.
    ///
    /// # Errors
    ///
    /// The place must still belong to the requesting fiber and resolve to an
    /// active variable.
    pub fn host_place_dimensions(
        &self,
        fiber: FiberId,
        place: &PlaceDescriptor,
    ) -> Result<Vec<u64>, VmError> {
        self.vm.host_place_dimensions(fiber, place)
    }

    /// Whether at least one fiber can make progress without a host completion.
    #[must_use]
    pub fn has_runnable_fibers(&self) -> bool {
        self.vm
            .fibers
            .values()
            .any(|fiber| matches!(fiber.state, FiberState::Runnable))
    }

    /// Whether a committed Host outcome still needs delivery to the runtime.
    #[must_use]
    pub fn has_pending_events(&self) -> bool {
        !self.pending_completion_events.is_empty()
    }

    /// Whether driving can deliver an event or execute a runnable fiber.
    #[must_use]
    pub fn has_work(&self) -> bool {
        self.has_pending_events() || self.has_runnable_fibers()
    }

    /// Export the exact SFMT stream position used by RAND natives.
    ///
    /// # Errors
    ///
    /// Returns an error if the native random state is unavailable or poisoned.
    pub fn export_random_state(&self) -> Result<Vec<i64>, VmError> {
        self.natives.random_values().map_err(VmError::InvalidState)
    }

    /// Restore a state previously returned by `export_random_state`.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded SFMT state is invalid or unavailable.
    pub fn restore_random_state(&mut self, values: &[i64]) -> Result<(), VmError> {
        self.natives
            .set_random_values(values)
            .map_err(VmError::InvalidState)
    }

    /// Export only VAREXT values declared for the requested save scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared structured state cannot be serialized.
    pub fn structured_extensions(
        &self,
        scope: StructuredScope,
    ) -> Result<Vec<StructuredExtension>, VmError> {
        self.natives
            .structured_extensions(scope)
            .map_err(VmError::InvalidState)
    }

    /// Prepare ordinary VM memory and VAREXT data as one atomic transaction.
    /// Unknown or undeclared records are deliberately ignored and can be retained
    /// losslessly by the runtime save adapter.
    ///
    /// # Errors
    ///
    /// Returns an error without mutation when either memory or extension data is invalid.
    pub fn prepare_runtime_state_with_extensions(
        &self,
        transaction: VmRuntimeStateTransaction,
        scope: StructuredScope,
        values: &[StructuredExtension],
    ) -> Result<(PreparedRuntimeState, BTreeSet<(u8, String)>), VmError> {
        let (structured_state, imported) = self
            .natives
            .prepare_structured_import(&transaction, scope, values)
            .map_err(VmError::InvalidState)?;
        let mut prepared = self.vm.prepare_runtime_state(transaction)?;
        prepared.structured_state = structured_state;
        prepared.base_map_stamp = self
            .natives
            .map_lease_stamp()
            .map_err(VmError::InvalidState)?;
        prepared.base_column_stamp = self
            .natives
            .column_identity_stamp()
            .map_err(VmError::InvalidState)?;
        Ok((prepared, imported))
    }
}

impl VmRuntimeStatePort for RuntimeVm {
    fn read_runtime_state(&self, reads: &[VmRuntimeRead]) -> Result<Vec<VmValue>, VmError> {
        self.vm.read_runtime_state(reads)
    }

    fn prepare_runtime_state(
        &self,
        transaction: VmRuntimeStateTransaction,
    ) -> Result<PreparedRuntimeState, VmError> {
        let structured_state = self
            .natives
            .prepare_structured_transaction(&transaction)
            .map_err(VmError::InvalidState)?;
        let mut prepared = self.vm.prepare_runtime_state(transaction)?;
        prepared.structured_state = structured_state;
        prepared.base_map_stamp = self
            .natives
            .map_lease_stamp()
            .map_err(VmError::InvalidState)?;
        prepared.base_column_stamp = self
            .natives
            .column_identity_stamp()
            .map_err(VmError::InvalidState)?;
        Ok(prepared)
    }

    fn commit_runtime_state(&mut self, mut prepared: PreparedRuntimeState) -> Result<(), VmError> {
        self.vm
            .validate_array_lease_stamp(&prepared.base_array_stamp)?;
        if prepared.generation != self.vm.current_generation() {
            return Err(VmError::InvalidState(
                "runtime state transaction belongs to a stale generation".into(),
            ));
        }
        self.natives
            .validate_column_identity_stamp(prepared.base_column_stamp)
            .map_err(VmError::InvalidState)?;
        self.natives
            .validate_map_lease_stamp(prepared.base_map_stamp)
            .map_err(VmError::InvalidState)?;
        let live_maps = if prepared.reset_execution {
            BTreeSet::new()
        } else {
            crate::interpreter::map_calls::live_map_leases(self.vm.fibers.values())
        };
        prepared.structured_state = self
            .natives
            .prepare_map_lease_cleanup(prepared.structured_state.as_deref(), &live_maps)
            .map_err(VmError::InvalidState)?;
        if let Some(structured_state) = &prepared.structured_state {
            self.natives
                .commit_structured_state(
                    structured_state,
                    prepared.base_column_stamp,
                    prepared.base_map_stamp,
                )
                .map_err(VmError::InvalidState)?;
        }
        self.vm.commit_runtime_state(prepared)?;
        self.refresh_draw_line_string();
        Ok(())
    }
}
