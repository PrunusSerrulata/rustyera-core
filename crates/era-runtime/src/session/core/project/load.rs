#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(in super::super::super) fn load_project(
        &mut self,
        message_id: u64,
        mut request: ProjectLoadRequest,
    ) -> Result<(), RuntimeError> {
        let previous_phase = self.phase;
        let staged_manifest = self.staged_project_manifest.take();
        let staged_cache = self.staged_project_file_cache.take();
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
        if request.manifest.is_none()
            && request.compiled_cache_transfer_id.is_none()
            && let Some(manifest) = staged_manifest
        {
            request.manifest = Some(manifest);
        }
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
        self.set_phase(RuntimePhase::LoadingProject)?;
        let mut build =
            match self.build_project_from_cache(request, cache_bytes.as_deref(), staged_cache) {
                Ok(build) => build,
                Err(report) => {
                    self.emit(RuntimeMessage::ProjectLoadReport(*report), Some(message_id))?;
                    return self.set_phase(previous_phase);
                }
            };
        build.incremental.compact();
        let exact_cache_hit = build
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit");
        let compiled_project_cache = if exact_cache_hit
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
        let metadata = build
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !build.report.success {
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        let candidate = PendingColdProjectLoad {
            build,
            previous_phase,
            compiled_project_cache,
        };
        if metadata.is_empty() {
            return self.commit_cold_project_load(message_id, candidate);
        }
        self.begin_project_image_metadata(
            message_id,
            PendingProjectCandidate::Cold(candidate),
            metadata,
        )
    }

    fn begin_project_image_metadata(
        &mut self,
        message_id: u64,
        mut candidate: PendingProjectCandidate,
        metadata: Vec<(String, [u8; 32])>,
    ) -> Result<(), RuntimeError> {
        if self
            .service_capabilities
            .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
            != Some(&IMAGE_METADATA_OPERATION_VERSION)
        {
            let (previous_phase, report) = match &mut candidate {
                PendingProjectCandidate::Cold(candidate) => {
                    (candidate.previous_phase, &mut candidate.build.report)
                }
                PendingProjectCandidate::Reload(candidate) => {
                    (candidate.previous_phase, &mut candidate.build.report)
                }
            };
            report.success = false;
            report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.missing_image_metadata_service".into(),
                level: RuntimeLogLevel::Error,
                message: "resource sprites require the negotiated image_metadata service".into(),
                source: None,
                notification: DiagnosticNotification::default(),
            });
            self.emit(
                RuntimeMessage::ProjectLoadReport(report.clone()),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        let remaining_metadata = metadata
            .iter()
            .map(|(path, _)| path.to_ascii_lowercase())
            .collect();
        self.pending_project_load = Some(PendingProjectLoad {
            message_id,
            remaining_metadata,
            queued_metadata: metadata.into(),
            candidate,
        });
        self.emit_project_image_metadata_requests()
    }

    pub(in super::super::super) fn emit_project_image_metadata_requests(
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

    pub(in super::super::super) fn build_project_from_cache(
        &self,
        request: ProjectLoadRequest,
        cache_bytes: Option<&[u8]>,
        staged_cache: Option<crate::compiled_cache::DecodedCompiledCache>,
    ) -> Result<ProjectBuild, Box<ProjectLoadReport>> {
        let (cached, mut cache_warning) =
            self.decode_project_cache_candidate(cache_bytes, staged_cache);
        let expected_key =
            crate::compiled_cache::project_key(&request.identity, &self.extension_declarations);
        let configuration_warning = cached
            .as_ref()
            .and_then(compiled_cache_configuration_warning);
        let cache_compatible = configuration_warning.is_none();
        cache_warning = configuration_warning.or(cache_warning);
        let mut build = match cached {
            Some(exact) if exact.key == expected_key && cache_compatible => {
                exact_cached_project_with_progress(
                    exact,
                    request.identity.project_revision,
                    self.configuration_profile,
                    self.project_progress_reporter.as_ref(),
                )
            }
            cached => {
                let embedded_manifest = cached.as_ref().and_then(|value| {
                    let mut manifest = value.snapshot.manifest.as_ref().clone();
                    manifest.project_revision = request.identity.project_revision;
                    (crate::compiled_cache::project_identity(&manifest).source_digest
                        == request.identity.source_digest)
                        .then_some(manifest)
                });
                let manifest = prefer_complete_manifest(request.manifest, embedded_manifest);
                let Some(manifest) = manifest else {
                    let mut report =
                        project_payload_required_report(request.identity.project_revision);
                    if let Some(error) = cache_warning.take() {
                        report.diagnostics.insert(
                            0,
                            crate::project::project_diagnostic(
                                "runtime.compiled_cache_ignored",
                                RuntimeLogLevel::Warning,
                                error,
                                None,
                            ),
                        );
                    }
                    return Err(Box::new(report));
                };
                if manifest_contains_omitted_payloads(&manifest) {
                    return Err(Box::new(project_payload_required_report(
                        request.identity.project_revision,
                    )));
                }
                let actual_identity = crate::compiled_cache::project_identity(&manifest);
                if actual_identity.source_digest != request.identity.source_digest {
                    return Err(Box::new(ProjectLoadReport {
                        project_revision: request.identity.project_revision,
                        success: false,
                        diagnostics: vec![crate::project::project_diagnostic(
                            "runtime.project_identity_mismatch",
                            RuntimeLogLevel::Error,
                            "submitted project payload differs from its source identity",
                            None,
                        )],
                        payload_required: false,
                        configuration: None,
                        game_information: None,
                    }));
                }
                let reusable_cache = cached.as_ref().filter(|_| cache_compatible);
                let previous_incremental = reusable_cache
                    .as_ref()
                    .map_or(self.incremental.as_ref(), |value| &value.incremental);
                let previous_artifact = reusable_cache
                    .map(|value| value.artifact.artifact())
                    .or_else(|| self.vm.as_ref().map(|vm| vm.vm().artifact()))
                    .or_else(|| self.artifact.as_ref().map(ValidatedArtifact::artifact));
                build_owned_project_with_extensions_and_progress(
                    manifest,
                    Some(previous_incremental),
                    previous_artifact,
                    &self.extension_declarations,
                    self.configuration_profile,
                    self.options.retain_project_source_payloads,
                    self.project_progress_reporter.as_ref(),
                )
            }
        };
        if let Some(error) = cache_warning {
            build
                .report
                .diagnostics
                .push(crate::project::project_diagnostic(
                    "runtime.compiled_cache_ignored",
                    RuntimeLogLevel::Warning,
                    error,
                    None,
                ));
        }
        if !self.options.retain_project_source_payloads
            && let Some(snapshot) = &mut build.snapshot
        {
            crate::project::release_snapshot_manifest_payloads(snapshot);
        }
        Ok(build)
    }

    fn decode_project_cache_candidate(
        &self,
        cache_bytes: Option<&[u8]>,
        staged_cache: Option<crate::compiled_cache::DecodedCompiledCache>,
    ) -> (
        Option<crate::compiled_cache::DecodedCompiledCache>,
        Option<String>,
    ) {
        if let Some(cache) = staged_cache {
            return (Some(cache), None);
        }
        let Some(bytes) = cache_bytes else {
            return (None, None);
        };
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        match crate::compiled_cache::decode_with_progress(
            bytes,
            maximum,
            self.project_progress_reporter.as_ref(),
        ) {
            Ok(cache) => (Some(cache), None),
            Err(error) => (None, Some(error)),
        }
    }

    pub(in super::super::super) fn commit_cold_project_load(
        &mut self,
        message_id: u64,
        candidate: PendingColdProjectLoad,
    ) -> Result<(), RuntimeError> {
        let mut report = candidate.build.report;
        self.compiled_project_cache = candidate.compiled_project_cache;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        self.full_project_file = None;
        self.full_project_task = None;
        self.full_project_failure = None;
        self.client_preferences = None;
        self.staged_full_project_manifest = None;
        self.compiled_cache_diagnostics = report
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
            .cloned()
            .collect();
        self.incremental = Arc::new(candidate.build.incremental);
        self.artifact = candidate.build.artifact;
        self.project_snapshot = candidate.build.snapshot;
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
        self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
        self.set_phase(RuntimePhase::Ready)
    }
}

fn compiled_cache_configuration_warning(
    cached: &crate::compiled_cache::DecodedCompiledCache,
) -> Option<String> {
    era_config::ReraConfigDocument::from_values(&cached.snapshot.configuration)
        .err()
        .map(|error| {
            format!("compiled cache configuration does not match the current catalog: {error}")
        })
}

fn prefer_complete_manifest(
    submitted: Option<ProjectManifest>,
    embedded: Option<ProjectManifest>,
) -> Option<ProjectManifest> {
    match submitted {
        Some(manifest) if !manifest_contains_omitted_payloads(&manifest) => Some(manifest),
        submitted => embedded.or(submitted),
    }
}
