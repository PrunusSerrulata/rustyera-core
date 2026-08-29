use erabasic_bytecode::{Digest, HostImport, HostSnapshotCapability, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::structured::{ColumnIdentityStamp, StructuredExtension, StructuredScope};
use crate::{
    EraState, EraStateReport, FiberId, FiberState, FiberStatus, GenerationId, HostCallRequest,
    HostCallResult, HostReady, HostRequestId, HostWaitStability, HotReloadReport,
    ImmediateHostCall, ImmediateHostCallResult, NativeServiceRegistry, PlaceDescriptor,
    PreparedRuntimeState, RunBudget, SnapshotEligibility, Vm, VmConfig, VmDriveMode, VmError,
    VmHost, VmHostCompletion, VmHostRequest, VmPortDriveReport, VmPortEvent, VmPortStop,
    VmRestorePort, VmRuntimePort, VmRuntimeRead, VmRuntimeStatePort, VmRuntimeStateTransaction,
    VmSnapshot, VmValue, VmWaitRebind,
};
use std::collections::BTreeSet;

use crate::debug::DebugState;

/// Runtime-facing VM owner. It keeps native services beside the interpreter so the
/// caller-pumped runtime port never needs a callback parameter.
pub struct RuntimeVm {
    vm: Vm,
    natives: NativeServiceRegistry,
    pending_natives: Option<(
        NativeServiceRegistry,
        Option<ColumnIdentityStamp>,
        Option<crate::structured::MapLeaseStamp>,
    )>,
    candidate_base_column_stamp: CandidateColumnBase,
    candidate_base_array_stamp: Option<crate::state::array_leases::ArrayLeaseStamp>,
    line_columns: u32,
    pending_completion_events: Vec<VmPortEvent>,
}

/// Distinguish an unforked runtime from a fork whose artifact has no structured services.
#[derive(Clone, Copy)]
enum CandidateColumnBase {
    Unforked,
    Forked(
        Option<ColumnIdentityStamp>,
        Option<crate::structured::MapLeaseStamp>,
    ),
}

/// The immutable program index retained while a runtime obtains title entropy.
///
/// Consuming a [`RuntimeVm`] into this type releases game memory, fibers, scheduler
/// state, derived caches, Native services, layout and VM configuration without
/// rebuilding the program index when the title timeline starts.
pub struct RetainedProgramIndex {
    program: std::sync::Arc<crate::ProgramGeneration>,
}

impl RetainedProgramIndex {
    /// Identify the exact artifact whose immutable index is retained.
    #[must_use]
    pub fn artifact_id(&self) -> Digest {
        self.program.artifact.manifest.artifact_id
    }
}

/// Stable logical width used until a frontend reports its projection dimensions.
pub const DEFAULT_LINE_COLUMNS: u32 = 75;

/// Opaque candidate state prepared against one exact artifact generation.
/// It intentionally excludes fibers, frames and scheduler counters.
pub struct PreparedCandidateState {
    artifact_id: Digest,
    base_column_stamp: CandidateColumnBase,
    base_array_stamp: Option<crate::state::array_leases::ArrayLeaseStamp>,
    memory: crate::Memory,
    natives: NativeServiceRegistry,
}

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

    fn refresh_draw_line_string(&mut self) {
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

    fn waiting_host_import(
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

#[derive(Default)]
struct CaptureHost {
    first: Option<HostCallRequest>,
    overflow: Vec<HostCallRequest>,
}

impl CaptureHost {
    fn take(&mut self, request: HostRequestId) -> Option<HostCallRequest> {
        if self.first.as_ref().is_some_and(|item| item.id == request) {
            return self.first.take();
        }
        // Cooperative batches are uncommon and small. Keep the single-request hot
        // path allocation-free while retaining request-id lookup for arbitrary event
        // and completion order when several fibers reach the host together.
        let index = self.overflow.iter().position(|item| item.id == request)?;
        Some(self.overflow.remove(index))
    }

    fn is_empty(&self) -> bool {
        self.first.is_none() && self.overflow.is_empty()
    }
}

impl VmHost for CaptureHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if self.first.is_none() {
            self.first = Some(request);
        } else {
            self.overflow.push(request);
        }
        // The runtime will classify the real wait after it has staged its own state.
        HostCallResult::Deferred
    }
}

struct CapturingRuntimeHost<'a, H> {
    immediate: &'a mut H,
    captured: CaptureHost,
}

