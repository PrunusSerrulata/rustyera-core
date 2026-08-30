// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

pub(in crate::session) struct OwnedReplacementTransaction {
    old_vm: Option<RuntimeVm>,
    candidate_vm: Option<RuntimeVm>,
    old_sql: Option<SqlRuntimeState>,
    candidate_sql: Option<SqlRuntimeState>,
    old_controller: SystemController,
    old_phase: RuntimePhase,
    old_revision: u64,
    old_epoch: SessionEpoch,
    old_operations: PendingOperations,
    old_device_input: crate::device_input::DeviceInput,
    old_input_notice_sites: BTreeSet<(String, u64, erabasic_bytecode::SymbolKey, u32)>,
    old_command_intents: BTreeMap<InteractionToken, VmValue>,
    old_reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    old_next_interaction_id: u64,
    old_accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    old_accepted_debug_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    old_queued_input: VecDeque<QueuedInput>,
    old_active_input_source: Option<InputSource>,
    old_input_controller: InputController,
    old_system_menu_host_request: Option<erabasic_vm::HostRequestId>,
    old_save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    old_presentation: PresentationModel,
    old_pending_presentation_update: bool,
    old_last_projection_state: Option<ProjectionState>,
    old_project_snapshot: Option<NormalizedProjectSnapshot>,
    old_outbound: VecDeque<Vec<u8>>,
    old_outbound_journal: BTreeMap<u64, Vec<u8>>,
    old_outbound_journal_bytes: u64,
    old_effect_journal: BTreeMap<u64, EffectEvent>,
    old_outbound_sequence: u64,
    old_next_message_id: u64,
    old_next_effect_id: u64,
    old_system_menu: SystemMenuState,
    old_undo_checkpoint: Option<UndoCheckpoint>,
    old_undo_replay: Option<UndoReplay>,
    old_undo_token: Option<InteractionToken>,
    old_input_replay: InputReplayHistory,
    old_retained_title_program: Option<RetainedProgramIndex>,
    old_active_debug_grant: Option<ActiveDebugGrant>,
    old_next_debug_grant_id: u64,
    old_debug_outbound_sequence: u64,
    old_load_slot_paths: Vec<String>,
    old_occupied_slot_paths: BTreeSet<String>,
    old_slot_change_tokens: BTreeMap<String, String>,
    old_slot_labels: BTreeMap<String, String>,
    old_invalid_slot_paths: BTreeSet<String>,
    old_system_menu_page: u32,
}

impl OwnedReplacementTransaction {
    pub(in crate::session) fn capture(
        session: &mut RuntimeSession,
        candidate_vm: RuntimeVm,
        replacement_sql: SqlRuntimeState,
    ) -> Self {
        Self {
            old_vm: session.vm.take(),
            candidate_vm: Some(candidate_vm),
            old_sql: None,
            candidate_sql: Some(replacement_sql),
            old_controller: session.controller.clone(),
            old_phase: session.phase,
            old_revision: session.revision,
            old_epoch: session.epoch,
            old_operations: session.operations.clone(),
            old_device_input: session.device_input.clone(),
            old_input_notice_sites: session.input_notice_sites.clone(),
            old_command_intents: session.command_intents.clone(),
            old_reusable_system_intents: session.reusable_system_intents.clone(),
            old_next_interaction_id: session.next_interaction_id,
            old_accepted_message_ids: session.accepted_message_ids.clone(),
            old_accepted_debug_message_ids: session.accepted_debug_message_ids.clone(),
            old_queued_input: session.queued_input.clone(),
            old_active_input_source: session.active_input_source.clone(),
            old_input_controller: session.input_controller.clone(),
            old_system_menu_host_request: session.system_menu_host_request,
            old_save_extensions: session.save_extensions.clone(),
            old_presentation: session.presentation.clone(),
            old_pending_presentation_update: session.pending_presentation_update,
            old_last_projection_state: session.last_projection_state.clone(),
            old_project_snapshot: session.project_snapshot.clone(),
            old_outbound: session.outbound.clone(),
            old_outbound_journal: session.outbound_journal.clone(),
            old_outbound_journal_bytes: session.outbound_journal_bytes,
            old_effect_journal: session.effect_journal.clone(),
            old_outbound_sequence: session.outbound_sequence,
            old_next_message_id: session.next_message_id,
            old_next_effect_id: session.next_effect_id,
            old_system_menu: session.system_menu,
            old_undo_checkpoint: session.undo_checkpoint.clone(),
            old_undo_replay: session.undo_replay.as_ref().map(|replay| UndoReplay {
                remaining: replay.remaining.clone(),
                queued_repeats: replay.queued_repeats,
            }),
            old_undo_token: session.undo_token,
            old_input_replay: session.input_replay.clone(),
            old_retained_title_program: session.retained_title_program.take(),
            old_active_debug_grant: session.active_debug_grant.clone(),
            old_next_debug_grant_id: session.next_debug_grant_id,
            old_debug_outbound_sequence: session.debug_outbound_sequence,
            old_load_slot_paths: session.load_slot_paths.clone(),
            old_occupied_slot_paths: session.occupied_slot_paths.clone(),
            old_slot_change_tokens: session.slot_change_tokens.clone(),
            old_slot_labels: session.slot_labels.clone(),
            old_invalid_slot_paths: session.invalid_slot_paths.clone(),
            old_system_menu_page: session.system_menu_page,
        }
    }

