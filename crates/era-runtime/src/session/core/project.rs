// This is part of the split RuntimeSession implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    /// Stage an owned source manifest for the next project-load command from an in-process host.
    ///
    /// The public protocol remains authoritative for ordering, identity, phase validation, and
    /// reporting. Staging only avoids serializing an already-owned manifest through that protocol.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Busy`] when another host manifest is already awaiting its load.
    pub fn stage_project_manifest(
        &mut self,
        manifest: ProjectManifest,
    ) -> Result<(), RuntimeError> {
        if self.staged_project_manifest.is_some() {
            return Err(RuntimeError::Busy(
                "another project manifest is already staged",
            ));
        }
        self.staged_project_manifest = Some(manifest);
        Ok(())
    }

    /// Discard a host-staged source manifest whose project-load command was not submitted.
    pub fn clear_staged_project_manifest(&mut self) {
        self.staged_project_manifest = None;
    }

    pub(in super::super) fn prepare_configuration_update(
        &mut self,
        message_id: u64,
        request: &PrepareConfigurationUpdate,
    ) -> Result<(), RuntimeError> {
        let Some(snapshot) = self.project_snapshot.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "configuration update requires a loaded project",
            );
        };
        let current = snapshot.configuration_snapshot();
        if request.project_revision != current.project_revision
            || request.expected_source_digest != current.source_digest
        {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "configuration source changed since the preferences dialog was opened",
            );
        }
        if self.pending_configuration_update.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "a configuration update is already pending",
            );
        }
        let profile_flag = crate::project::profile_applicability(self.configuration_profile);
        let transactional = profile_flag.is_some();
        let validated = match validate_configuration_changes(
            snapshot,
            &current,
            &request.changes,
            profile_flag,
        ) {
            Ok(validated) => validated,
            Err(message) => {
                return self.reject(message_id, CommandErrorCode::InvalidValue, message);
            }
        };
        let contents = validated.document.to_lf_string();
        let prepared_source_digest =
            ProtocolBytes::new(blake3::hash(contents.as_bytes()).as_bytes().to_vec());
        if transactional {
            self.pending_configuration_update = Some(PendingConfigurationUpdate {
                preparation_message_id: message_id,
                project_revision: current.project_revision,
                expected_source_digest: current.source_digest.clone(),
                prepared_source_digest: prepared_source_digest.clone(),
                contents: contents.clone(),
                values: validated.values,
                document: validated.document,
                changed_codes: validated.changed_codes,
            });
        }
        self.emit(
            RuntimeMessage::ConfigurationUpdatePrepared(ConfigurationUpdatePrepared {
                project_revision: current.project_revision,
                expected_source_digest: current.source_digest,
                contents,
                restart_required: if transactional {
                    validated.restart_required
                } else {
                    true
                },
                prepared_source_digest,
            }),
            Some(message_id),
        )
    }

    pub(in super::super) fn finalize_configuration_update(
        &mut self,
        message_id: u64,
        request: FinalizeConfigurationUpdate,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.pending_configuration_update.take() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "configuration update finalization has no prepared transaction",
            );
        };
        if pending.preparation_message_id != request.preparation_message_id {
            self.pending_configuration_update = Some(pending);
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "configuration update finalization does not match the prepared transaction",
            );
        }
        if request.outcome == ConfigurationUpdateOutcome::Abort {
            let configuration = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("configuration project disappeared".into()))?
                .configuration_snapshot();
            return self.emit(
                RuntimeMessage::ConfigurationUpdateCommitted(ConfigurationUpdateCommitted {
                    configuration,
                }),
                Some(message_id),
            );
        }

        let old_ctrl_z = self
            .project_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.ctrl_z_enabled);
        let current = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Internal("configuration project disappeared before commit".into())
            })?
            .configuration_snapshot();
        if current.project_revision != pending.project_revision
            || current.source_digest != pending.expected_source_digest
        {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "configuration project changed before commit",
            );
        }
        let (next_snapshot, replay_origin) =
            self.build_committed_configuration_snapshot(pending)?;
        let configuration = next_snapshot.configuration_snapshot();
        self.project_snapshot = Some(next_snapshot);
        self.presentation.configure_project(
            self.project_snapshot
                .as_ref()
                .expect("configuration project remains loaded after commit"),
        );
        let character_width_mode = configured_character_width_mode(self.project_snapshot.as_ref());
        if let Some(vm) = &mut self.vm {
            vm.set_character_width_mode(character_width_mode);
        }
        self.compiled_project_cache = None;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        self.full_project_file = None;
        self.full_project_task = None;
        self.full_project_failure = None;
        self.staged_full_project_manifest = None;
        if old_ctrl_z
            && self
                .project_snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.ctrl_z_enabled)
        {
            self.invalidate_input_undo(Some("Ctrl-Z was disabled by a hot configuration update"))?;
        }
        if let Some(origin) = replay_origin {
            self.install_input_replay(origin);
        }
        self.emit(
            RuntimeMessage::ConfigurationUpdateCommitted(ConfigurationUpdateCommitted {
                configuration,
            }),
            Some(message_id),
        )?;
        self.emit_presentation()
    }

    fn build_committed_configuration_snapshot(
        &self,
        pending: PendingConfigurationUpdate,
    ) -> Result<(NormalizedProjectSnapshot, Option<ReplayOrigin>), RuntimeError> {
        let changed_codes = pending.changed_codes.iter().cloned().collect::<Vec<_>>();
        let artifact = self.artifact.as_ref().ok_or_else(|| {
            RuntimeError::Internal("configuration commit has no loaded artifact".into())
        })?;
        let mut snapshot = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Internal("configuration project disappeared before commit".into())
            })?
            .clone();
        let before_revision = snapshot.manifest.project_revision.to_string();
        let before_identity = crate::input_replay::identity_hex(&snapshot.project_identity);
        commit_configuration_manifest(
            &mut snapshot,
            &pending.contents,
            &pending.prepared_source_digest,
        );
        snapshot.editable_configuration = pending.values;
        snapshot.configuration_document = pending.document;
        snapshot.generated_configuration_source = None;
        apply_hot_configuration(&mut snapshot, &pending.changed_codes);
        crate::project::refresh_project_identity(&mut snapshot, artifact);
        let replay_origin = (!changed_codes.is_empty()).then(|| {
            self.input_replay_for_project(
                ReplayOriginDetails::ConfigurationUpdate {
                    before_revision,
                    before_identity,
                    after_revision: snapshot.manifest.project_revision.to_string(),
                    after_identity: crate::input_replay::identity_hex(&snapshot.project_identity),
                    changed_codes,
                },
                &snapshot,
            )
        });
        Ok((snapshot, replay_origin))
    }

    pub(in super::super) fn observe_projection(
        &mut self,
        message_id: u64,
        observation: ProjectionObservation,
    ) -> Result<(), RuntimeError> {
        if observation.environment_revision <= self.projection_environment_revision {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "projection environment revision is not newer",
            );
        }
        if observation.presentation_revision != self.presentation.revision() {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "projection observation does not match the canonical presentation",
            );
        }
        let width = u32::try_from(observation.client_size.width.0).ok();
        let height = u32::try_from(observation.client_size.height.0).ok();
        if width.is_none()
            || width == Some(0)
            || height.is_none()
            || height == Some(0)
            || observation.line_columns == 0
            || !observation.transform.is_valid()
            || observation.projection_space_revision < self.projection_space_revision
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "projection dimensions must be positive",
            );
        }
        self.projection_environment_revision = observation.environment_revision;
        self.projection_space_revision = observation.projection_space_revision;
        self.client_width = width.expect("validated projection width");
        self.client_height = height.expect("validated projection height");
        self.line_columns = observation.line_columns;
        if let Some(vm) = &mut self.vm {
            vm.set_line_columns(self.line_columns);
        }
        self.text_box = observation.text_box;
        Ok(())
    }

    pub(in super::super) fn emit_projection_state(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::ProjectionState(ProjectionState {
                runtime_revision: self.revision,
                text_box: self.text_box.clone(),
                hotkey_state: self.hotkey_state.clone(),
                button_generation: self.button_generation,
                text_box_layout: self.text_box_layout,
            }),
            None,
        )
    }

    pub(in super::super) fn hello(
        &mut self,
        message_id: u64,
        hello: &ClientHello,
    ) -> Result<(), RuntimeError> {
        let supported = VersionRange::exact(RUNTIME_PROTOCOL_VERSION);
        let Some(selected) = negotiate_version(hello.runtime_versions, supported) else {
            self.emit_log(
                RuntimeLogLevel::Error,
                "runtime protocol negotiation failed: runtime protocol 30.0 is required",
            )?;
            return self.emit(
                RuntimeMessage::VersionRejected(VersionRejected {
                    supported,
                    message: "runtime protocol 30.0 is required".into(),
                }),
                Some(message_id),
            );
        };
        self.state = SessionState::Active;
        self.epoch = SessionEpoch(1);
        let limits = intersect_limits(self.options.limits, hello.requested_limits);
        self.options.limits = limits;
        self.options.wire_limits.maximum_envelope_bytes =
            usize::try_from(limits.maximum_envelope_bytes).unwrap_or(usize::MAX);
        self.options.wire_limits.maximum_payload_bytes =
            usize::try_from(limits.maximum_payload_bytes).unwrap_or(usize::MAX);
        let implemented = [
            RuntimeFeature::TraditionalSave,
            RuntimeFeature::VmSnapshot,
            RuntimeFeature::ProjectReload,
            RuntimeFeature::Storage,
            RuntimeFeature::TimedInput,
            RuntimeFeature::ExternalServices,
            RuntimeFeature::StateResynchronization,
            RuntimeFeature::InputUndo,
            RuntimeFeature::ProjectAnalysis,
            RuntimeFeature::KeyMacros,
        ];
        let features: Vec<_> = implemented
            .into_iter()
            .filter(|feature| hello.features.contains(feature))
            .collect();
        self.negotiated_features = features.iter().copied().collect();
        let configuration_profile = hello.configuration_profile.filter(|profile| {
            matches!(
                profile,
                ConfigurationClientProfile::Tui
                    | ConfigurationClientProfile::Browser
                    | ConfigurationClientProfile::Tauri
            )
        });
        self.configuration_profile =
            configuration_profile.unwrap_or(ConfigurationClientProfile::Reference);
        let selected_capabilities = selected_capabilities(&hello.capabilities);
        self.service_capabilities = selected_capabilities
            .services
            .iter()
            .map(|capability| {
                (
                    (capability.kind, capability.operation.clone()),
                    capability.versions.maximum,
                )
            })
            .collect();
        self.storage_capabilities = selected_capabilities.storage;
        self.available_fonts = selected_capabilities
            .available_fonts
            .iter()
            .map(|name| name.to_lowercase())
            .collect();
        self.selected_locale = select_locale(&hello.preferred_locales).into();
        self.presentation.set_projection(
            selected_capabilities.column_cells,
            selected_capabilities.separators,
            selected_capabilities.html,
            selected_capabilities.graphics,
            selected_capabilities.audio,
        );
        self.emit(
            RuntimeMessage::ServerHello(ServerHello {
                selected_version: selected,
                session: self.options.session_id,
                features,
                limits,
                epoch: self.epoch.0,
                selected_capabilities,
                selected_locale: self.selected_locale.clone(),
                configuration_profile,
            }),
            Some(message_id),
        )?;
        self.emit_log(
            RuntimeLogLevel::Debug,
            format!("runtime handshake complete (epoch {})", self.epoch.0),
        )
    }

    pub(in super::super) fn analyze_project(
        &mut self,
        message_id: u64,
        request: &ProjectAnalysisRequest,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::ProjectAnalysis)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "project analysis was not negotiated",
            );
        }
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::LoadingProject | RuntimePhase::Ready
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project analysis requires an idle runtime",
            );
        }
        let return_phase = self.phase;
        self.set_phase(RuntimePhase::AnalyzingProject)?;
        let report = crate::project::analyze_submitted_project_with_extensions(
            request,
            &self.extension_declarations,
            self.configuration_profile,
        );
        self.emit(
            RuntimeMessage::ProjectAnalysisReport(report),
            Some(message_id),
        )?;
        self.set_phase(return_phase)
    }

    pub(in super::super) fn submit_key_macro_profile(
        &mut self,
        message_id: u64,
        profile: &KeyMacroProfileSubmit,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::KeyMacros)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "key macros were not negotiated",
            );
        }
        let path = era_runtime_protocol::validate_relative_path(&profile.relative_path)?;
        if !path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("macro.txt"))
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "key macro profile must be named macro.txt",
            );
        }
        match &profile.payload {
            FilePayload::Utf8(text) => self.key_macros.load(text),
            FilePayload::IoError(error) if error.kind == FrontendIoErrorKind::NotFound => {
                self.key_macros = KeyMacros::default();
            }
            FilePayload::IoError(_) | FilePayload::Bytes(_) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "key macro profile must be UTF-8 or not-found",
                );
            }
        }
        self.emit(
            RuntimeMessage::KeyMacroStateChanged(self.key_macros.state()),
            Some(message_id),
        )
    }

    pub(in super::super) fn submit_extension_registry(
        &mut self,
        message_id: u64,
        registry: ExtensionRegistrySubmit,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::ExternalServices)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "Host extensions require external services",
            );
        }
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::LoadingProject | RuntimePhase::Ready
        ) || self.project_snapshot.is_some()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "extensions must be registered before loading a project",
            );
        }
        let mut declarations = registry.declarations;
        declarations.sort_by(|left, right| {
            left.era_name
                .to_ascii_uppercase()
                .cmp(&right.era_name.to_ascii_uppercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        for declaration in &mut declarations {
            declaration.operation.make_ascii_lowercase();
        }
        if declarations.iter().any(|declaration| {
            self.service_capabilities
                .get(&(ServiceKind::Extension, declaration.operation.clone()))
                != Some(&declaration.operation_version)
        }) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "each Host extension must match an exactly negotiated Extension service",
            );
        }
        self.extension_declarations = declarations;
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.extension_registry_accepted".into(),
                level: RuntimeLogLevel::Info,
                message: format!(
                    "accepted {} portable Host extension declarations",
                    self.extension_declarations.len()
                ),
                source: None,
            }),
            Some(message_id),
        )
    }

    pub(in super::super) fn apply_key_macro_command(
        &mut self,
        message_id: u64,
        command: KeyMacroCommand,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::KeyMacros)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "key macros were not negotiated",
            );
        }
        let valid = match command {
            KeyMacroCommand::SelectGroup(group) => self.key_macros.select_group(group),
            KeyMacroCommand::Store { group, slot, text } => {
                self.key_macros.store(group, slot, text)
            }
            KeyMacroCommand::Clear { group, slot } => {
                self.key_macros.store(group, slot, String::new())
            }
        };
        if !valid {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "key macro group or slot is out of range",
            );
        }
        let state = self.key_macros.state();
        self.emit(
            RuntimeMessage::KeyMacroStateChanged(state.clone()),
            Some(message_id),
        )?;
        if self.negotiated_features.contains(&RuntimeFeature::Storage) {
            let resume_phase = self.phase;
            return self.issue_storage(
                PendingStorage::KeyMacroWrite { resume_phase },
                StorageNamespace::Project,
                StorageOperation::Write {
                    data: ProtocolBytes::new(state.serialized.into_bytes()),
                    atomic_replace: self.storage_capabilities.atomic_replace,
                    precondition: StoragePrecondition::Any,
                },
                "macro.txt".into(),
            );
        }
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.key_macro_not_persisted".into(),
                level: RuntimeLogLevel::Info,
                message: "key macro state changed in memory; frontend storage was not negotiated"
                    .into(),
                source: None,
            }),
            Some(message_id),
        )
    }

    pub(in super::super) fn load_project(
        &mut self,
        message_id: u64,
        request: &ProjectLoadRequest,
    ) -> Result<(), RuntimeError> {
        let staged_manifest = self.staged_project_manifest.take();
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::Ready | RuntimePhase::Faulted
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project loading requires an idle runtime",
            );
        }
        let staged_request;
        let request = if request.manifest.is_none()
            && request.compiled_cache_transfer_id.is_none()
            && let Some(manifest) = staged_manifest
        {
            staged_request = ProjectLoadRequest {
                identity: request.identity.clone(),
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            };
            &staged_request
        } else {
            request
        };
        let cache_bytes = match request.compiled_cache_transfer_id {
            Some(transfer_id) => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::CompiledProjectCache,
                )?
                else {
                    return Ok(());
                };
                Some(bytes)
            }
            None => None,
        };
        // Loading a new project invalidates both a completed cache and any detached result
        // still being produced for the previous project identity.
        self.compiled_project_cache = None;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        self.full_project_file = None;
        self.full_project_task = None;
        self.full_project_failure = None;
        self.staged_full_project_manifest = None;
        self.set_phase(RuntimePhase::LoadingProject)?;
        let mut build = match self.build_project_from_cache(request, cache_bytes.as_deref()) {
            Ok(build) => build,
            Err(report) => {
                self.emit(RuntimeMessage::ProjectLoadReport(*report), Some(message_id))?;
                return self.set_phase(RuntimePhase::Ready);
            }
        };
        build.incremental.compact();
        let exact_cache_hit = build
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit");
        self.compiled_project_cache = if exact_cache_hit
            && cache_bytes
                .as_deref()
                .is_some_and(|bytes| bytes.starts_with(b"RERACACH"))
        {
            // The validated imported bytes are already the desired opaque export. Re-encoding
            // the multi-gigabyte logical artifact would erase most of the warm-start win.
            cache_bytes.map(Arc::new)
        } else {
            // Cache serialization is intentionally lazy. It is a frontend persistence concern
            // and must not add a multi-second zstd pass to the cold-start critical path.
            None
        };
        self.compiled_cache_diagnostics = build
            .report
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
            .cloned()
            .collect();
        self.incremental = Arc::new(build.incremental);
        self.artifact = build.artifact;
        self.project_snapshot = build.snapshot;
        let metadata = self
            .project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !build.report.success || metadata.is_empty() {
            return self.finish_project_load(message_id, build.report);
        }
        self.begin_project_image_metadata(message_id, build.report, metadata)
    }

    fn begin_project_image_metadata(
        &mut self,
        message_id: u64,
        mut report: ProjectLoadReport,
        metadata: Vec<(String, [u8; 32])>,
    ) -> Result<(), RuntimeError> {
        if self
            .service_capabilities
            .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
            != Some(&IMAGE_METADATA_OPERATION_VERSION)
        {
            report.success = false;
            report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.missing_image_metadata_service".into(),
                level: RuntimeLogLevel::Error,
                message: "resource sprites require the negotiated image_metadata service".into(),
                source: None,
            });
            return self.finish_project_load(message_id, report);
        }
        let remaining_metadata = metadata
            .iter()
            .map(|(path, _)| path.to_ascii_lowercase())
            .collect();
        self.pending_project_load = Some(PendingProjectLoad {
            message_id,
            report,
            remaining_metadata,
            queued_metadata: metadata.into(),
            reload: None,
        });
        self.emit_project_image_metadata_requests()
    }

    pub(in super::super) fn emit_project_image_metadata_requests(
        &mut self,
    ) -> Result<(), RuntimeError> {
        let maximum = self.options.limits.maximum_pending_requests as usize;
        if maximum == 0
            && self
                .pending_project_load
                .as_ref()
                .is_some_and(|pending| !pending.queued_metadata.is_empty())
        {
            return Err(RuntimeError::ResourceLimit(
                "too many pending service requests",
            ));
        }
        while self.operations.total_count() < maximum {
            let Some((relative_path, digest)) = self
                .pending_project_load
                .as_mut()
                .and_then(|pending| pending.queued_metadata.pop_front())
            else {
                break;
            };
            let request_id = self.allocate_request()?;
            self.operations.insert_service(
                request_id,
                PendingService::ProjectImageMetadata {
                    relative_path: relative_path.clone(),
                },
            );
            self.emit(
                RuntimeMessage::ServiceRequest(ServiceRequest {
                    request_id,
                    kind: ServiceKind::Image,
                    operation: IMAGE_METADATA_OPERATION.into(),
                    operation_version: IMAGE_METADATA_OPERATION_VERSION,
                    payload: ProtocolBytes::new(encode_canonical(&ImageMetadataRequest {
                        resource_id: relative_path,
                        content_digest: ProtocolBytes::new(digest),
                    })?),
                    deadline_ns: None,
                }),
                None,
            )?;
        }
        Ok(())
    }

    pub(in super::super) fn build_project_from_cache(
        &self,
        request: &ProjectLoadRequest,
        cache_bytes: Option<&[u8]>,
    ) -> Result<ProjectBuild, Box<ProjectLoadReport>> {
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        let mut cache_warning = None;
        let cached =
            cache_bytes.and_then(
                |bytes| match crate::compiled_cache::decode(bytes, maximum) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        cache_warning = Some(error);
                        None
                    }
                },
            );
        let expected_key =
            crate::compiled_cache::project_key(&request.identity, &self.extension_declarations);
        let mut build = match cached {
            Some(exact) if exact.key == expected_key => exact_cached_project(
                exact,
                request.identity.project_revision,
                self.configuration_profile,
            ),
            cached => {
                let embedded_manifest = cached.as_ref().and_then(|value| {
                    let mut manifest = value.snapshot.manifest.as_ref().clone();
                    manifest.project_revision = request.identity.project_revision;
                    (crate::compiled_cache::project_identity(&manifest).source_digest
                        == request.identity.source_digest)
                        .then_some(manifest)
                });
                let Some(manifest) = embedded_manifest.as_ref().or(request.manifest.as_ref())
                else {
                    let mut report =
                        project_payload_required_report(request.identity.project_revision);
                    if let Some(error) = cache_warning.take() {
                        report.diagnostics.insert(
                            0,
                            ProtocolDiagnostic {
                                code: "runtime.compiled_cache_ignored".into(),
                                level: RuntimeLogLevel::Warning,
                                message: error,
                                source: None,
                            },
                        );
                    }
                    return Err(Box::new(report));
                };
                if manifest_contains_omitted_payloads(manifest) {
                    return Err(Box::new(project_payload_required_report(
                        request.identity.project_revision,
                    )));
                }
                let actual_identity = crate::compiled_cache::project_identity(manifest);
                if actual_identity.source_digest != request.identity.source_digest {
                    return Err(Box::new(ProjectLoadReport {
                        project_revision: request.identity.project_revision,
                        success: false,
                        diagnostics: vec![ProtocolDiagnostic {
                            code: "runtime.project_identity_mismatch".into(),
                            level: RuntimeLogLevel::Error,
                            message: "submitted project payload differs from its source identity"
                                .into(),
                            source: None,
                        }],
                        payload_required: false,
                        configuration: None,
                        game_information: None,
                    }));
                }
                let previous_incremental = cached
                    .as_ref()
                    .map_or(self.incremental.as_ref(), |value| &value.incremental);
                let previous_artifact = cached
                    .as_ref()
                    .map(|value| value.artifact.artifact())
                    .or_else(|| self.vm.as_ref().map(|vm| vm.vm().artifact()))
                    .or_else(|| self.artifact.as_ref().map(ValidatedArtifact::artifact));
                build_project_with_extensions_and_progress(
                    manifest,
                    Some(previous_incremental),
                    previous_artifact,
                    &self.extension_declarations,
                    self.configuration_profile,
                    self.project_progress_reporter.as_ref(),
                )
            }
        };
        if let Some(error) = cache_warning {
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.compiled_cache_ignored".into(),
                level: RuntimeLogLevel::Warning,
                message: error,
                source: None,
            });
        }
        Ok(build)
    }

    pub(in super::super) fn finish_project_load(
        &mut self,
        message_id: u64,
        mut report: ProjectLoadReport,
    ) -> Result<(), RuntimeError> {
        if report.success {
            self.undo_checkpoint = None;
            self.undo_replay = None;
            self.undo_token = None;
            if let Some(snapshot) = &self.project_snapshot {
                report.configuration = Some(snapshot.configuration_snapshot());
                self.key_macros.set_enabled(matches!(
                    snapshot.configuration.get_code("UseKeyMacro"),
                    Some(era_config::ConfigValue::Boolean(true))
                ));
                self.presentation.configure_project(snapshot);
            }
            let canvas_defaults = (
                self.presentation.default_foreground_rgb(),
                self.presentation.default_background_rgb(),
                self.presentation.font(),
                u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
            );
            if let Some(snapshot) = &mut self.project_snapshot {
                snapshot.resource_graph.configure_canvas_defaults(
                    canvas_defaults.0,
                    canvas_defaults.1,
                    canvas_defaults.2,
                    canvas_defaults.3,
                );
            }
            self.sync_resource_replay();
        } else {
            self.artifact = None;
            self.project_snapshot = None;
        }
        let success = report.success;
        self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
        self.set_phase(if success {
            RuntimePhase::Ready
        } else {
            RuntimePhase::Faulted
        })
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn reload_project(
        &mut self,
        message_id: u64,
        reload: &ReloadProject,
    ) -> Result<(), RuntimeError> {
        let previous_phase = self.phase;
        if !matches!(
            previous_phase,
            RuntimePhase::Ready | RuntimePhase::Running | RuntimePhase::WaitingInput
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload requires a ready or running runtime",
            );
        }
        if self.operations.total_count() != 0 && !self.operations.is_snapshot_stable() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload cannot cross transient runtime operations",
            );
        }
        let current = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("project reload has no base manifest".into()))?;
        let manifest = match apply_project_delta(&current.manifest, reload) {
            Ok(manifest) => manifest,
            Err(error) => {
                return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
            }
        };
        self.set_phase(RuntimePhase::Reloading)?;
        let previous_artifact = self
            .vm
            .as_ref()
            .map(|vm| vm.vm().artifact())
            .or_else(|| self.artifact.as_ref().map(ValidatedArtifact::artifact));
        let mut build = build_project_with_extensions_and_progress(
            &manifest,
            Some(self.incremental.as_ref()),
            previous_artifact,
            &self.extension_declarations,
            self.configuration_profile,
            self.project_progress_reporter.as_ref(),
        );
        if !build.report.success {
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let (Some(next), Some(previous)) =
            (build.snapshot.as_mut(), self.project_snapshot.as_ref())
        {
            next.resource_graph
                .inherit_runtime_graph(&previous.resource_graph);
        }
        let replay_origin = if reload.changes.is_empty() {
            None
        } else {
            let details = replay_hot_reload_origin(
                reload,
                self.project_snapshot
                    .as_ref()
                    .expect("reload base was checked"),
                build
                    .snapshot
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("reload result has no project".into()))?,
            )?;
            Some(self.input_replay_for_project(
                details,
                build.snapshot.as_ref().expect("reload result was checked"),
            ))
        };
        let metadata = build
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !metadata.is_empty() {
            if self
                .service_capabilities
                .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
                != Some(&IMAGE_METADATA_OPERATION_VERSION)
            {
                build.report.success = false;
                build.report.diagnostics.push(ProtocolDiagnostic {
                    code: "runtime.missing_image_metadata_service".into(),
                    level: RuntimeLogLevel::Error,
                    message:
                        "changed image resources require the negotiated image_metadata service"
                            .into(),
                    source: None,
                });
                self.emit(
                    RuntimeMessage::ProjectLoadReport(build.report),
                    Some(message_id),
                )?;
                return self.set_phase(previous_phase);
            }
            let remaining_metadata = metadata
                .iter()
                .map(|(path, _)| path.to_ascii_lowercase())
                .collect();
            let report = build.report.clone();
            self.pending_project_load = Some(PendingProjectLoad {
                message_id,
                report,
                remaining_metadata,
                queued_metadata: metadata.into(),
                reload: Some(PendingProjectReload {
                    build,
                    previous_phase,
                    replay_origin,
                }),
            });
            return self.emit_project_image_metadata_requests();
        }
        self.commit_project_reload(message_id, build, previous_phase, replay_origin)
    }

    pub(in super::super) fn commit_project_reload(
        &mut self,
        message_id: u64,
        mut build: crate::project::ProjectBuild,
        previous_phase: RuntimePhase,
        replay_origin: Option<ReplayOrigin>,
    ) -> Result<(), RuntimeError> {
        let target = build
            .artifact
            .take()
            .ok_or_else(|| RuntimeError::Internal("successful reload has no artifact".into()))?;
        if let Some(vm) = &mut self.vm
            && let Err(error) = vm.prepare_hot_reload(target.clone())
        {
            build.report.success = false;
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.hot_reload_incompatible".into(),
                level: RuntimeLogLevel::Error,
                message: error.to_string(),
                source: None,
            });
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let Some(vm) = &mut self.vm {
            vm.commit_hot_reload()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }

        self.artifact = Some(target);
        build.incremental.compact();
        self.incremental = Arc::new(build.incremental);
        self.project_snapshot = build.snapshot;
        self.compiled_cache_diagnostics = build
            .report
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
            .cloned()
            .collect();
        self.compiled_project_cache = None;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        self.full_project_file = None;
        self.full_project_task = None;
        self.full_project_failure = None;
        self.staged_full_project_manifest = None;
        if let Some(snapshot) = &self.project_snapshot {
            self.presentation.configure_project(snapshot);
        }
        let character_width_mode = configured_character_width_mode(self.project_snapshot.as_ref());
        if let Some(vm) = &mut self.vm {
            vm.set_character_width_mode(character_width_mode);
        }
        let canvas_defaults = (
            self.presentation.default_foreground_rgb(),
            self.presentation.default_background_rgb(),
            self.presentation.font(),
            u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
        );
        if let Some(snapshot) = &mut self.project_snapshot {
            snapshot.resource_graph.configure_canvas_defaults(
                canvas_defaults.0,
                canvas_defaults.1,
                canvas_defaults.2,
                canvas_defaults.3,
            );
        }
        self.sync_resource_replay();
        let new_epoch = self.epoch.0.saturating_add(1);
        let (tokens, waits) = self.operations.rebind_stable_inputs(
            new_epoch,
            &mut self.next_wait_id,
            &mut self.next_interaction_id,
        );
        self.presentation.rebind_interactions(&tokens, &waits);
        self.command_intents = std::mem::take(&mut self.command_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.reusable_system_intents = std::mem::take(&mut self.reusable_system_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.epoch = SessionEpoch(new_epoch);
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
        self.invalidate_input_undo(Some(
            "successful bytecode hot reload invalidated the Ctrl-Z checkpoint",
        ))?;
        if let Some(origin) = replay_origin {
            self.install_input_replay(origin);
        }
        self.emit(
            RuntimeMessage::ProjectLoadReport(build.report),
            Some(message_id),
        )?;
        self.set_phase(previous_phase)?;
        self.renew_debug_grant()?;
        self.emit_presentation()
    }
}