impl<H: VmHost> VmHost for CapturingRuntimeHost<'_, H> {
    fn path_memo_safe(&self, import: &erabasic_bytecode::RuntimeImport) -> bool {
        self.immediate.path_memo_safe(import)
    }

    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        self.immediate.call_immediate(request)
    }

    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        self.captured.call(request)
    }
}

impl RuntimeVm {
    fn deliver_captured_host(&mut self, request: HostCallRequest, events: &mut Vec<VmPortEvent>) {
        let definition = self
            .vm
            .fibers
            .get(&request.fiber)
            .and_then(|fiber| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request.id => {
                    self.waiting_host_import(request.fiber, wait)
                }
                _ => None,
            });
        if let Some(import) = definition.filter(|definition| definition.import == request.import) {
            events.push(VmPortEvent::HostCall(VmHostRequest {
                id: request.id,
                fiber: request.fiber,
                import,
                arguments: request.arguments,
                omitted_arguments: request.omitted_arguments,
                origin: request.origin,
            }));
        } else {
            // Missing owner/grant is an invariant failure, never a name fallback
            // or a silently dropped HostPending event.
            let failure = crate::ExecutionFailure::classified(
                crate::FaultCategory::InternalInvariant,
                crate::VmFaultCode::Host,
                "captured Host request lost its exact generation/owner authorization",
            );
            if let Ok((fiber, Some(fault))) = self.vm.fail_waiting_host(request.id, failure) {
                events.push(VmPortEvent::FiberFaulted(fiber, fault));
            } else {
                // Internal failures are never catchable. A missing wait or
                // an impossible recovery still reports the captured origin.
                let fault = crate::VmFault::from_origin(
                    request.fiber,
                    request.origin,
                    crate::ExecutionFailure::classified(
                        crate::FaultCategory::InternalInvariant,
                        crate::VmFaultCode::Host,
                        "captured Host request has no recoverable authorized owner",
                    ),
                );
                let published = match self.vm.fibers.remove(&request.fiber) {
                    Some(mut fiber) => {
                        let published = match self.vm.transition_fault(&mut fiber, fault) {
                            crate::interpreter::fault_hooks::FaultTransition::Published(fault) => {
                                *fault
                            }
                            crate::interpreter::fault_hooks::FaultTransition::HookStarted => {
                                unreachable!("internal invariant faults cannot start script hooks")
                            }
                        };
                        self.vm.fibers.insert(request.fiber, fiber);
                        published
                    }
                    None => fault,
                };
                events.push(VmPortEvent::FiberFaulted(request.fiber, published));
            }
        }
    }

    /// Drive with an optional immediate Host implementation. Unsupported calls still cross the
    /// ordinary caller-pumped port and retain all persistent wait/debug semantics.
    pub fn drive_with_immediate_host(
        &mut self,
        budget: RunBudget,
        mode: VmDriveMode,
        immediate: &mut impl VmHost,
    ) -> VmPortDriveReport {
        if !self.pending_completion_events.is_empty() {
            return VmPortDriveReport {
                stop: VmPortStop::Idle,
                instructions: 0,
                events: std::mem::take(&mut self.pending_completion_events),
            };
        }
        self.vm.retire_terminal_fibers();
        if matches!(mode, VmDriveMode::SelectedFiber(_)) {
            return VmPortDriveReport {
                stop: VmPortStop::DebugStopped,
                instructions: 0,
                events: Vec::new(),
            };
        }
        let mut host = CapturingRuntimeHost {
            immediate,
            captured: CaptureHost::default(),
        };
        let report = self.vm.run_slice(&mut host, &mut self.natives, budget);
        let mut events = Vec::new();
        for event in report.events {
            match event {
                crate::VmEvent::Diagnostic {
                    fiber,
                    code,
                    message,
                    origin,
                    notification,
                } => events.push(VmPortEvent::Diagnostic {
                    fiber,
                    code,
                    message,
                    origin,
                    notification,
                }),
                crate::VmEvent::HostPending { request, .. } => {
                    if let Some(request) = host.captured.take(request) {
                        self.deliver_captured_host(request, &mut events);
                    }
                }
                crate::VmEvent::FiberYielded { fiber } => {
                    events.push(VmPortEvent::FiberYielded(fiber));
                }
                crate::VmEvent::FiberCompleted { fiber, value } => {
                    events.push(VmPortEvent::FiberCompleted(fiber, value));
                }
                crate::VmEvent::FiberFaulted { fiber, fault } => {
                    events.push(VmPortEvent::FiberFaulted(fiber, fault));
                }
                crate::VmEvent::DebugStopped(stop) => {
                    events.push(VmPortEvent::DebugStopped(stop));
                }
            }
        }
        debug_assert!(
            host.captured.is_empty(),
            "captured host request lost its VM event"
        );
        let debug_stopped = events
            .iter()
            .any(|event| matches!(event, VmPortEvent::DebugStopped(_)));
        VmPortDriveReport {
            stop: if debug_stopped {
                VmPortStop::DebugStopped
            } else if matches!(report.stop, crate::VmRunStop::BudgetExhausted) {
                VmPortStop::BudgetExhausted
            } else {
                VmPortStop::Idle
            },
            instructions: report.instructions,
            events,
        }
    }
}

