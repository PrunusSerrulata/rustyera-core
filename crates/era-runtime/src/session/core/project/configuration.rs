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
}
