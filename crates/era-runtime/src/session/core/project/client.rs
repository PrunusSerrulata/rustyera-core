#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(in super::super::super) fn observe_projection(
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

    pub(in super::super::super) fn emit_projection_state(&mut self) -> Result<(), RuntimeError> {
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

    pub(in super::super::super) fn hello(
        &mut self,
        message_id: u64,
        hello: &ClientHello,
    ) -> Result<(), RuntimeError> {
        let supported = VersionRange::exact(RUNTIME_PROTOCOL_VERSION);
        let Some(selected) = negotiate_version(hello.runtime_versions, supported) else {
            self.emit_log(
                RuntimeLogLevel::Error,
                "runtime protocol negotiation failed: runtime protocol 31.0 is required",
            )?;
            return self.emit(
                RuntimeMessage::VersionRejected(VersionRejected {
                    supported,
                    message: "runtime protocol 31.0 is required".into(),
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

    pub(in super::super::super) fn analyze_project(
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

    pub(in super::super::super) fn submit_key_macro_profile(
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
            FilePayload::IoError(_) | FilePayload::Bytes(_) | FilePayload::ExternalResource(_) => {
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

    pub(in super::super::super) fn submit_extension_registry(
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

    pub(in super::super::super) fn apply_key_macro_command(
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
}