pub struct PreparedHostCompletion {
    generation: GenerationId,
    request: HostRequestId,
    completion: VmHostCompletion,
}

impl VmRuntimePort for RuntimeVm {
    type PreparedCompletion = PreparedHostCompletion;

    fn artifact_id(&self) -> Digest {
        self.vm.artifact_id()
    }

    fn current_generation(&self) -> GenerationId {
        self.vm.current_generation()
    }

    fn spawn_entry(
        &mut self,
        function: SymbolKey,
        arguments: Vec<VmValue>,
    ) -> Result<FiberId, VmError> {
        let fiber = self.vm.spawn_entry(function, arguments)?;
        // Runtime roots are dispatched sequentially by the caller-pumped system
        // controller. The newest root owns the input wait that an exact snapshot
        // must resume; older completed roots must not remain the primary fiber.
        self.vm.set_primary_fiber(fiber)?;
        Ok(fiber)
    }

    fn fiber_status(&self, fiber: FiberId) -> Option<FiberStatus> {
        self.vm.fiber_status(fiber)
    }

    fn drive(&mut self, budget: RunBudget, mode: VmDriveMode) -> VmPortDriveReport {
        self.drive_with_immediate_host(budget, mode, &mut CaptureHost::default())
    }

    fn retire_terminal_fibers(&mut self) -> usize {
        if self.has_pending_events() {
            // The caller has not observed the terminal state yet. Retiring it can
            // otherwise discard the primary identity before event dispatch.
            0
        } else {
            self.vm.retire_terminal_fibers()
        }
    }