    pub(in crate::session) fn candidate_vm(&self) -> &RuntimeVm {
        self.candidate_vm
            .as_ref()
            .expect("owned replacement retains its VM candidate until publication")
    }

    pub(in crate::session) fn candidate_vm_mut(&mut self) -> &mut RuntimeVm {
        self.candidate_vm
            .as_mut()
            .expect("owned replacement retains its VM candidate until publication")
    }

    pub(in crate::session) fn publish(&mut self, session: &mut RuntimeSession) {
        let candidate = self
            .candidate_sql
            .take()
            .expect("owned replacement retains its SQL candidate until publication");
        self.old_sql = Some(std::mem::replace(&mut session.sql, candidate));
        session.vm = self.candidate_vm.take();
    }

    pub(in crate::session) fn old_sql_cleanup(
        &self,
    ) -> (
        era_runtime_protocol::SqlProviderHandleV1,
        Vec<era_runtime_protocol::SqlConnectionHandleV1>,
    ) {
        let old = self
            .old_sql
            .as_ref()
            .expect("owned replacement published SQL before cleanup collection");
        (
            old.provider(),
            old.connections()
                .map(|(_, connection)| connection.handle)
                .collect(),
        )
    }

    pub(in crate::session) fn rollback(mut self, session: &mut RuntimeSession) {
        session.vm = self.old_vm.take();
        let candidate_sql = if let Some(old_sql) = self.old_sql.take() {
            std::mem::replace(&mut session.sql, old_sql)
        } else {
            self.candidate_sql
                .take()
                .expect("unpublished owned replacement retains its SQL candidate")
        };
        let candidate_provider = candidate_sql.provider();
        let candidate_handles = candidate_sql.cleanup_handles();
        session.controller = self.old_controller;
        session.phase = self.old_phase;
        session.revision = self.old_revision;
        session.epoch = self.old_epoch;
        session.operations = self.old_operations;
        session.device_input = self.old_device_input;
        session.input_notice_sites = self.old_input_notice_sites;
        session.command_intents = self.old_command_intents;
        session.reusable_system_intents = self.old_reusable_system_intents;
        session.next_interaction_id = self.old_next_interaction_id;
        session.accepted_message_ids = self.old_accepted_message_ids;
        session.accepted_debug_message_ids = self.old_accepted_debug_message_ids;
        session.queued_input = self.old_queued_input;
        session.active_input_source = self.old_active_input_source;
        session.input_controller = self.old_input_controller;
        session.system_menu_host_request = self.old_system_menu_host_request;
        session.save_extensions = self.old_save_extensions;
        session.presentation = self.old_presentation;
        session.pending_presentation_update = self.old_pending_presentation_update;
        session.last_projection_state = self.old_last_projection_state;
        session.project_snapshot = self.old_project_snapshot;
        session.outbound = self.old_outbound;
        session.outbound_journal = self.old_outbound_journal;
        session.outbound_journal_bytes = self.old_outbound_journal_bytes;
        session.effect_journal = self.old_effect_journal;
        session.outbound_sequence = self.old_outbound_sequence;
        session.next_message_id = self.old_next_message_id;
        session.next_effect_id = self.old_next_effect_id;
        session.system_menu = self.old_system_menu;
        session.undo_checkpoint = self.old_undo_checkpoint;
        session.undo_replay = self.old_undo_replay;
        session.undo_token = self.old_undo_token;
        session.input_replay = self.old_input_replay;
        session.retained_title_program = self.old_retained_title_program;
        session.active_debug_grant = self.old_active_debug_grant;
        session.next_debug_grant_id = self.old_next_debug_grant_id;
        session.debug_outbound_sequence = self.old_debug_outbound_sequence;
        session.load_slot_paths = self.old_load_slot_paths;
        session.occupied_slot_paths = self.old_occupied_slot_paths;
        session.slot_change_tokens = self.old_slot_change_tokens;
        session.slot_labels = self.old_slot_labels;
        session.invalid_slot_paths = self.old_invalid_slot_paths;
        session.system_menu_page = self.old_system_menu_page;
        for handle in candidate_handles {
            session.retain_sql_cleanup(candidate_provider, handle);
        }
    }
}

