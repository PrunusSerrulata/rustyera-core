#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    /// Stage an owned compiled-project cache for the next project-load request.
    ///
    /// In-process hosts use this entry point to avoid serializing an already contiguous cache
    /// through the chunked frontend protocol. The cache's embedded version, digest, identities,
    /// and bytecode are still validated by the normal project-load path before installation.
    ///
    /// # Errors
    ///
    /// Returns an error when another import is active or the cache exceeds the negotiated
    /// transfer limit.
    pub fn stage_compiled_project_cache(&mut self, bytes: Vec<u8>) -> Result<u64, RuntimeError> {
        if self.inbound_transfer.is_some() {
            return Err(RuntimeError::Busy("another state import is already active"));
        }
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            > self.options.limits.maximum_transfer_bytes
        {
            return Err(RuntimeError::ResourceLimit(
                "compiled project cache exceeds the negotiated transfer limit",
            ));
        }
        let transfer_id = self.allocate_transfer();
        self.inbound_transfer = Some(InboundStateTransfer {
            descriptor: StateTransferDescriptor {
                transfer_id,
                kind: StateExportKind::CompiledProjectCache,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                // Host staging transfers ownership in one call. The compiled-cache decoder
                // validates the format's own trailing digest before any artifact is installed.
                digest: ProtocolBytes::new(Vec::new()),
                artifact_id: None,
            },
            bytes,
            committed: true,
        });
        Ok(transfer_id)
    }

    pub(in super::super) fn stage_full_project_manifest(
        &mut self,
        message_id: u64,
        request: FullProjectManifest,
    ) -> Result<(), RuntimeError> {
        if self.full_project_task.is_some() || self.outbound_transfer.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "a project export is already active",
            );
        }
        self.full_project_failure = None;
        self.full_project_file = None;
        self.staged_full_project_manifest = Some(request.manifest);
        Ok(())
    }

    /// Return the negotiated upper bound for an in-process compiled-cache staging call.
    #[must_use]
    pub const fn maximum_transfer_bytes(&self) -> u64 {
        self.options.limits.maximum_transfer_bytes
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn export_state(
        &mut self,
        message_id: u64,
        request: StateExportRequest,
    ) -> Result<(), RuntimeError> {
        if request.kind != StateExportKind::VmSnapshot
            && request.snapshot_purpose != SnapshotExportPurpose::Normal
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "snapshot purpose is only valid for VM snapshot exports",
            );
        }
        if request.kind == StateExportKind::VmSnapshot && !self.queued_input.is_empty() {
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ineligible {
                        reasons: vec![SnapshotIneligibleReason::SnapshotStateUnavailable],
                    },
                }),
                Some(message_id),
            );
        }
        if request.snapshot_purpose == SnapshotExportPurpose::Debug
            && self.active_debug_grant.is_none()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "debug snapshot export requires an active debug session",
            );
        }
        if request.kind == StateExportKind::CompiledProjectCache {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            if let Some(error) = self.compiled_cache_failure.clone() {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    &format!("compiled project cache preparation failed: {error}"),
                );
            }
            if self.compiled_project_cache.is_none() && self.compiled_cache_task.is_none() {
                if let Err(error) = self.start_compiled_cache_build() {
                    return self.reject(message_id, CommandErrorCode::ResourceLimit, &error);
                }
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "compiled project cache preparation started",
                );
            }
            let Some(bytes) = self.compiled_project_cache.clone() else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    if self.compiled_cache_task.is_some() {
                        "compiled project cache is still being prepared"
                    } else {
                        "no compiled project cache is available"
                    },
                );
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "compiled project cache exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes,
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        if request.kind == StateExportKind::FullProjectFile {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            if let Some(error) = self.full_project_failure.clone() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("full project preparation failed: {error}"),
                );
            }
            if self.full_project_file.is_none() && self.full_project_task.is_none() {
                if let Err(error) = self.start_full_project_build() {
                    return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
                }
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "full project preparation started",
                );
            }
            let Some(bytes) = self.full_project_file.take() else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "full project is still being prepared",
                );
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "full project exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes,
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        if request.kind == StateExportKind::InputReplay {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            let bytes = self
                .input_replay
                .encode()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "input replay exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes: Arc::new(bytes),
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        let unrestricted_snapshot = request.kind == StateExportKind::VmSnapshot
            && request.snapshot_purpose != SnapshotExportPurpose::Normal;
        let stable_wait = self.operations.active_input().is_some_and(|pending| {
            pending.wait.stability == WaitStability::StableInput
                && pending.wait.deadline_ns.is_none()
        });
        let mut reasons = Vec::new();
        if !unrestricted_snapshot {
            if self.phase != RuntimePhase::WaitingInput || !stable_wait {
                reasons.push(SnapshotIneligibleReason::StableWaitRequired);
            }
            if self.operations.has_transient_external() || !self.effect_journal.is_empty() {
                reasons.push(SnapshotIneligibleReason::ExternalOperationPending);
            }
            if request.kind == StateExportKind::VmSnapshot && !self.operations.is_snapshot_stable()
            {
                reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
            }
            if request.kind == StateExportKind::VmSnapshot && self.undo_replay.is_some() {
                reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
            }
        }
        if request.kind == StateExportKind::VmSnapshot && self.vm.is_none() {
            reasons.push(SnapshotIneligibleReason::VmSnapshotUnavailable);
        }
        if request.kind == StateExportKind::VmSnapshot && self.project_snapshot.is_none() {
            reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
        }
        let result = if reasons.is_empty() {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("save export has no VM".into()))?;
            let bytes = match request.kind {
                StateExportKind::TraditionalSave => encode_era_save(
                    &vm.export_era_state(),
                    vm.vm().artifact(),
                    String::new(),
                    merge_structured_extensions(
                        &self.save_extensions,
                        vm.structured_extensions(StructuredScope::Ordinary)
                            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    )
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    self.traditional_save_format(),
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                StateExportKind::VmSnapshot => {
                    if !unrestricted_snapshot
                        && !matches!(vm.snapshot_eligibility(), SnapshotEligibility::Eligible)
                    {
                        self.emit_log(
                            RuntimeLogLevel::Warning,
                            "state export is ineligible: SnapshotStateUnavailable",
                        )?;
                        return self.emit(
                            RuntimeMessage::StateExportReady(StateExportReady {
                                kind: request.kind,
                                result: StateExportResult::Ineligible {
                                    reasons: vec![
                                        SnapshotIneligibleReason::SnapshotStateUnavailable,
                                    ],
                                },
                            }),
                            Some(message_id),
                        );
                    }
                    let vm_snapshot = if unrestricted_snapshot {
                        vm.encode_unrestricted_snapshot()
                    } else {
                        vm.encode_snapshot()
                    }
                    .map_err(|error| {
                        RuntimeError::Internal(format!("VM snapshot encode failed: {error}"))
                    })?;
                    let project = self.project_snapshot.as_ref().ok_or_else(|| {
                        RuntimeError::Internal("snapshot export has no project identity".into())
                    })?;
                    runtime_snapshot::encode(&RuntimeSnapshotPayload {
                        format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
                        origin: match request.snapshot_purpose {
                            SnapshotExportPurpose::Normal => RuntimeSnapshotOrigin::Normal,
                            SnapshotExportPurpose::Debug => RuntimeSnapshotOrigin::Debug,
                            SnapshotExportPurpose::Diagnosis => RuntimeSnapshotOrigin::Diagnosis,
                        },
                        artifact_id: vm.artifact_id(),
                        project_identity: project.project_identity,
                        resource_count: u64::try_from(project.resources.len()).unwrap_or(u64::MAX),
                        resource_graph: project.resource_graph.compact_snapshot(),
                        epoch: self.epoch.0,
                        vm_snapshot,
                        presentation: self.presentation.clone(),
                        operations: self.operations.clone(),
                        controller: self.controller.clone(),
                        logical_time_ns: self.logical_time_ns,
                        random_seed: self.random_seed,
                        selected_locale: self.selected_locale.clone(),
                        culture_table_version: CULTURE_TABLE_VERSION,
                        message_skip: self.message_skip,
                        skip_print: self.skip_print,
                        user_defined_skip: self.user_defined_skip,
                        saved_skip: self.saved_skip,
                        force_kana_mode: self.force_kana_mode,
                        hotkey_state: self.hotkey_state.clone(),
                        key_macros: self.key_macros.clone(),
                        text_box: self.text_box.clone(),
                        text_box_layout: self.text_box_layout,
                        flow_input_enabled: self.flow_input_enabled,
                        flow_input_default: self.flow_input_default,
                        flow_input_can_skip: self.flow_input_can_skip,
                        flow_input_force_skip: self.flow_input_force_skip,
                        flow_input_string: self.flow_input_string,
                        flow_input_default_string: self.flow_input_default_string.clone(),
                        button_generation: self.button_generation,
                        debug_output: self.debug_output.clone(),
                        debug_output_base: self.debug_output_base,
                        command_intents: self.command_intents.clone(),
                        reusable_system_intents: self.reusable_system_intents.clone(),
                        save_extensions: self.save_extensions.clone(),
                        system_menu: match self.system_menu {
                            SystemMenuState::Title => 0,
                            SystemMenuState::LoadSlots => 1,
                            SystemMenuState::SaveSlots => 2,
                            SystemMenuState::ConfirmOverwrite { .. } => 3,
                        },
                        system_menu_slot: match self.system_menu {
                            SystemMenuState::ConfirmOverwrite { slot } => Some(slot),
                            _ => None,
                        },
                        load_slot_paths: self.load_slot_paths.clone(),
                        occupied_slot_paths: self.occupied_slot_paths.clone(),
                        system_menu_host_request: self.system_menu_host_request,
                        system_menu_page: self.system_menu_page,
                        undo_checkpoint: self.undo_checkpoint.clone(),
                        undo_replay: self.undo_replay.clone(),
                    })
                    .map_err(RuntimeError::Internal)?
                }
                StateExportKind::CompiledProjectCache
                | StateExportKind::FullProjectFile
                | StateExportKind::InputReplay => {
                    unreachable!("handled above")
                }
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "state export exceeds the negotiated transfer limit",
                );
            }
            let export_artifact_id = (request.kind == StateExportKind::VmSnapshot)
                .then(|| ProtocolBytes::new(vm.artifact_id().bytes()));
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: export_artifact_id,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes: Arc::new(bytes),
                next_offset: 0,
            });
            StateExportResult::Ready {
                transfer: descriptor,
            }
        } else {
            self.emit_log(
                RuntimeLogLevel::Warning,
                format!("state export is ineligible: {reasons:?}"),
            )?;
            StateExportResult::Ineligible { reasons }
        };
        self.emit(
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: request.kind,
                result,
            }),
            Some(message_id),
        )
    }

    pub(in super::super) fn start_compiled_cache_build(&mut self) -> Result<(), String> {
        let artifact = self
            .artifact
            .clone()
            .ok_or_else(|| "compiled cache build has no loaded artifact".to_owned())?;
        self.compiled_cache_failure = None;
        let snapshot = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| "compiled cache build has no project snapshot".to_owned())?;
        if snapshot.configuration_snapshot().restart_pending {
            return Err(
                "compiled cache build requires restarting to apply pending configuration"
                    .to_owned(),
            );
        }
        let manifest = Arc::clone(&snapshot.manifest);
        let snapshot = crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot);
        let extensions = self.extension_declarations.clone();
        let incremental = Arc::clone(&self.incremental);
        let diagnostics = self.compiled_cache_diagnostics.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let cancelled = Arc::new(AtomicBool::new(false));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let worker_cancelled = Arc::clone(&cancelled);
            let handle = std::thread::Builder::new()
                .name("rustyera-compiled-cache".into())
                .spawn(move || {
                    crate::compiled_cache::encode_cancellable(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        worker_cancelled,
                    )
                })
                .map_err(|error| format!("cannot start compiled cache worker: {error}"))?;
            self.compiled_cache_task = Some(ProjectContainerTask::Native {
                cancelled,
                handle: Some(handle),
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
                encoder: Box::new(
                    crate::compiled_cache::CooperativeCompiledCacheEncoder::new_with_incremental(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        None,
                    ),
                ),
            });
        }
        Ok(())
    }

    fn start_full_project_build(&mut self) -> Result<(), String> {
        let artifact = self
            .artifact
            .clone()
            .ok_or_else(|| "full project build has no loaded artifact".to_owned())?;
        let snapshot = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| "full project build has no project snapshot".to_owned())?;
        if snapshot.configuration_snapshot().restart_pending {
            return Err("full project export requires restarting pending configuration".into());
        }
        let manifest = self
            .staged_full_project_manifest
            .take()
            .unwrap_or_else(|| snapshot.manifest.as_ref().clone());
        crate::compiled_cache::validate_full_project_manifest(
            &manifest,
            &crate::compiled_cache::project_identity(&snapshot.manifest),
            &artifact.artifact().source_map.sources,
        )?;
        // A user-requested full export takes precedence over speculative cache preparation.
        // Dropping the cache task signals cancellation without coupling game interaction to it.
        self.compiled_cache_task = None;
        self.compiled_project_cache = None;
        self.compiled_cache_failure = None;
        let manifest = Arc::new(manifest);
        let snapshot = crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot);
        let extensions = self.extension_declarations.clone();
        let incremental = Arc::clone(&self.incremental);
        let diagnostics = self.compiled_cache_diagnostics.clone();
        let progress = self.project_progress_reporter.clone();
        self.full_project_failure = None;
        if let Some(reporter) = &self.project_progress_reporter {
            reporter.report(ProjectProgress {
                stage: ProjectProgressStage::Packaging,
                completed: 0,
                total: 1,
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let cancelled = Arc::new(AtomicBool::new(false));
            let worker_cancelled = Arc::clone(&cancelled);
            let handle = std::thread::Builder::new()
                .name("rustyera-full-project".into())
                .spawn(move || {
                    crate::compiled_cache::encode_full_project_cancellable(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        crate::compiled_cache::ProjectContainerControl {
                            cancelled: worker_cancelled,
                            progress,
                        },
                    )
                })
                .map_err(|error| format!("cannot start full project worker: {error}"))?;
            self.full_project_task = Some(ProjectContainerTask::Native {
                cancelled,
                handle: Some(handle),
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.full_project_task = Some(ProjectContainerTask::Cooperative {
                encoder: Box::new(
                    crate::compiled_cache::CooperativeCompiledCacheEncoder::new_full_project(
                        manifest,
                        extensions,
                        artifact,
                        incremental,
                        snapshot,
                        diagnostics,
                        None,
                        progress,
                    ),
                ),
            });
        }
        Ok(())
    }

    pub(in super::super) fn poll_compiled_cache_task(&mut self) -> Result<bool, RuntimeError> {
        let (result, cooperative_work) = poll_project_container_task(
            &mut self.compiled_cache_task,
            "compiled cache worker panicked",
        );
        let Some(result) = result else {
            return Ok(cooperative_work);
        };
        match result {
            Ok(bytes) => {
                self.compiled_cache_failure = None;
                self.compiled_project_cache = Some(Arc::new(bytes));
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.compiled_cache_ready".into(),
                        level: RuntimeLogLevel::Info,
                        message: "compiled project cache is ready for frontend persistence".into(),
                        source: None,
                    }),
                    None,
                )?;
            }
            Err(error) => {
                self.compiled_cache_failure = Some(error.clone());
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.compiled_cache_failed".into(),
                        level: RuntimeLogLevel::Warning,
                        message: error,
                        source: None,
                    }),
                    None,
                )?;
            }
        }
        Ok(cooperative_work)
    }

    pub(in super::super) fn poll_full_project_task(&mut self) -> bool {
        let (result, cooperative_work) = poll_project_container_task(
            &mut self.full_project_task,
            "full project worker panicked",
        );
        let Some(result) = result else {
            return cooperative_work;
        };
        match result {
            Ok(bytes) => {
                self.full_project_failure = None;
                self.full_project_file = Some(Arc::new(bytes));
            }
            Err(error) => self.full_project_failure = Some(error),
        }
        cooperative_work
    }

    pub(in super::super) fn cancel_state_export(&mut self, cancel: StateExportCancel) {
        if self
            .outbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.kind == cancel.kind)
        {
            self.outbound_transfer = None;
        }
        match cancel.kind {
            StateExportKind::CompiledProjectCache => {
                self.compiled_cache_task = None;
                self.compiled_project_cache = None;
                self.compiled_cache_failure = None;
            }
            StateExportKind::FullProjectFile => {
                self.full_project_task = None;
                self.full_project_file = None;
                self.full_project_failure = None;
                self.staged_full_project_manifest = None;
            }
            StateExportKind::TraditionalSave
            | StateExportKind::VmSnapshot
            | StateExportKind::InputReplay => {}
        }
    }

    pub(in super::super) fn begin_state_import(
        &mut self,
        message_id: u64,
        request: StateImportBegin,
    ) -> Result<(), RuntimeError> {
        if request.kind == StateExportKind::InputReplay {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "input replay is export-only and cannot be imported",
            );
        }
        if self.inbound_transfer.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "another state import is already active",
            );
        }
        if request.total_bytes > self.options.limits.maximum_transfer_bytes {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import exceeds the negotiated transfer limit",
            );
        }
        if request.digest.as_slice().len() != blake3::OUT_LEN {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import digest must contain 32 bytes",
            );
        }
        match usize::try_from(request.total_bytes) {
            Ok(_) => {}
            Err(_) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state import length is not addressable on this platform",
                );
            }
        }
        let transfer_id = self.allocate_transfer();
        self.inbound_transfer = Some(InboundStateTransfer {
            descriptor: StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: request.total_bytes,
                digest: request.digest,
                artifact_id: request.artifact_id,
            },
            // Grow with accepted chunks instead of trusting a potentially huge declaration.
            bytes: Vec::new(),
            committed: false,
        });
        self.emit(
            RuntimeMessage::StateImportAccepted(StateImportAccepted { transfer_id }),
            Some(message_id),
        )
    }

    pub(in super::super) fn append_state_import(
        &mut self,
        message_id: u64,
        chunk: &StateImportChunk,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != chunk.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if chunk.offset != u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunks must be contiguous and ordered",
            );
        }
        if chunk.data.as_slice().is_empty()
            || chunk
                .offset
                .saturating_add(u64::try_from(chunk.data.as_slice().len()).unwrap_or(u64::MAX))
                > transfer.descriptor.total_bytes
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunk has an invalid length",
            );
        }
        transfer
            .bytes
            .try_reserve(chunk.data.as_slice().len())
            .map_err(|_| RuntimeError::ResourceLimit("state import allocation failed"))?;
        transfer.bytes.extend_from_slice(chunk.data.as_slice());
        Ok(())
    }

    pub(in super::super) fn commit_state_import(
        &mut self,
        message_id: u64,
        commit: StateImportCommit,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != commit.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX)
            != transfer.descriptor.total_bytes
            || transfer.descriptor.digest.as_slice() != blake3::hash(&transfer.bytes).as_bytes()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import length or digest does not match its descriptor",
            );
        }
        transfer.committed = true;
        let kind = transfer.descriptor.kind;
        self.emit(
            RuntimeMessage::StateImportReady(StateImportReady {
                transfer_id: commit.transfer_id,
                kind,
            }),
            Some(message_id),
        )
    }

    pub(in super::super) fn read_state_export(
        &mut self,
        message_id: u64,
        request: StateExportChunkRequest,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.outbound_transfer.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state export is active",
            );
        };
        if transfer.descriptor.transfer_id != request.transfer_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state export transfer is stale",
            );
        }
        if request.offset != transfer.next_offset {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunks must be read contiguously and in order",
            );
        }
        let offset = match usize::try_from(request.offset) {
            Ok(offset) if offset <= transfer.bytes.len() => offset,
            _ => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state export offset is outside the payload",
                );
            }
        };
        if request.maximum_bytes == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunk size must be non-zero",
            );
        }
        let protocol_overhead = 1024_u64;
        let negotiated = self
            .options
            .limits
            .maximum_payload_bytes
            .saturating_sub(protocol_overhead);
        let requested = u64::from(request.maximum_bytes).min(negotiated);
        if requested == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::ResourceLimit,
                "negotiated payload limit cannot carry a state chunk",
            );
        }
        let end = offset
            .saturating_add(usize::try_from(requested).unwrap_or(usize::MAX))
            .min(transfer.bytes.len());
        let complete = end == transfer.bytes.len();
        let response = StateExportChunk {
            transfer_id: request.transfer_id,
            offset: request.offset,
            data: ProtocolBytes::new(transfer.bytes[offset..end].to_vec()),
            complete,
        };
        self.emit(RuntimeMessage::StateExportChunk(response), Some(message_id))?;
        if complete {
            self.outbound_transfer = None;
        } else if let Some(transfer) = self.outbound_transfer.as_mut() {
            transfer.next_offset = u64::try_from(end).unwrap_or(u64::MAX);
        }
        Ok(())
    }

    pub(in super::super) fn cancel_state_transfer(
        &mut self,
        message_id: u64,
        cancel: StateTransferCancel,
    ) -> Result<(), RuntimeError> {
        let inbound = self
            .inbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        let outbound = self
            .outbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        if !inbound && !outbound {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state transfer is stale",
            );
        }
        if inbound {
            self.inbound_transfer = None;
        }
        if outbound {
            self.outbound_transfer = None;
        }
        Ok(())
    }

    pub(in super::super) fn consume_state_import(
        &mut self,
        message_id: u64,
        transfer_id: u64,
        kind: StateExportKind,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        let valid = self.inbound_transfer.as_ref().is_some_and(|transfer| {
            transfer.descriptor.transfer_id == transfer_id
                && transfer.descriptor.kind == kind
                && transfer.committed
        });
        if !valid {
            self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "start requires a committed state import of the requested kind",
            )?;
            return Ok(None);
        }
        Ok(self.inbound_transfer.take().map(|transfer| transfer.bytes))
    }
}