    fn validate_host_completion(
        &self,
        request: HostRequestId,
        completion: VmHostCompletion,
    ) -> Result<Self::PreparedCompletion, VmError> {
        let (fiber_id, fiber, wait) = self
            .vm
            .fibers
            .iter()
            .find_map(|(id, fiber)| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request => {
                    Some((*id, fiber, wait))
                }
                _ => None,
            })
            .ok_or(VmError::StaleHostRequest(request))?;
        let import = self
            .waiting_host_import(fiber_id, wait)
            .ok_or_else(|| VmError::InvalidState("waiting host import is missing".into()))?;
        match &completion {
            VmHostCompletion::Ready(ready) => {
                validate_ready(
                    &self.vm,
                    fiber_id,
                    fiber,
                    &import.import.name,
                    wait.result,
                    ready,
                )?;
            }
            VmHostCompletion::ReturnCurrent(_) => {
                if wait.form_scope.is_some() {
                    return Err(VmError::InvalidState(
                        "direct Host expression cannot return its owner frame".into(),
                    ));
                }
                if fiber.frames.len() <= 1 {
                    return Err(VmError::InvalidState(
                        "cannot return the root frame through a host completion".into(),
                    ));
                }
            }
            VmHostCompletion::Pending { stability, .. } => {
                // A caller-pumped runtime necessarily unwinds every external call
                // before it can ask the frontend for a service result. `may_suspend`
                // describes the EraBasic-visible operation, not this transport wait.
                if *stability == HostWaitStability::StableInput
                    && import.snapshot_capability != HostSnapshotCapability::StableWait
                {
                    return Err(VmError::InvalidState(
                        "host wait exceeds the import snapshot capability".into(),
                    ));
                }
            }
            VmHostCompletion::Error(_) => {}
        }
        Ok(PreparedHostCompletion {
            generation: self.vm.current_generation(),
            request,
            completion,
        })
    }

    fn commit_host_completion(
        &mut self,
        completion: Self::PreparedCompletion,
    ) -> Result<FiberId, VmError> {
        if completion.generation != self.vm.current_generation() {
            return Err(VmError::StaleHostRequest(completion.request));
        }
        match completion.completion {
            VmHostCompletion::Ready(ready) => self.vm.resume_host(completion.request, ready),
            VmHostCompletion::ReturnCurrent(value) => {
                let fiber = self
                    .vm
                    .return_current_from_host(completion.request, value.as_ref())?;
                self.natives
                    .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                        self.vm.fibers.values(),
                    ))
                    .map_err(|error| VmError::InvalidState(error.to_string()))?;
                if let Some(FiberStatus::Completed(value)) = self.vm.fiber_status(fiber) {
                    self.pending_completion_events
                        .push(VmPortEvent::FiberCompleted(fiber, value));
                }
                Ok(fiber)
            }
            VmHostCompletion::Pending {
                stability,
                rebind_payload,
            } => {
                let (fiber_id, wait) = self
                    .vm
                    .fibers
                    .iter_mut()
                    .find_map(|(id, fiber)| match &mut fiber.state {
                        FiberState::WaitingHost(wait) if wait.request == completion.request => {
                            Some((*id, wait))
                        }
                        _ => None,
                    })
                    .ok_or(VmError::StaleHostRequest(completion.request))?;
                wait.stability = stability;
                wait.rebind_payload = rebind_payload;
                Ok(fiber_id)
            }
            VmHostCompletion::Error(failure) => {
                let (fiber, fault) = self.vm.fail_waiting_host(completion.request, failure)?;
                self.natives
                    .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                        self.vm.fibers.values(),
                    ))
                    .map_err(|error| VmError::InvalidState(error.to_string()))?;
                if let Some(fault) = fault {
                    self.pending_completion_events
                        .push(VmPortEvent::FiberFaulted(fiber, fault));
                }
                Ok(fiber)
            }
        }
    }

    fn cancel_fiber(&mut self, fiber: FiberId) -> Result<(), VmError> {
        self.vm.cancel_fiber(fiber)?;
        if let Some(FiberStatus::Faulted(fault)) = self.vm.fiber_status(fiber) {
            self.pending_completion_events
                .push(VmPortEvent::FiberFaulted(fiber, fault));
        }
        self.natives
            .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                self.vm.fibers.values(),
            ))
            .map_err(|error| VmError::InvalidState(error.to_string()))
    }

    fn export_era_state(&self) -> EraState {
        self.vm.export_era_state()
    }

    fn restore_era_state(&mut self, state: &EraState) -> Result<EraStateReport, VmError> {
        let report = self.vm.reset_with_era_state(state)?;
        self.refresh_draw_line_string();
        Ok(report)
    }

    fn snapshot_eligibility(&self) -> SnapshotEligibility {
        if !self.pending_completion_events.is_empty() {
            return SnapshotEligibility::Ineligible(vec![
                crate::SnapshotBlocker::PendingCompletionEvents,
            ]);
        }
        self.vm.snapshot_eligibility(&self.natives)
    }

    fn snapshot(&self) -> Result<VmSnapshot, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.snapshot(&self.natives)
    }

    fn encode_snapshot(&self) -> Result<Vec<u8>, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.encode_snapshot(&self.natives)
    }

    fn encode_unrestricted_snapshot(&self) -> Result<Vec<u8>, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.encode_unrestricted_snapshot(&self.natives)
    }

    fn prepare_hot_reload(&mut self, target: ValidatedArtifact) -> Result<(), VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::HotReload(
                "host completion events have not been delivered".into(),
            ));
        }
        let base_column_stamp = self
            .natives
            .column_identity_stamp()
            .map_err(VmError::Snapshot)?;
        let migrated = self
            .natives
            .migrated_for_artifact(target.artifact())
            .map_err(VmError::Snapshot)?;
        self.vm.prepare_hot_reload_artifact(target)?;
        let map_stamp = self.natives.map_lease_stamp().map_err(VmError::Snapshot)?;
        self.pending_natives = Some((migrated, base_column_stamp, map_stamp));
        Ok(())
    }

    fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError> {
        let (_, base_column_stamp, base_map_stamp) = self
            .pending_natives
            .as_ref()
            .ok_or_else(|| VmError::InvalidState("prepared native migration is missing".into()))?;
        self.natives
            .validate_column_identity_stamp(*base_column_stamp)
            .map_err(VmError::InvalidState)?;
        self.natives
            .validate_map_lease_stamp(*base_map_stamp)
            .map_err(VmError::InvalidState)?;
        let report = self.vm.commit_hot_reload()?;
        let (natives, _, _) = self
            .pending_natives
            .take()
            .expect("validated native migration remains available");
        self.natives = natives;
        self.refresh_draw_line_string();
        Ok(report)
    }
}