impl RuntimeSession {
    pub(in crate::session) fn prepare_owned_vm_candidate(
        mut candidate: RuntimeVm,
        input: OwnedVmCandidateInput,
    ) -> Result<PreparedOwnedVm, RuntimeError> {
        let OwnedVmCandidateInput {
            state,
            description,
            opaque_extensions,
            structured_extensions,
            owned,
            last_load,
        } = input;
        let transaction = match last_load {
            OwnedLastLoad::None => VmRuntimeStateTransaction::RestoreOrdinary(Box::new(state)),
            OwnedLastLoad::Slot(slot) => VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad {
                state: Box::new(state),
                slot,
                text: description,
            },
        };
        let (ordinary, _) = candidate
            .prepare_runtime_state_with_extensions(
                transaction,
                StructuredScope::Ordinary,
                &structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        candidate
            .commit_runtime_state(ordinary)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let reset_global = candidate
            .prepare_runtime_state(VmRuntimeStateTransaction::ResetGlobalData)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        candidate
            .commit_runtime_state(reset_global)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let (global, _) = candidate
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::OverlayGlobal(Box::new(owned.global_state)),
                StructuredScope::Global,
                &owned.global_structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        candidate
            .commit_runtime_state(global)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        candidate
            .restore_random_state(&owned.sfmt_state)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok(PreparedOwnedVm {
            vm: candidate,
            opaque_extensions: merge_opaque_extensions(
                &opaque_extensions,
                owned.global_opaque_extensions,
            ),
            sql: owned
                .databases
                .into_iter()
                .map(|database| crate::runtime_snapshot::SqlConnectionSnapshot {
                    logical_name: database.logical_name,
                    identity: database.identity,
                    durable_revision: database.exact_durable_revision,
                })
                .collect(),
        })
    }

    pub(in super::super) fn resume_storage_host_value(
        &mut self,
        request: erabasic_vm::HostRequestId,
        value: VmValue,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    pub(in super::super) fn file_list_writes(
        &self,
        target: Option<PlaceDescriptor>,
        values: &[String],
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("file list completion has no VM".into()))?;
        let base = target.or_else(|| global_place_at(vm, "RESULTS", 0));
        let Some(base) = base else {
            return Ok(Vec::new());
        };
        let maximum = vm
            .vm()
            .artifact()
            .globals
            .iter()
            .find(|definition| definition.key == base.variable)
            .and_then(|definition| definition.dimensions.first())
            .and_then(|value| usize::try_from(*value).ok())
            .unwrap_or(0);
        Ok(values
            .iter()
            .take(maximum)
            .enumerate()
            .map(|(index, value)| {
                let mut target = base.clone();
                if let Some(last) = target.indices.last_mut() {
                    *last = u64::try_from(index).unwrap_or(u64::MAX);
                } else {
                    target
                        .indices
                        .push(u64::try_from(index).unwrap_or(u64::MAX));
                }
                HostWrite {
                    target,
                    value: VmValue::String(value.clone()),
                }
            })
            .collect())
    }

    pub(in super::super) fn result_write(
        &self,
        value: i64,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        Ok(global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(value),
                }]
            })
            .unwrap_or_default())
    }

    pub(in super::super) fn check_data_writes(
        &self,
        description: &str,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("save check completion has no VM".into()))?;
        let mut writes = Vec::new();
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(description.to_owned()),
            });
        }
        Ok(writes)
    }

    pub(in super::super) fn complete_global_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
        storage_path: &str,
    ) -> Result<(), RuntimeError> {
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::ExternalDataLoad {
            storage_path: storage_path.to_owned(),
            payload_digest: crate::input_replay::digest_hex(bytes),
            data_type: crate::input_replay::ReplayExternalDataType::Global,
        })?;
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("global load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Global,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid global save: {error}")))?;
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::OverlayGlobal(Box::new(decoded.state)),
                StructuredScope::Global,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let writes = global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        // Decode, state preparation and host validation must not remove the live VM.
        // A corrupt or incompatible GLOBAL load leaves both state and replay intact.
        let completion = vm
            .validate_host_completion(
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let vm = self
            .vm
            .as_mut()
            .expect("global load was prepared against the live VM");
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_host_completion(completion)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions =
            merge_opaque_extensions(&self.save_extensions, decoded.opaque_extensions);
        self.set_phase(RuntimePhase::Running)?;
        self.install_input_replay(replay_origin);
        Ok(())
    }

    pub(in super::super) fn complete_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        host_request: Option<erabasic_vm::HostRequestId>,
    ) -> Result<(), RuntimeError> {
        let (decoded, owned_profile) = {
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
            (
                decode_scoped_save(
                    bytes,
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                ),
                vm.vm().artifact().manifest.compatibility.profile
                    == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            )
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) if owned_profile => {
                return self.finish_owned_load_failure(
                    host_request,
                    &format!("invalid owned save: {error}"),
                );
            }
            Err(error) => {
                return Err(RuntimeError::Internal(format!(
                    "invalid ordinary save: {error}"
                )));
            }
        };
        self.complete_decoded_ordinary_load(slot, bytes, decoded, host_request)
    }

    pub(in super::super) fn complete_decoded_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        decoded: DecodedEraSave,
        host_request: Option<erabasic_vm::HostRequestId>,
    ) -> Result<(), RuntimeError> {
        let owned = decoded.owned_state.is_some();
        let load = match self.prepare_decoded_ordinary_load(slot, decoded, host_request) {
            Ok(load) => load,
            Err(error) if owned => {
                return self.finish_owned_load_failure(host_request, &error.to_string());
            }
            Err(error) => return Err(error),
        };
        self.complete_prepared_ordinary_load(slot, bytes, load)
    }

    pub(in super::super) fn prepare_decoded_ordinary_load(
        &self,
        slot: u32,
        decoded: DecodedEraSave,
        host_request: Option<erabasic_vm::HostRequestId>,
    ) -> Result<PreparedOrdinaryLoad, RuntimeError> {
        let DecodedEraSave {
            state,
            description,
            opaque_extensions,
            structured_extensions,
            owned_state,
        } = decoded;
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
        let Some(owned) = owned_state else {
            let (prepared, _) = vm
                .prepare_runtime_state_with_extensions(
                    VmRuntimeStateTransaction::RestoreOrdinaryWithLastLoad {
                        state: Box::new(state),
                        slot: i64::from(slot),
                        text: description,
                    },
                    StructuredScope::Ordinary,
                    &structured_extensions,
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return Ok(PreparedOrdinaryLoad {
                vm: PreparedOrdinaryVm::Traditional(Box::new(prepared)),
                opaque_extensions,
                sql: None,
                host_request,
            });
        };

        let candidate = vm
            .fork_for_state_replacement()
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let prepared = Self::prepare_owned_vm_candidate(
            candidate,
            OwnedVmCandidateInput {
                state,
                description,
                opaque_extensions,
                structured_extensions,
                owned,
                last_load: OwnedLastLoad::Slot(i64::from(slot)),
            },
        )?;
        Ok(PreparedOrdinaryLoad {
            vm: PreparedOrdinaryVm::Owned(Box::new(prepared.vm)),
            opaque_extensions: prepared.opaque_extensions,
            sql: Some(prepared.sql),
            host_request,
        })
    }

    pub(in super::super) fn complete_prepared_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        mut load: PreparedOrdinaryLoad,
    ) -> Result<(), RuntimeError> {
        if let Some(connections) = load.sql.take() {
            if let Err(blocker) = self.sql.snapshot() {
                return self.finish_owned_load_failure(
                    load.host_request,
                    owned_load_sql_blocker_message(blocker),
                );
            }
            return self.begin_owned_save_sql_restore(slot, bytes.to_vec(), load, connections);
        }
        self.commit_prepared_ordinary_load(slot, bytes, load, None)
    }

    pub(in super::super) fn complete_owned_sql_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        load: PreparedOrdinaryLoad,
        sql: SqlRuntimeState,
    ) -> Result<(), RuntimeError> {
        self.commit_prepared_ordinary_load(slot, bytes, load, Some(sql))
    }

    // The replacement transaction remains linear so every rollback edge is visible beside the
    // state change it protects; splitting it would hide the publication boundary across helpers.
    #[allow(clippy::too_many_lines)]
    fn commit_prepared_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
        load: PreparedOrdinaryLoad,
        replacement_sql: Option<SqlRuntimeState>,
    ) -> Result<(), RuntimeError> {
        let mut replacement_sql = replacement_sql;
        let establish_undo = self.undo_replay.is_none();
        let replay_details = if let Some(replay) = &self.undo_replay {
            ReplayOriginDetails::InputUndo {
                checkpoint_slot: slot,
                save_digest: crate::input_replay::digest_hex(bytes),
                retained_input_count: replay.remaining.len(),
            }
        } else {
            ReplayOriginDetails::OrdinarySave {
                slot,
                storage_path: save_slot_path(slot),
                payload_digest: crate::input_replay::digest_hex(bytes),
            }
        };
        let replay_origin = self.prepare_input_replay(replay_details)?;
        let random_before_load = establish_undo
            .then(|| {
                self.vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?
                    .export_random_state()
                    .map_err(|error| RuntimeError::Internal(error.to_string()))
            })
            .transpose()?;
        let mut owned_transaction = None;
        let mut traditional_vm = None;
        let mut sql_cleanup = None;
        match load.vm {
            PreparedOrdinaryVm::Traditional(prepared) => {
                let mut vm = self
                    .vm
                    .take()
                    .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
                if let Err(error) = vm.commit_runtime_state(*prepared) {
                    self.vm = Some(vm);
                    return Err(RuntimeError::Internal(error.to_string()));
                }
                traditional_vm = Some(vm);
            }
            PreparedOrdinaryVm::Owned(candidate) => {
                self.vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?
                    .validate_state_replacement(&candidate)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                let replacement = replacement_sql.take().ok_or_else(|| {
                    RuntimeError::Internal("owned load has no exact SQL candidate".into())
                })?;
                let transaction =
                    OwnedReplacementTransaction::capture(self, *candidate, replacement);
                owned_transaction = Some(transaction);
            }
        }
        if owned_transaction.is_none() {
            sql_cleanup = replacement_sql.take().map(|replacement| {
                let previous = std::mem::replace(&mut self.sql, replacement);
                (
                    previous.provider(),
                    previous
                        .connections()
                        .map(|(_, connection)| connection.handle)
                        .collect::<Vec<_>>(),
                )
            });
        }
        self.system_menu_host_request = None;
        self.save_extensions = load.opaque_extensions;
        self.advance_epoch();
        self.queued_input.clear();
        self.active_input_source = None;
        self.input_controller = if self.undo_replay.is_some() {
            self.undo_checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.input_controller.clone())
                .unwrap_or_default()
        } else {
            // Bare saves do not serialize process-owned input controls. A LOAD
            // in the same process retains the live switch and single pending slot.
            self.input_controller.clone()
        };
        self.controller.clear();
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        let vm = if let Some(transaction) = owned_transaction.as_mut() {
            transaction.candidate_vm_mut()
        } else {
            traditional_vm
                .as_mut()
                .expect("traditional replacement retains its VM until publication")
        };
        self.controller.prepare_load_sequence(vm.vm().artifact());
        let flow = if self.controller.is_complete() {
            self.continue_system_flow(vm)
        } else {
            self.spawn_next_event(vm)
        };
        if let Err(error) = flow {
            if let Some(transaction) = owned_transaction.take() {
                transaction.rollback(self);
            } else {
                self.vm = traditional_vm.take();
            }
            return Err(error);
        }
        if let Err(error) = self.set_phase(RuntimePhase::Running) {
            if let Some(transaction) = owned_transaction.take() {
                transaction.rollback(self);
            } else {
                self.vm = traditional_vm.take();
            }
            return Err(error);
        }
        if let Some(random) = random_before_load
            && let Err(error) = self.establish_input_undo_checkpoint(slot, bytes.to_vec(), random)
        {
            if let Some(transaction) = owned_transaction.take() {
                transaction.rollback(self);
            } else {
                self.vm = traditional_vm.take();
            }
            return Err(error);
        }
        self.install_input_replay(replay_origin);
        #[cfg(test)]
        if slot == u32::MAX
            && let Some(transaction) = owned_transaction.take()
        {
            transaction.rollback(self);
            return Err(RuntimeError::Internal(
                "injected owned replacement failure before publication".into(),
            ));
        }
        if let Some(transaction) = owned_transaction.as_mut() {
            transaction.publish(self);
            sql_cleanup = Some(transaction.old_sql_cleanup());
        } else {
            self.vm = traditional_vm.take();
        }
        drop(owned_transaction);
        if let Some((provider, handles)) = sql_cleanup {
            let _ = self.emit_sql_cleanup_for(provider, &handles);
        }
        Ok(())
    }

    pub(in super::super) fn finish_owned_load_failure(
        &mut self,
        host_request: Option<erabasic_vm::HostRequestId>,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                context: None,
                code: "runtime.owned_save_restore_failed".into(),
                level: RuntimeLogLevel::Warning,
                message: message.into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        )?;
        if let Some(request) = host_request {
            self.resume_storage_host(request, Vec::new())
        } else if self.system_menu == SystemMenuState::LoadSlots {
            self.presentation.append_system_text(
                localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                SystemTextKey::InvalidValue,
                Vec::new(),
                true,
            );
            self.render_slot_menu(false)
        } else {
            self.set_phase(RuntimePhase::Running)
        }
    }

    pub(in super::super) fn complete_character_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
        storage_path: &str,
    ) -> Result<(), RuntimeError> {
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::ExternalDataLoad {
            storage_path: storage_path.to_owned(),
            payload_digest: crate::input_replay::digest_hex(bytes),
            data_type: crate::input_replay::ReplayExternalDataType::Character,
        })?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("character load has no VM".into()))?;
        let Ok(decoded) = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Character,
        ) else {
            let writes = global_place(&vm, "RESULT")
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: VmValue::Integer(0),
                    }]
                })
                .unwrap_or_default();
            commit_completion(
                &mut vm,
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )?;
            self.vm = Some(vm);
            return self.set_phase(RuntimePhase::Running);
        };
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::AppendCharacters(Box::new(
                decoded.state,
            )))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.install_input_replay(replay_origin);
        Ok(())
    }
}

const fn owned_load_sql_blocker_message(blocker: crate::sql::SqlSnapshotBlocker) -> &'static str {
    match blocker {
        crate::sql::SqlSnapshotBlocker::Inflight => {
            "owned load cannot replace SQL while a request is pending"
        }
        crate::sql::SqlSnapshotBlocker::Reader => {
            "owned load cannot replace SQL while a reader is active"
        }
        crate::sql::SqlSnapshotBlocker::Transaction => {
            "owned load cannot replace SQL while a transaction is active"
        }
        crate::sql::SqlSnapshotBlocker::RevisionMissing => {
            "owned load cannot replace SQL with an untracked current revision"
        }
    }
}
