// This is part of the split RuntimeSession interaction implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn complete_service(
        &mut self,
        message_id: u64,
        response: ServiceResponse,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.operations.take_service(response.request_id) else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "service response has no pending request",
            );
        };
        if let PendingService::Sql(continuation) = pending {
            return self.complete_sql_service(
                message_id,
                response.request_id,
                continuation,
                response.result,
            );
        }
        if let PendingService::Host(ExternalCompletion::DevicePump {
            request,
            epoch,
            after_event_sequence,
            milliseconds,
        }) = pending
        {
            return self.finish_device_pump(
                request,
                epoch,
                after_event_sequence,
                milliseconds,
                response.result,
            );
        }
        if let PendingService::Host(ExternalCompletion::HtmlQuery { continuation }) = pending {
            return self.complete_html_query(continuation, response.result);
        }
        if let PendingService::ProjectImageMetadata { relative_path } = pending {
            let result = match response.result {
                ServiceResult::Ready { payload } => {
                    let metadata: ImageMetadataResponse = decode_canonical(payload.as_slice())?;
                    let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                        RuntimeError::Internal(
                            "image metadata completion has no pending project".into(),
                        )
                    })?;
                    let snapshot =
                        pending
                            .candidate
                            .build_mut()
                            .snapshot
                            .as_mut()
                            .ok_or_else(|| {
                                RuntimeError::Internal(
                                    "image metadata completion has no resource graph".into(),
                                )
                            })?;
                    snapshot
                        .resource_graph
                        .apply_metadata(&relative_path, metadata)
                }
                ServiceResult::Error { error } => Err(format!("{}: {}", error.code, error.message)),
            };
            {
                let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                    RuntimeError::Internal("image metadata completion has no load report".into())
                })?;
                pending
                    .remaining_metadata
                    .remove(&relative_path.to_ascii_lowercase());
                if let Err(message) = result {
                    let report = &mut pending.candidate.build_mut().report;
                    report.success = false;
                    report.diagnostics.push(ProtocolDiagnostic {
                        context: None,
                        code: "runtime.invalid_image_metadata".into(),
                        level: RuntimeLogLevel::Error,
                        message,
                        source: Some(era_runtime_protocol::SourceLocation {
                            relative_path,
                            byte_start: 0,
                            byte_end: 0,
                            line: None,
                            byte_column: None,
                        }),
                        notification: DiagnosticNotification::default(),
                    });
                }
            }
            self.emit_project_image_metadata_requests()?;
            let pending = self.pending_project_load.as_mut().expect("checked above");
            if pending.remaining_metadata.is_empty() {
                let pending = self.pending_project_load.take().expect("checked above");
                match pending.candidate {
                    PendingProjectCandidate::Reload(reload) => {
                        if reload.build.report.success {
                            return self.commit_project_reload(
                                pending.message_id,
                                reload.build,
                                reload.previous_phase,
                                reload.replay_origin,
                            );
                        }
                        self.emit(
                            RuntimeMessage::ProjectLoadReport(reload.build.report),
                            Some(pending.message_id),
                        )?;
                        return self.set_phase(reload.previous_phase);
                    }
                    PendingProjectCandidate::Cold(candidate) => {
                        if candidate.build.report.success {
                            return self.commit_cold_project_load(pending.message_id, candidate);
                        }
                        self.emit(
                            RuntimeMessage::ProjectLoadReport(candidate.build.report),
                            Some(pending.message_id),
                        )?;
                        return self.set_phase(candidate.previous_phase);
                    }
                }
            }
            return Ok(());
        }
        if let PendingService::PlatformEffect { operation } = &pending {
            let failure = match response.result {
                ServiceResult::Ready { payload } if operation == OPEN_URL_OPERATION => {
                    let response: OpenUrlResponse = decode_canonical(payload.as_slice())?;
                    (!response.opened).then_some("frontend declined the URL request".to_owned())
                }
                ServiceResult::Ready { .. } => None,
                ServiceResult::Error { error } => {
                    Some(format!("{}: {}", error.code, error.message))
                }
            };
            if let Some(message) = failure {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        context: None,
                        code: "runtime.platform_effect_failed".into(),
                        level: RuntimeLogLevel::Warning,
                        message,
                        source: None,
                        notification: DiagnosticNotification::default(),
                    }),
                    Some(message_id),
                )?;
            }
            return Ok(());
        }
        if let PendingService::CandidateSaveClock {
            slot,
            precondition,
            continuation,
        } = pending
        {
            let payload = match response.result {
                ServiceResult::Ready { payload } => payload,
                ServiceResult::Error { error } => {
                    return self.finish_candidate_save_failure(
                        continuation,
                        &format!("candidate clock failed: {}: {}", error.code, error.message),
                    );
                }
            };
            let time: LocalDateTimeResponse = decode_canonical(payload.as_slice())?;
            let (mut candidate, bytes) = match self.prepare_candidate_save(time) {
                Ok(value) => value,
                Err(error) => {
                    return self.finish_candidate_save_failure(
                        continuation,
                        &format!("candidate SAVEINFO failed: {error}"),
                    );
                }
            };
            if matches!(continuation, CandidateSaveContinuation::SystemMenu { .. }) {
                candidate.save_bytes.clone_from(&bytes);
                candidate.save_slot = match continuation {
                    CandidateSaveContinuation::SystemMenu { slot, .. } => Some(slot),
                    CandidateSaveContinuation::Autosave => None,
                };
            }
            self.pending_candidate_commit = Some(candidate);
            return self.issue_storage(
                PendingStorage::CandidateSaveWrite { continuation },
                StorageNamespace::Save,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition,
                },
                save_slot_path(slot),
            );
        }
        if let PendingService::Host(ExternalCompletion::UpdateCheck { request, .. }) = &pending
            && let ServiceResult::Error { error } = &response.result
        {
            let result = if error.code.eq_ignore_ascii_case("network_unavailable") {
                5
            } else {
                3
            };
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending update check has no VM".into()))?;
            commit_host_result_write(vm, *request, result)?;
            return self.set_phase(RuntimePhase::Running);
        }
        if let PendingService::Host(
            ExternalCompletion::DecodeCanvasImage { request, .. }
            | ExternalCompletion::EncodeCanvasPng { request, .. }
            | ExternalCompletion::SerializePhysicalHistory { request, .. },
        ) = &pending
            && matches!(&response.result, ServiceResult::Error { .. })
        {
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("graphics service has no VM".into()))?;
            commit_integer_result(vm, *request, 0)?;
            return self.set_phase(RuntimePhase::Running);
        }
        let payload = match response.result {
            ServiceResult::Ready { payload } => payload,
            ServiceResult::Error { error } => {
                let projection_query = matches!(
                    &pending,
                    PendingService::Host(
                        ExternalCompletion::PointerState { .. }
                            | ExternalCompletion::LineGeometry { .. }
                            | ExternalCompletion::CanvasPixel { .. }
                    )
                );
                let code = if projection_query {
                    super::super::html_query::projection_service_failure(&error.code).0
                } else {
                    FaultCode::ServiceFailure
                };
                return self.fault(code, &format!("{}: {}", error.code, error.message), None);
            }
        };
        match pending {
            PendingService::StartEntropy => {
                let seed: RandomSeedResponse = decode_canonical(payload.as_slice())?;
                self.start_new_game(seed.seed)
            }
            PendingService::Host(completion) => {
                let mut writes = Vec::new();
                let host_request = match &completion {
                    ExternalCompletion::DevicePump { .. }
                    | ExternalCompletion::HtmlQuery { .. } => {
                        unreachable!("handled before payload decoding")
                    }
                    ExternalCompletion::GetKey { request: id, .. }
                    | ExternalCompletion::LocalDateTime { request: id, .. }
                    | ExternalCompletion::SpritePixel { request: id }
                    | ExternalCompletion::UpdateCheck { request: id, .. }
                    | ExternalCompletion::PointerState { request: id, .. }
                    | ExternalCompletion::LineGeometry { request: id, .. }
                    | ExternalCompletion::Extension { request: id, .. }
                    | ExternalCompletion::TextExtent { request: id, .. }
                    | ExternalCompletion::DrawTextExtent { request: id, .. }
                    | ExternalCompletion::CanvasPixel { request: id, .. }
                    | ExternalCompletion::DecodeCanvasImage { request: id, .. }
                    | ExternalCompletion::EncodeCanvasPng { request: id, .. }
                    | ExternalCompletion::SerializePhysicalHistory { request: id, .. } => *id,
                };
                let value = match completion {
                    ExternalCompletion::DevicePump { .. } => {
                        unreachable!("device pump handled before generic response decoding")
                    }
                    ExternalCompletion::GetKey {
                        key_code,
                        triggered,
                        ..
                    } => {
                        let state: GetKeyStateResponse = decode_canonical(payload.as_slice())?;
                        let index = usize::from(key_code);
                        let previous = self.key_toggle_state[index];
                        let current = u8::from(state.toggle_state) + 1;
                        self.key_toggle_state[index] = current;
                        Some(VmValue::Integer(i64::from(
                            state.frontend_active
                                && state.pressed
                                && (!triggered || previous != current),
                        )))
                    }
                    ExternalCompletion::LocalDateTime {
                        operation, result, ..
                    } => {
                        let time: LocalDateTimeResponse = decode_canonical(payload.as_slice())?;
                        if result.is_none() {
                            let vm = self.vm.as_ref().ok_or_else(|| {
                                RuntimeError::Internal("pending clock service has no VM".into())
                            })?;
                            if let Some(target) = global_place(vm, "RESULT") {
                                writes.push(HostWrite {
                                    target,
                                    value: VmValue::Integer(calendar_number(time)),
                                });
                            }
                            if let Some(target) = global_place(vm, "RESULTS") {
                                writes.push(HostWrite {
                                    target,
                                    value: VmValue::String(calendar_string(time)),
                                });
                            }
                            None
                        } else {
                            Some(match operation {
                                ClockOperation::Time => VmValue::Integer(calendar_number(time)),
                                ClockOperation::Times => VmValue::String(calendar_string(time)),
                                ClockOperation::Millisecond => {
                                    VmValue::Integer(milliseconds_since_year_one(time))
                                }
                                ClockOperation::Second => {
                                    VmValue::Integer(milliseconds_since_year_one(time) / 1_000)
                                }
                            })
                        }
                    }
                    ExternalCompletion::SpritePixel { .. } => {
                        let pixel: ImagePixelResponse = decode_canonical(payload.as_slice())?;
                        Some(VmValue::Integer(i64::from(pixel.argb)))
                    }
                    ExternalCompletion::UpdateCheck { request } => {
                        let update: UpdateCheckResponse = decode_canonical(payload.as_slice())?;
                        if update.remote_version.is_empty() || update.download_url.is_empty() {
                            let vm = self.vm.as_mut().ok_or_else(|| {
                                RuntimeError::Internal("pending update check has no VM".into())
                            })?;
                            commit_host_result_write(vm, request, 3)?;
                            return self.set_phase(RuntimePhase::Running);
                        }
                        let current_version = self
                            .vm
                            .as_ref()
                            .map(|vm| {
                                &vm.vm()
                                    .artifact()
                                    .project_data
                                    .static_data
                                    .game_base
                                    .version_name
                            })
                            .cloned()
                            .unwrap_or_default();
                        if update.remote_version == current_version {
                            let vm = self.vm.as_mut().ok_or_else(|| {
                                RuntimeError::Internal("pending update check has no VM".into())
                            })?;
                            commit_host_result_write(vm, request, 0)?;
                            return self.set_phase(RuntimePhase::Running);
                        }
                        return self.open_update_prompt(
                            request,
                            &update.remote_version,
                            update.download_url,
                        );
                    }
                    ExternalCompletion::PointerState {
                        coordinate,
                        presentation_revision,
                        environment_revision,
                        projection_space_revision,
                        ..
                    } => {
                        let state: PointerStateResponse = match decode_canonical(payload.as_slice())
                        {
                            Ok(state) => state,
                            Err(error) => {
                                // The pending continuation has already been consumed. A bad
                                // reply must terminate its wait instead of escaping from drive.
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    &format!("invalid pointer_state service response: {error}"),
                                    None,
                                );
                            }
                        };
                        if state.presentation_revision != presentation_revision
                            || state.presentation_revision != self.presentation.revision()
                            || state.environment_revision != environment_revision
                            || state.environment_revision != self.projection_environment_revision
                            || state.projection_space_revision != projection_space_revision
                            || state.projection_space_revision != self.projection_space_revision
                        {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "stale pointer projection revision",
                                None,
                            );
                        }
                        Some(match coordinate {
                            PointerCoordinate::X => VmValue::Integer(state.x.0),
                            PointerCoordinate::Y => VmValue::Integer(state.y.0),
                            PointerCoordinate::Button => VmValue::String(state.button_value),
                        })
                    }
                    ExternalCompletion::LineGeometry {
                        context, line_id, ..
                    } => {
                        let geometry: GetLineGeometryV1Response =
                            match decode_canonical(payload.as_slice()) {
                                Ok(geometry) => geometry,
                                Err(error) => {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        &format!(
                                            "invalid get_line_geometry_v1 service response: {error}"
                                        ),
                                        None,
                                    );
                                }
                            };
                        if !self.validate_projection_query_context(context, geometry.context)? {
                            return Ok(());
                        }
                        if geometry.line_id != line_id
                            || geometry.height.0 < 0
                            || geometry.viewport_height.0 < 0
                        {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "line geometry response contains a mismatched line or negative size",
                                None,
                            );
                        }
                        let Some(value) = geometry
                            .top
                            .0
                            .checked_add(geometry.height.0)
                            .and_then(|value| value.checked_sub(geometry.viewport_height.0))
                        else {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "line geometry response overflows GETLINEY coordinates",
                                None,
                            );
                        };
                        Some(VmValue::Integer(value))
                    }
                    ExternalCompletion::Extension {
                        return_type,
                        mutable_places,
                        ..
                    } => {
                        let result: era_runtime_protocol::ExtensionResult =
                            decode_canonical(payload.as_slice())?;
                        let mut seen = BTreeSet::new();
                        for write in result.writes {
                            let ordinal =
                                usize::try_from(write.argument_ordinal).map_err(|_| {
                                    RuntimeError::Internal(
                                        "extension write ordinal is too large".into(),
                                    )
                                })?;
                            if !seen.insert(ordinal) {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    "extension response contains duplicate writes",
                                    None,
                                );
                            }
                            let Some((place, declared_type)) =
                                mutable_places.get(ordinal).and_then(Clone::clone)
                            else {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    "extension wrote a non-mutable argument",
                                    None,
                                );
                            };
                            let Some(value) = extension_protocol_value(write.value) else {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    "extension returned opaque bytes as an EraBasic value",
                                    None,
                                );
                            };
                            if matches!(
                                (declared_type, &value),
                                (
                                    era_runtime_protocol::ExtensionValueType::Integer,
                                    VmValue::String(_)
                                ) | (
                                    era_runtime_protocol::ExtensionValueType::String,
                                    VmValue::Integer(_)
                                )
                            ) {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    "extension write type differs from its argument",
                                    None,
                                );
                            }
                            writes.push(HostWrite {
                                target: place,
                                value,
                            });
                        }
                        match (return_type, result.value) {
                            (era_runtime_protocol::ExtensionValueType::Void, None) => None,
                            (era_runtime_protocol::ExtensionValueType::Integer, Some(value)) => {
                                let Some(value) = extension_protocol_value(value) else {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        "extension returned opaque bytes as an EraBasic value",
                                        None,
                                    );
                                };
                                if !matches!(value, VmValue::Integer(_)) {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        "extension return type differs",
                                        None,
                                    );
                                }
                                Some(value)
                            }
                            (era_runtime_protocol::ExtensionValueType::String, Some(value)) => {
                                let Some(value) = extension_protocol_value(value) else {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        "extension returned opaque bytes as an EraBasic value",
                                        None,
                                    );
                                };
                                if !matches!(value, VmValue::String(_)) {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        "extension return type differs",
                                        None,
                                    );
                                }
                                Some(value)
                            }
                            (era_runtime_protocol::ExtensionValueType::Any, Some(value)) => {
                                let Some(value) = extension_protocol_value(value) else {
                                    return self.fault(
                                        FaultCode::ServiceFailure,
                                        "extension returned opaque bytes as an EraBasic value",
                                        None,
                                    );
                                };
                                Some(value)
                            }
                            _ => {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    "extension response omitted or added an invalid return value",
                                    None,
                                );
                            }
                        }
                    }
                    ExternalCompletion::HtmlQuery { .. } => {
                        unreachable!("handled before payload decoding")
                    }
                    ExternalCompletion::TextExtent { context, .. } => {
                        let result: TextExtentResponse = decode_canonical(payload.as_slice())?;
                        if !self.validate_projection_query_context(context, result.context)? {
                            return Ok(());
                        }
                        if result.width.0 < 0 || result.height.0 < 0 {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "font metric response contains a negative extent",
                                None,
                            );
                        }
                        let vm = self.vm.as_ref().ok_or_else(|| {
                            RuntimeError::Internal("text extent completion has no VM".into())
                        })?;
                        if let Some(target) = global_place_at(vm, "RESULT", 1) {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::Integer(result.height.0),
                            });
                        }
                        Some(VmValue::Integer(result.width.0))
                    }
                    ExternalCompletion::DrawTextExtent {
                        context,
                        canvas_id,
                        text,
                        point,
                        ..
                    } => {
                        let result: TextExtentResponse = decode_canonical(payload.as_slice())?;
                        if !self.validate_projection_query_context(context, result.context)? {
                            return Ok(());
                        }
                        if result.width.0 < 0 || result.height.0 < 0 {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "font metric response contains a negative extent",
                                None,
                            );
                        }
                        let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                            project
                                .resource_graph
                                .draw_canvas_text(canvas_id, text, point)
                        });
                        if !changed {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "canvas changed while text measurement was pending",
                                None,
                            );
                        }
                        self.sync_resource_replay();
                        let vm = self.vm.as_ref().ok_or_else(|| {
                            RuntimeError::Internal("draw-text completion has no VM".into())
                        })?;
                        for (index, value) in [(1, result.width.0), (2, result.height.0)] {
                            if let Some(target) = global_place_at(vm, "RESULT", index) {
                                writes.push(HostWrite {
                                    target,
                                    value: VmValue::Integer(value),
                                });
                            }
                        }
                        Some(VmValue::Integer(1))
                    }
                    ExternalCompletion::CanvasPixel {
                        context,
                        canvas_id,
                        canvas_revision,
                        ..
                    } => {
                        let result: CanvasPixelResponse = match decode_canonical(payload.as_slice())
                        {
                            Ok(result) => result,
                            Err(error) => {
                                return self.fault(
                                    FaultCode::ServiceFailure,
                                    &format!(
                                        "invalid sample_canvas_pixel service response: {error}"
                                    ),
                                    None,
                                );
                            }
                        };
                        if !self.validate_projection_query_context(context, result.context)? {
                            return Ok(());
                        }
                        let current_revision = self
                            .project_snapshot
                            .as_ref()
                            .and_then(|project| {
                                project.resource_graph.canvas_observation(canvas_id)
                            })
                            .map(|(_, _, revision)| revision);
                        if result.canvas_revision != canvas_revision
                            || current_revision != Some(canvas_revision)
                        {
                            return self.fault(
                                FaultCode::ServiceFailure,
                                "stale canvas raster revision",
                                None,
                            );
                        }
                        Some(VmValue::Integer(i64::from(result.argb)))
                    }
                    ExternalCompletion::DecodeCanvasImage {
                        canvas_id, encoded, ..
                    } => {
                        let result: DecodeCanvasImageResponse =
                            decode_canonical(payload.as_slice())?;
                        let created = self.project_snapshot.as_mut().is_some_and(|project| {
                            project.resource_graph.create_canvas_from_encoded(
                                canvas_id,
                                result.width,
                                result.height,
                                encoded,
                            )
                        });
                        self.sync_resource_replay();
                        Some(VmValue::Integer(i64::from(created)))
                    }
                    ExternalCompletion::EncodeCanvasPng {
                        request,
                        relative_path,
                    } => {
                        let result: EncodeCanvasPngResponse = decode_canonical(payload.as_slice())?;
                        if result.encoded.as_slice().is_empty() {
                            Some(VmValue::Integer(0))
                        } else {
                            return self.issue_storage(
                                PendingStorage::GraphicsImageWrite { request },
                                StorageNamespace::Save,
                                StorageOperation::Write {
                                    data: result.encoded,
                                    atomic_replace: true,
                                    precondition: StoragePrecondition::Any,
                                },
                                relative_path,
                            );
                        }
                    }
                    ExternalCompletion::SerializePhysicalHistory {
                        request,
                        context,
                        relative_path,
                    } => {
                        let result: SerializePhysicalHistoryResponse =
                            decode_canonical(payload.as_slice())?;
                        if !self.validate_presentation_query_context(context, result.context)? {
                            return Ok(());
                        }
                        let mut data = vec![0xef, 0xbb, 0xbf];
                        data.extend_from_slice(result.utf8.as_bytes());
                        return self.issue_storage(
                            PendingStorage::HostFunctionWrite { request },
                            StorageNamespace::Log,
                            StorageOperation::Write {
                                data: ProtocolBytes::new(data),
                                atomic_replace: true,
                                precondition: StoragePrecondition::Any,
                            },
                            relative_path,
                        );
                    }
                };
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("pending service has no VM".into()))?;
                commit_completion(
                    vm,
                    host_request,
                    VmHostCompletion::Ready(HostReady { value, writes }),
                )?;
                self.set_phase(RuntimePhase::Running)
            }
            PendingService::ProjectImageMetadata { .. }
            | PendingService::PlatformEffect { .. }
            | PendingService::CandidateSaveClock { .. }
            | PendingService::Sql(_) => {
                unreachable!("handled above")
            }
        }
    }

    fn validate_projection_query_context(
        &mut self,
        expected: ProjectionQueryContext,
        actual: ProjectionQueryContext,
    ) -> Result<bool, RuntimeError> {
        if actual != expected
            || actual.presentation_revision != self.presentation.revision()
            || actual.environment_revision != self.projection_environment_revision
            || actual.projection_space_revision != self.projection_space_revision
        {
            self.fault(
                FaultCode::ServiceFailure,
                "stale or mismatched projection query context",
                None,
            )?;
            return Ok(false);
        }
        Ok(true)
    }

    fn validate_presentation_query_context(
        &mut self,
        expected: ProjectionQueryContext,
        actual: ProjectionQueryContext,
    ) -> Result<bool, RuntimeError> {
        // Display-line serialization is determined by the canonical presentation. A resize may
        // publish a newer projection environment while the frontend is answering the request,
        // but it does not invalidate the requested historical line.
        if actual != expected || actual.presentation_revision != self.presentation.revision() {
            self.fault(
                FaultCode::ServiceFailure,
                "stale or mismatched presentation query context",
                None,
            )?;
            return Ok(false);
        }
        Ok(true)
    }
}
