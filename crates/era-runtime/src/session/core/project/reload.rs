#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super::super) fn reload_project(
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

    pub(in super::super::super) fn commit_project_reload(
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

pub(super) struct ValidatedConfigurationUpdate {
    pub(super) values: era_config::ConfigStore,
    pub(super) document: era_config::ReraConfigDocument,
    pub(super) changed_codes: BTreeSet<String>,
    pub(super) restart_required: bool,
}

pub(super) fn validate_configuration_changes(
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

pub(super) fn commit_configuration_manifest(
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

pub(super) fn apply_hot_configuration(
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

pub(super) fn manifest_contains_omitted_payloads(manifest: &ProjectManifest) -> bool {
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

fn report_project_progress_boundary(
    reporter: Option<&ProjectProgressReporter>,
    stage: ProjectProgressStage,
    complete: bool,
) {
    if let Some(reporter) = reporter {
        reporter.report(ProjectProgress {
            stage,
            completed: u64::from(complete),
            total: 1,
        });
    }
}

pub(super) fn project_payload_required_report(project_revision: u64) -> ProjectLoadReport {
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

pub(super) fn exact_cached_project_with_progress(
    exact: crate::compiled_cache::DecodedCompiledCache,
    project_revision: u64,
    configuration_profile: ConfigurationClientProfile,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    report_project_progress_boundary(progress, ProjectProgressStage::Preparing, false);
    let build = exact_cached_project(exact, project_revision, configuration_profile);
    report_project_progress_boundary(progress, ProjectProgressStage::Preparing, true);
    build
}