fn replay_hot_reload_origin(
    reload: &ReloadProject,
    before: &NormalizedProjectSnapshot,
    after: &NormalizedProjectSnapshot,
) -> Result<ReplayOriginDetails, RuntimeError> {
    let changes = reload
        .changes
        .iter()
        .map(|change| {
            let (operation, category, relative_path) = match change {
                era_runtime_protocol::FileChange::Upsert { file } => (
                    crate::input_replay::ReplayFileOperation::Upsert,
                    file.category,
                    file.relative_path.as_str(),
                ),
                era_runtime_protocol::FileChange::Remove {
                    category,
                    relative_path,
                } => (
                    crate::input_replay::ReplayFileOperation::Remove,
                    *category,
                    relative_path.as_str(),
                ),
            };
            Ok(crate::input_replay::ReplayFileChange {
                operation,
                relative_path: era_runtime_protocol::validate_relative_path(relative_path)?,
                category: category.into(),
            })
        })
        .collect::<Result<Vec<_>, era_protocol::ProtocolError>>()
        .map_err(RuntimeError::Protocol)?;
    Ok(ReplayOriginDetails::HotReload {
        before_revision: before.manifest.project_revision.to_string(),
        before_identity: crate::input_replay::identity_hex(&before.project_identity),
        after_revision: after.manifest.project_revision.to_string(),
        after_identity: crate::input_replay::identity_hex(&after.project_identity),
        changes,
    })
}