fn poll_project_container_task(
    task: &mut Option<ProjectContainerTask>,
    panic_message: &'static str,
) -> (Option<Result<Vec<u8>, String>>, bool) {
    let Some(active) = task.as_mut() else {
        return (None, false);
    };
    match active {
        #[cfg(any(target_arch = "wasm32", test))]
        ProjectContainerTask::Cooperative { encoder } => match encoder.step() {
            Ok(None) => (None, true),
            result => {
                *task = None;
                (
                    Some(result.transpose().expect("completed container result")),
                    true,
                )
            }
        },
        #[cfg(not(target_arch = "wasm32"))]
        ProjectContainerTask::Native { handle, .. } => {
            if !handle.as_ref().is_some_and(JoinHandle::is_finished) {
                return (None, false);
            }
            let mut finished = task.take().expect("finished container task exists");
            let handle = match &mut finished {
                ProjectContainerTask::Native { handle, .. } => handle,
                #[cfg(test)]
                ProjectContainerTask::Cooperative { .. } => {
                    unreachable!("finished native container task changed variant")
                }
            }
            .take()
            .expect("finished container task has a join handle");
            drop(finished);
            let result = handle
                .join()
                .unwrap_or_else(|_| Err(panic_message.to_owned()));
            (Some(result), false)
        }
    }
}