impl crate::VmDebugInspect for RuntimeVm {
    fn stop_token(&self) -> Option<crate::VmStopToken> {
        self.vm.stop_token()
    }

    fn fibers(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugFiber>, VmError> {
        self.vm.fibers(stop, cursor, limit)
    }

    fn call_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
    ) -> Result<Vec<crate::VmDebugFrame>, VmError> {
        self.vm.call_stack(stop, fiber)
    }

    fn operand_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        frame: crate::FrameId,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugOperand>, VmError> {
        self.vm.operand_stack(stop, fiber, frame, cursor, limit)
    }

    fn variables(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugVariable>, VmError> {
        self.vm.variables(stop, cursor, limit)
    }

    fn read_variable(
        &self,
        stop: crate::VmStopToken,
        target: &crate::VmDebugVariableRef,
    ) -> Result<crate::VmDebugVariable, VmError> {
        crate::VmDebugInspect::read_variable(&self.vm, stop, target)
    }
}

impl crate::VmDebugControl for RuntimeVm {
    fn request_pause(&mut self) -> Result<crate::VmDebugStop, VmError> {
        self.vm.request_pause()
    }

    fn continue_execution(&mut self, stop: crate::VmStopToken) -> Result<(), VmError> {
        self.vm.continue_execution(stop)
    }

    fn step(
        &mut self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        kind: crate::VmStepKind,
    ) -> Result<(), VmError> {
        self.vm.step(stop, fiber, kind)
    }

    fn write_variables(
        &mut self,
        stop: crate::VmStopToken,
        writes: &[crate::VmDebugVariableWrite],
    ) -> Result<Vec<crate::VmDebugVariable>, VmError> {
        self.vm.write_variables(stop, writes)
    }

    fn update_breakpoints(
        &mut self,
        breakpoints: &[crate::VmBreakpoint],
        remove: &[u64],
    ) -> Result<Vec<crate::VmResolvedBreakpoint>, VmError> {
        self.vm.update_breakpoints(breakpoints, remove)
    }
}

pub struct PreparedVmRestore {
    runtime: RuntimeVm,
    waits: Vec<VmWaitRebind>,
}

#[derive(Default)]
struct RestoreCaptureHost {
    waits: Vec<VmWaitRebind>,
}

impl VmHost for RestoreCaptureHost {
    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Error("restore capture host cannot execute calls".into())
    }

    fn rebind_snapshot(&mut self, requests: &[crate::HostRebindRequest]) -> Result<(), String> {
        self.waits = requests
            .iter()
            .map(|request| VmWaitRebind {
                request: request.id,
                fiber: request.fiber,
                import: request.import.clone(),
                payload: request.payload.clone(),
            })
            .collect();
        Ok(())
    }
}

impl VmRestorePort for RuntimeVm {
    type PreparedRestore = PreparedVmRestore;