struct ValidatedConfigurationUpdate {
    values: era_config::ConfigStore,
    document: era_config::ReraConfigDocument,
    changed_codes: BTreeSet<String>,
    restart_required: bool,
}

fn validate_configuration_changes(
    snapshot: &NormalizedProjectSnapshot,
    current: &era_runtime_protocol::ProjectConfigurationSnapshot,
    changes: &[era_runtime_protocol::ConfigurationChange],
    profile_flag: Option<u32>,
) -> Result<ValidatedConfigurationUpdate, &'static str> {
    let editable = current
        .entries
        .iter()
        .filter(|entry| profile_flag.is_none_or(|flag| entry.applicability & flag != 0))
        .map(|entry| (entry.code.to_ascii_uppercase(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut values = snapshot.editable_configuration.clone();
    let mut document = snapshot.configuration_document.clone();
    for change in changes {
        if !editable.contains_key(&change.code.to_ascii_uppercase())
            || values.is_fixed(&change.code)
            || values.apply(&change.code, &change.value, false).is_err()
        {
            return Err("configuration update contains an unsupported, fixed, or invalid value");
        }
        let value = values
            .get_code(&change.code)
            .expect("a validated configuration change has a catalog value");
        if document.set_code(&change.code, value).is_err() {
            return Err("configuration update contains a locked or invalid value");
        }
    }
    let changed_codes = editable
        .values()
        .filter(|entry| {
            values
                .get_code(&entry.code)
                .is_some_and(|value| value.config_text() != entry.value)
        })
        .map(|entry| entry.code.clone())
        .collect::<BTreeSet<_>>();
    let restart_required = editable.values().any(|entry| {
        changed_codes.contains(&entry.code)
            && entry.application == ConfigurationApplication::Restart
    });
    Ok(ValidatedConfigurationUpdate {
        values,
        document,
        changed_codes,
        restart_required,
    })
}

fn commit_configuration_manifest(
    snapshot: &mut NormalizedProjectSnapshot,
    contents: &str,
    digest: &ProtocolBytes,
) {
    let manifest = Arc::make_mut(&mut snapshot.manifest);
    if let Some(file) = manifest
        .files
        .iter_mut()
        .find(|file| crate::project::is_root_configuration_file(file))
    {
        file.payload = FilePayload::Utf8(contents.into());
        file.content_hash = Some(digest.clone());
    } else {
        manifest.files.push(era_runtime_protocol::SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(contents.into()),
            content_hash: Some(digest.clone()),
        });
    }
}

fn apply_hot_configuration(
    snapshot: &mut NormalizedProjectSnapshot,
    changed_codes: &BTreeSet<String>,
) {
    for code in changed_codes {
        if crate::project::profile_application(code, snapshot.configuration_profile)
            != ConfigurationApplication::Hot
        {
            continue;
        }
        let value = snapshot
            .editable_configuration
            .get_code(code)
            .expect("prepared configuration retained a changed value")
            .config_text();
        snapshot
            .configuration
            .apply(code, &value, false)
            .expect("a prepared configuration value remains valid");
    }

    let boolean = |code| match snapshot.configuration.get_code(code) {
        Some(era_config::ConfigValue::Boolean(value)) => Some(*value),
        _ => None,
    };
    let integer = |code| match snapshot.configuration.get_code(code) {
        Some(era_config::ConfigValue::Integer(value)) => Some(*value),
        _ => None,
    };
    snapshot.ctrl_z_enabled = boolean("Ctrl_Z_Enabled").unwrap_or(snapshot.ctrl_z_enabled);
    snapshot.allow_long_input_by_activation =
        boolean("AllowLongInputByMouse").unwrap_or(snapshot.allow_long_input_by_activation);
    if let Some(value) = integer("PrintCPerLine").and_then(|value| u32::try_from(value).ok()) {
        snapshot.print_c_per_line = value.max(1);
    }
    if let Some(value) = integer("PrintCLength").and_then(|value| u32::try_from(value).ok()) {
        snapshot.print_c_length = value.max(1);
    }
    if let Some(value) = integer("WindowX").and_then(|value| u32::try_from(value).ok()) {
        snapshot.viewport_width = value.max(1);
    }
    if let Some(value) = integer("WindowY").and_then(|value| u32::try_from(value).ok()) {
        snapshot.viewport_height = value.max(1);
    }
    if let Some(value) = integer("FontSize").and_then(|value| u32::try_from(value).ok()) {
        snapshot.font_size = value.max(1);
    }
    if let Some(value) = integer("LineHeight").and_then(|value| u32::try_from(value).ok()) {
        snapshot.line_height = value.max(snapshot.font_size);
    }
}

fn manifest_contains_omitted_payloads(manifest: &ProjectManifest) -> bool {
    manifest.files.iter().any(|file| {
        let Some(expected) = &file.content_hash else {
            return false;
        };
        let empty_payload = match &file.payload {
            FilePayload::Utf8(value) => value.is_empty(),
            FilePayload::Bytes(value) => value.as_slice().is_empty(),
            FilePayload::IoError(_) => false,
        };
        empty_payload && expected.as_slice() != blake3::hash(&[]).as_bytes()
    })
}

fn project_payload_required_report(project_revision: u64) -> ProjectLoadReport {
    ProjectLoadReport {
        project_revision,
        success: false,
        diagnostics: vec![ProtocolDiagnostic {
            code: "runtime.project_payload_required".into(),
            level: RuntimeLogLevel::Info,
            message: "compiled cache is missing or does not match the project".into(),
            source: None,
        }],
        payload_required: true,
        configuration: None,
        game_information: None,
    }
}

fn exact_cached_project(
    mut exact: crate::compiled_cache::DecodedCompiledCache,
    project_revision: u64,
    configuration_profile: ConfigurationClientProfile,
) -> ProjectBuild {
    Arc::make_mut(&mut exact.snapshot.manifest).project_revision = project_revision;
    exact.snapshot.configuration_profile = configuration_profile;
    for diagnostic in &mut exact.diagnostics {
        diagnostic.message = format!("[cached] {}", diagnostic.message);
    }
    exact.diagnostics.push(ProtocolDiagnostic {
        code: "runtime.compiled_cache_hit".into(),
        level: RuntimeLogLevel::Debug,
        message: "loaded the exact compiled project cache".into(),
        source: None,
    });
    let game_information = crate::project::project_game_information(&exact.artifact);
    ProjectBuild {
        artifact: Some(exact.artifact),
        incremental: exact.incremental,
        report: ProjectLoadReport {
            project_revision,
            success: true,
            diagnostics: exact.diagnostics,
            payload_required: false,
            configuration: None,
            game_information: Some(Box::new(game_information)),
        },
        snapshot: Some(exact.snapshot),
    }
}
