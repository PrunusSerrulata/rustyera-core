#[allow(clippy::wildcard_imports)]
use super::*;

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

    pub(in super::super::super) fn prepare_configuration_update(
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

    pub(in super::super::super) fn finalize_configuration_update(
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

    pub(in super::super::super) fn apply_client_preferences(
        &mut self,
        message_id: u64,
        request: ClientPreferenceLayers,
    ) -> Result<(), RuntimeError> {
        let Some(snapshot) = self.project_snapshot.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "client preferences require a loaded project",
            );
        };
        if snapshot.manifest.project_revision != request.project_revision {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "client preference project revision is stale",
            );
        }
        let normalized =
            match normalize_client_preferences(snapshot, request, self.configuration_profile) {
                Ok(value) => value,
                Err(message) => {
                    return self.reject(message_id, CommandErrorCode::InvalidValue, message);
                }
            };
        let snapshot = self
            .project_snapshot
            .as_mut()
            .expect("validated client preference project remains loaded");
        resolve_client_configuration(snapshot, &normalized);
        self.client_preferences = Some(normalized);
        self.presentation.configure_project(snapshot);
        let canvas_defaults = (
            self.presentation.default_foreground_rgb(),
            self.presentation.default_background_rgb(),
            self.presentation.font(),
            u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
        );
        snapshot.resource_graph.configure_canvas_defaults(
            canvas_defaults.0,
            canvas_defaults.1,
            canvas_defaults.2,
            canvas_defaults.3,
        );
        let configuration = snapshot.configuration_snapshot();
        self.emit(
            RuntimeMessage::ClientPreferencesApplied(ClientPreferencesApplied { configuration }),
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
        if let Some(preferences) = &self.client_preferences {
            resolve_client_configuration(&mut snapshot, preferences);
        } else {
            snapshot.client_configuration = snapshot.configuration.clone();
        }
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
}

fn normalize_client_preferences(
    snapshot: &NormalizedProjectSnapshot,
    request: ClientPreferenceLayers,
    profile: ConfigurationClientProfile,
) -> Result<ClientPreferenceLayers, &'static str> {
    fn normalize_layer(
        snapshot: &NormalizedProjectSnapshot,
        changes: Vec<ConfigurationChange>,
        profile: ConfigurationClientProfile,
    ) -> Result<Vec<ConfigurationChange>, &'static str> {
        let client = match profile {
            ConfigurationClientProfile::Tui => era_config::ConfigClient::Tui,
            ConfigurationClientProfile::Browser => era_config::ConfigClient::Browser,
            ConfigurationClientProfile::Tauri => era_config::ConfigClient::Tauri,
            ConfigurationClientProfile::Reference => {
                return Err("reference clients do not support client preferences");
            }
        };
        let catalog = era_config::catalog();
        let mut seen = BTreeSet::new();
        let mut normalized = Vec::with_capacity(changes.len());
        for change in changes {
            let Some(spec) = catalog
                .iter()
                .find(|spec| spec.code.eq_ignore_ascii_case(&change.code))
            else {
                return Err("client preferences contain an unknown setting");
            };
            if spec.effect != era_config::ConfigEffect::QueryOnlyClientPreference
                || !spec.clients.contains(&client)
                || !seen.insert(spec.code.to_ascii_uppercase())
            {
                return Err("client preferences contain an unsupported or duplicate setting");
            }
            let mut values = snapshot.configuration.clone();
            if values.apply(spec.code, &change.value, false).is_err() {
                return Err("client preferences contain an invalid value");
            }
            let value = values
                .get_code(spec.code)
                .expect("catalog preference has a parsed value");
            let mut validation = era_config::ReraConfigDocument::empty();
            if validation.set_code(spec.code, value).is_err() {
                return Err("client preferences contain an out-of-range value");
            }
            normalized.push(ConfigurationChange {
                code: spec.code.into(),
                value: value.config_text(),
            });
        }
        Ok(normalized)
    }

    Ok(ClientPreferenceLayers {
        project_revision: request.project_revision,
        global: normalize_layer(snapshot, request.global, profile)?,
        project: normalize_layer(snapshot, request.project, profile)?,
    })
}

pub(super) fn resolve_client_configuration(
    snapshot: &mut NormalizedProjectSnapshot,
    preferences: &ClientPreferenceLayers,
) {
    snapshot.client_configuration = snapshot.configuration.clone();
    for change in &preferences.global {
        if snapshot.editable_configuration.is_specified(&change.code) {
            continue;
        }
        snapshot
            .client_configuration
            .apply(&change.code, &change.value, false)
            .expect("normalized global client preference remains valid");
    }
    for change in &preferences.project {
        snapshot
            .client_configuration
            .apply(&change.code, &change.value, false)
            .expect("normalized project client preference remains valid");
    }
}