    fn prepare_restore(
        artifact: ValidatedArtifact,
        config: VmConfig,
        snapshot: VmSnapshot,
    ) -> Result<Self::PreparedRestore, VmError> {
        let mut natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        let mut host = RestoreCaptureHost::default();
        let vm = Vm::restore_snapshot(artifact, config, snapshot, &mut host, &mut natives)?;
        // Preserve the captured calculated value until the runtime supplies its
        // current frontend projection after committing the restore.
        let runtime = Self {
            vm,
            natives,
            pending_natives: None,
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
        };
        Ok(PreparedVmRestore {
            runtime,
            waits: host.waits,
        })
    }

    fn restore_waits(plan: &Self::PreparedRestore) -> &[VmWaitRebind] {
        &plan.waits
    }

    fn commit_restore(plan: Self::PreparedRestore) -> Result<Self, VmError> {
        Ok(plan.runtime)
    }
}

fn validate_ready(
    vm: &Vm,
    fiber_id: FiberId,
    fiber: &crate::Fiber,
    operation: &str,
    expected: Option<erabasic_bytecode::BytecodeType>,
    ready: &HostReady,
) -> Result<(), VmError> {
    let actual = ready.value.as_ref().map(VmValue::value_type);
    if expected != actual {
        return Err(VmError::InvalidArguments(format!(
            "{operation} host completion result type differs: expected {expected:?}, found {actual:?}"
        )));
    }
    for write in &ready.writes {
        if write.target.fiber.is_some_and(|owner| owner != fiber_id) {
            return Err(VmError::InvalidState(
                "host write belongs to another fiber".into(),
            ));
        }
        // Public Host descriptors never carry the VM-private backing capability.
        // A legitimate REF write names its live formal and resolves the binding in VM.
        if write.target.backing.is_some() {
            return Err(VmError::InvalidState(
                "Host cannot inject an array backing identity".into(),
            ));
        }
        let (_, definition) = vm.place_definition(fiber, &write.target).map_err(|error| {
            VmError::ScriptFailure(crate::ExecutionFailure::classified(
                crate::FaultCategory::HostContract,
                crate::VmFaultCode::Host,
                error.to_string(),
            ))
        })?;
        // Host completions are constructed by the trusted runtime and must update
        // reference pseudo-variables such as immutable-to-script ISTIMEOUT.
        if definition.value_type != write.value.value_type() {
            return Err(VmError::InvalidArguments(
                "host write value type differs".into(),
            ));
        }
        let _ = vm.read_place(fiber, &write.target).map_err(|error| {
            VmError::ScriptFailure(crate::ExecutionFailure::classified(
                crate::FaultCategory::HostContract,
                crate::VmFaultCode::Host,
                error.to_string(),
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_bytecode::RuntimeImport;

    fn host_request(id: u64) -> HostCallRequest {
        HostCallRequest {
            id: HostRequestId(id),
            fiber: FiberId(id.saturating_add(100)),
            import: RuntimeImport {
                key: SymbolKey([u8::try_from(id).unwrap_or(u8::MAX); 16]),
                namespace: "test".into(),
                name: format!("HOST_{id}"),
                abi_version: 1,
                parameters: Vec::new(),
                result: None,
            },
            arguments: vec![VmValue::Integer(i64::try_from(id).unwrap_or(i64::MAX))],
            omitted_arguments: Vec::new(),
            origin: crate::VmExecutionOrigin {
                generation: GenerationId(1),
                function: SymbolKey([0; 16]),
                function_name: "TEST".into(),
                instruction: u32::try_from(id).unwrap_or(u32::MAX),
                command: format!("HOST_{id}"),
                source: None,
            },
        }
    }

    #[test]
    fn capture_host_keeps_the_single_request_inline() {
        let mut host = CaptureHost::default();
        let request = host_request(1);
        assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
        assert!(host.overflow.is_empty());
        assert_eq!(host.take(request.id), Some(request));
        assert!(host.is_empty());
    }

    #[test]
    fn capture_host_preserves_multiple_fibers_without_fifo_assumptions() {
        let mut host = CaptureHost::default();
        let requests = [host_request(1), host_request(2), host_request(3)];
        for request in &requests {
            assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
        }
        assert_eq!(host.take(requests[1].id), Some(requests[1].clone()));
        assert_eq!(host.take(requests[0].id), Some(requests[0].clone()));
        assert_eq!(host.take(requests[2].id), Some(requests[2].clone()));
        assert!(host.is_empty());
    }
}
