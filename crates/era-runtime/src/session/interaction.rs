//! Service completion, input adjudication, and runtime-owned system flows.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn complete_service(
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
        if let PendingService::ProjectImageMetadata { relative_path } = pending {
            let result = match response.result {
                ServiceResult::Ready { payload } => {
                    let metadata: ImageMetadataResponse = decode_canonical(payload.as_slice())?;
                    let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                        RuntimeError::Internal(
                            "image metadata completion has no pending project".into(),
                        )
                    })?;
                    let snapshot = match pending.reload.as_mut() {
                        Some(reload) => reload.build.snapshot.as_mut(),
                        None => self.project_snapshot.as_mut(),
                    }
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
            let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                RuntimeError::Internal("image metadata completion has no load report".into())
            })?;
            pending
                .remaining_metadata
                .remove(&relative_path.to_ascii_lowercase());
            if let Err(message) = result {
                pending.report.success = false;
                pending.report.diagnostics.push(ProtocolDiagnostic {
                    code: "runtime.invalid_image_metadata".into(),
                    severity: DiagnosticSeverity::Error,
                    message,
                    source: Some(era_runtime_protocol::SourceLocation {
                        relative_path,
                        byte_start: 0,
                        byte_end: 0,
                        line: None,
                        byte_column: None,
                    }),
                });
            }
            if pending.remaining_metadata.is_empty() {
                let mut pending = self.pending_project_load.take().expect("checked above");
                if let Some(mut reload) = pending.reload.take() {
                    reload.build.report = pending.report;
                    if reload.build.report.success {
                        return self.commit_project_reload(
                            pending.message_id,
                            reload.build,
                            reload.previous_phase,
                        );
                    }
                    self.emit(
                        RuntimeMessage::ProjectLoadReport(reload.build.report),
                        Some(pending.message_id),
                    )?;
                    return self.set_phase(reload.previous_phase);
                }
                return self.finish_project_load(pending.message_id, pending.report);
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
                        code: "runtime.platform_effect_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message,
                        source: None,
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
                return self.fault(
                    FaultCode::ServiceFailure,
                    &format!("{}: {}", error.code, error.message),
                    None,
                );
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
                    ExternalCompletion::GetKey { request: id, .. }
                    | ExternalCompletion::LocalDateTime { request: id, .. }
                    | ExternalCompletion::SpritePixel { request: id }
                    | ExternalCompletion::UpdateCheck { request: id, .. }
                    | ExternalCompletion::PointerState { request: id, .. }
                    | ExternalCompletion::Extension { request: id, .. }
                    | ExternalCompletion::ProjectionString { request: id, .. }
                    | ExternalCompletion::ProjectionInteger { request: id, .. }
                    | ExternalCompletion::HtmlSubstring { request: id, .. }
                    | ExternalCompletion::TextExtent { request: id, .. }
                    | ExternalCompletion::DrawTextExtent { request: id, .. }
                    | ExternalCompletion::CanvasPixel { request: id, .. }
                    | ExternalCompletion::DecodeCanvasImage { request: id, .. }
                    | ExternalCompletion::EncodeCanvasPng { request: id, .. }
                    | ExternalCompletion::SerializePhysicalHistory { request: id, .. } => *id,
                };
                let value = match completion {
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
                        let state: PointerStateResponse = decode_canonical(payload.as_slice())?;
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
                    ExternalCompletion::ProjectionString {
                        operation, context, ..
                    } => {
                        let result: ProjectionStringResponse =
                            decode_canonical(payload.as_slice())?;
                        self.validate_projection_query_context(context, result.context)?;
                        let _ = operation;
                        Some(VmValue::String(result.value))
                    }
                    ExternalCompletion::ProjectionInteger { context, .. } => {
                        let result: ProjectionIntegerResponse =
                            decode_canonical(payload.as_slice())?;
                        self.validate_projection_query_context(context, result.context)?;
                        Some(VmValue::Integer(result.value))
                    }
                    ExternalCompletion::HtmlSubstring { context, .. } => {
                        let result: HtmlSubstringResponse = decode_canonical(payload.as_slice())?;
                        self.validate_projection_query_context(context, result.context)?;
                        let vm = self.vm.as_ref().ok_or_else(|| {
                            RuntimeError::Internal("HTML substring completion has no VM".into())
                        })?;
                        if let Some(target) = global_place_at(vm, "RESULTS", 0) {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(result.head.clone()),
                            });
                        }
                        if let Some(target) = global_place_at(vm, "RESULTS", 1) {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(result.tail),
                            });
                        }
                        Some(VmValue::String(result.head))
                    }
                    ExternalCompletion::TextExtent { context, .. } => {
                        let result: TextExtentResponse = decode_canonical(payload.as_slice())?;
                        self.validate_projection_query_context(context, result.context)?;
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
                        self.validate_projection_query_context(context, result.context)?;
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
                        canvas_revision,
                        ..
                    } => {
                        let result: CanvasPixelResponse = decode_canonical(payload.as_slice())?;
                        self.validate_projection_query_context(context, result.context)?;
                        if result.canvas_revision != canvas_revision {
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
                        self.validate_projection_query_context(context, result.context)?;
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
            | PendingService::CandidateSaveClock { .. } => {
                unreachable!("handled above")
            }
        }
    }

    fn validate_projection_query_context(
        &mut self,
        expected: ProjectionQueryContext,
        actual: ProjectionQueryContext,
    ) -> Result<(), RuntimeError> {
        if actual != expected
            || actual.presentation_revision != self.presentation.revision()
            || actual.environment_revision != self.projection_environment_revision
            || actual.projection_space_revision != self.projection_space_revision
        {
            return self.fault(
                FaultCode::ServiceFailure,
                "stale or mismatched projection query context",
                None,
            );
        }
        Ok(())
    }

    pub(super) fn open_update_prompt(
        &mut self,
        request: erabasic_vm::HostRequestId,
        remote_version: &str,
        download_url: String,
    ) -> Result<(), RuntimeError> {
        self.presentation.append_text(
            format!("New version {remote_version} is available: {download_url}"),
            false,
        );
        let no = self.allocate_interaction();
        let yes = self.allocate_interaction();
        self.presentation.append_button(
            "No".into(),
            era_runtime_protocol::ProtocolValue::Integer(0),
            no,
            None,
        );
        self.presentation.append_button(
            "Yes".into(),
            era_runtime_protocol::ProtocolValue::Integer(1),
            yes,
            None,
        );
        let submission = self.allocate_interaction();
        let pending = PendingInput {
            host_request: Some(request),
            wait: InputWait {
                wait_id: self.allocate_wait(),
                kind: WaitKind::IntegerButton,
                stability: WaitStability::Transient,
                one_input: false,
                stop_message_skip: false,
                system_input: false,
                mouse_input: false,
                default_value: None,
                deadline_ns: None,
                display_time: false,
                timeout_message: None,
                submission_token: submission,
                countdown_remaining_ms: None,
            },
            result_name: Some("RESULT".into()),
            choices: BTreeMap::from([(no, VmValue::Integer(1)), (yes, VmValue::Integer(2))]),
            timeout_duration_ns: None,
            post_input: Some(PostInputAction::OpenUrl {
                url: download_url,
                trigger_value: 2,
            }),
        };
        self.open_wait(pending, false)
    }

    pub(super) fn complete_input(
        &mut self,
        message_id: u64,
        input: FrontendInput,
    ) -> Result<(), RuntimeError> {
        let Some(wait_id) = self
            .operations
            .active_input()
            .map(|pending| pending.wait.wait_id)
        else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "no input is pending",
            );
        };
        if wait_id != input.wait_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input wait identity is stale",
            );
        }
        let observed_time = self.observe_frontend_time(input.monotonic_time_ns);
        let pending = self.operations.active_input().expect("checked above");
        if pending
            .wait
            .deadline_ns
            .is_some_and(|deadline| observed_time > deadline)
        {
            return self.advance_time(
                message_id,
                AdvanceTime {
                    monotonic_time_ns: input.monotonic_time_ns,
                },
            );
        }
        if let InputIntent::CommitText(command) = &input.intent
            && command.len() > 1
            && command.starts_with('@')
            && !pending.wait.one_input
        {
            return self.handle_system_input_command(message_id, command);
        }
        self.message_skip = input.message_skip;
        if let InputIntent::ActivateKeyMacro { group, slot } = &input.intent {
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
            if input.token != pending.wait.submission_token {
                return self.reject(
                    message_id,
                    CommandErrorCode::StaleRequest,
                    "key macro activation token is stale",
                );
            }
            let Some(text) = self.key_macros.recall(*group, *slot) else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "key macro is disabled or out of range",
                );
            };
            self.text_box = text.into();
            return self.emit_projection_state();
        }
        let mut intent = input.intent;
        if let InputIntent::CommitText(text) = intent {
            let Ok(mut pieces) = preprocess_input(&text) else {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "input macro expansion exceeds the runtime limit",
                );
            };
            let (text, skip) = pieces.remove(0);
            self.message_skip |= skip;
            self.queued_input.extend(pieces);
            intent = InputIntent::CommitText(text);
        }
        let allow_long_activation = self
            .project_snapshot
            .as_ref()
            .is_some_and(|project| project.allow_long_input_by_activation);
        let Some(submission) = input_value(pending, input.token, intent, allow_long_activation)
        else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "input value does not match the active wait",
            );
        };
        self.finish_input(submission, false)
    }

    /// Feed the next expanded keyboard-input segment into the next wait without
    /// accepting a new frontend event or bypassing the wait's ordinary validator.
    pub(super) fn consume_queued_input(&mut self) -> Result<(), RuntimeError> {
        let Some((text, skip)) = self.queued_input.pop_front() else {
            return Ok(());
        };
        if text.len() > 1
            && text.starts_with('@')
            && self
                .operations
                .active_input()
                .is_some_and(|pending| !pending.wait.one_input)
        {
            return self.handle_system_input_command(0, &text);
        }
        let pending = self
            .operations
            .active_input()
            .ok_or_else(|| RuntimeError::Internal("queued input has no active wait".into()))?;
        self.message_skip = skip;
        let Some(submission) = input_value(
            pending,
            pending.wait.submission_token,
            InputIntent::CommitText(text),
            false,
        ) else {
            self.queued_input.clear();
            return Ok(());
        };
        self.finish_input(submission, false)
    }

    pub(super) fn handle_system_input_command(
        &mut self,
        message_id: u64,
        command: &str,
    ) -> Result<(), RuntimeError> {
        if self
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.deadline_ns.is_some())
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.system_command_during_timed_wait".into(),
                    severity: DiagnosticSeverity::Warning,
                    message: "system commands cannot be entered during a timed wait".into(),
                    source: None,
                }),
                Some(message_id),
            );
        }
        self.presentation
            .append_print_text(command.to_owned(), false, true);
        self.emit_presentation()?;
        match command[1..].trim().to_ascii_uppercase().as_str() {
            "QUIT" | "EXIT" => {
                let exit = ExitRequested {
                    reason: ExitReason::Quit,
                    force: false,
                    runtime_revision: self.revision.saturating_add(1),
                };
                self.exit_requested = Some(exit);
                self.emit(RuntimeMessage::ExitRequested(exit), Some(message_id))?;
                self.set_phase(RuntimePhase::Stopping)
            }
            "REBOOT" => {
                let exit = ExitRequested {
                    reason: ExitReason::Restart,
                    force: false,
                    runtime_revision: self.revision.saturating_add(1),
                };
                self.exit_requested = Some(exit);
                self.emit(RuntimeMessage::ExitRequested(exit), Some(message_id))?;
                self.set_phase(RuntimePhase::Stopping)
            }
            "OUTPUT" | "OUTPUTLOG" => {
                if !self.negotiated_features.contains(&RuntimeFeature::Storage) {
                    return self.emit(
                        RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                            code: "runtime.system_output_unavailable".into(),
                            severity: DiagnosticSeverity::Warning,
                            message: "@OUTPUT requires negotiated frontend storage".into(),
                            source: None,
                        }),
                        Some(message_id),
                    );
                }
                let mut data = vec![0xef, 0xbb, 0xbf];
                data.extend_from_slice(self.presentation.log_text(false).as_bytes());
                self.issue_storage(
                    PendingStorage::SystemOutputLog {
                        resume_phase: self.phase,
                    },
                    StorageNamespace::Log,
                    StorageOperation::Write {
                        data: ProtocolBytes::new(data),
                        atomic_replace: self.storage_capabilities.atomic_replace,
                        precondition: StoragePrecondition::Any,
                    },
                    "emuera.log".into(),
                )
            }
            "CONFIG" => self.emit_effect(EffectKind::OpenConfiguration),
            "DEBUG" => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.debug_command_requires_debug_channel".into(),
                    severity: DiagnosticSeverity::Warning,
                    message: "@DEBUG is available only through the granted debug protocol".into(),
                    source: None,
                }),
                Some(message_id),
            ),
            _ => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.debug_command_requires_debug_channel".into(),
                    severity: DiagnosticSeverity::Warning,
                    message: "arbitrary input debug commands are available only through the granted debug protocol".into(),
                    source: None,
                }),
                Some(message_id),
            ),
        }
    }

    pub(super) fn advance_time(
        &mut self,
        _message_id: u64,
        time: AdvanceTime,
    ) -> Result<(), RuntimeError> {
        self.observe_frontend_time(time.monotonic_time_ns);
        let ready_delays = self.operations.take_ready_delays(self.logical_time_ns);
        for request in ready_delays {
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending AWAIT has no VM".into()))?;
            commit_completion(vm, request, VmHostCompletion::Ready(HostReady::empty()))?;
            self.set_phase(RuntimePhase::Running)?;
        }
        let timed_out = self
            .operations
            .active_input()
            .and_then(|pending| pending.wait.deadline_ns)
            .is_some_and(|deadline| self.logical_time_ns >= deadline);
        if !timed_out
            && let Some(pending) = self.operations.active_input_mut()
            && pending.wait.display_time
            && let Some(deadline) = pending.wait.deadline_ns
        {
            let remaining = deadline
                .saturating_sub(self.logical_time_ns)
                .saturating_add(999_999)
                / 1_000_000;
            pending.wait.countdown_remaining_ms = Some(remaining);
            let wait = pending.wait.clone();
            self.presentation.set_wait(Some(wait.clone()));
            self.emit(RuntimeMessage::WaitChanged(WaitChange::Updated(wait)), None)?;
            self.emit_presentation()?;
        }
        if timed_out {
            let pending = self.operations.active_input().expect("checked above");
            if let Some(message) = &pending.wait.timeout_message {
                self.presentation.append_text(message.clone(), false);
            }
            let submission = if pending.wait.kind == WaitKind::PrimitiveMouseKey {
                InputSubmission::Primitive(PrimitiveResult {
                    fields: [4, 0, 0, 0, 0],
                    selection: None,
                })
            } else {
                InputSubmission::Value(
                    pending
                        .wait
                        .default_value
                        .as_ref()
                        .map_or(VmValue::Integer(0), protocol_to_vm),
                )
            };
            self.finish_input(submission, true)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn finish_input(
        &mut self,
        submission: InputSubmission,
        timed_out: bool,
    ) -> Result<(), RuntimeError> {
        let pending = self
            .operations
            .take_active_input()
            .ok_or_else(|| RuntimeError::Internal("input wait disappeared".into()))?;
        // The reference TextBox restores its configured position only after a
        // value was accepted, never after a rejected frontend event.
        self.text_box_layout = TextBoxLayout::default();
        if pending.wait.system_input {
            let InputSubmission::Value(value) = submission else {
                return Err(RuntimeError::Internal(
                    "system input cannot accept primitive fields".into(),
                ));
            };
            self.finish_system_input(pending, &value)?;
            return self.emit_projection_state();
        }
        if let InputSubmission::Value(value) = &submission {
            self.record_input_undo_value(value)?;
        }
        let request = pending
            .host_request
            .ok_or_else(|| RuntimeError::Internal("VM wait has no host request".into()))?;
        let post_url = match (&pending.post_input, &submission) {
            (
                Some(PostInputAction::OpenUrl { url, trigger_value }),
                InputSubmission::Value(VmValue::Integer(value)),
            ) if value == trigger_value => Some(url.clone()),
            _ => None,
        };
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("input wait has no VM".into()))?;
        let mut writes = Vec::new();
        match submission {
            InputSubmission::Value(value) => {
                let result_name = if pending.wait.kind == WaitKind::AnyValue {
                    Some(match &value {
                        VmValue::String(_) => "RESULTS",
                        _ => "RESULT",
                    })
                } else {
                    pending.result_name.as_deref()
                };
                if let Some(target) = result_name.and_then(|name| global_place(vm, name)) {
                    writes.push(HostWrite { target, value });
                }
            }
            InputSubmission::Primitive(primitive) => {
                for (index, value) in primitive.fields.into_iter().enumerate() {
                    if let Some(target) = global_place_at(vm, "RESULT", index) {
                        writes.push(HostWrite {
                            target,
                            value: VmValue::Integer(i64::from(value)),
                        });
                    }
                }
                let result_5 = match primitive.selection {
                    Some(VmValue::Integer(value)) => value,
                    Some(VmValue::String(value)) => {
                        if let Some(target) = global_place(vm, "RESULTS") {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(value),
                            });
                        }
                        0
                    }
                    None => 0,
                    Some(VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) => {
                        return Err(RuntimeError::Internal(
                            "an interaction token resolved to a VM place".into(),
                        ));
                    }
                };
                if let Some(target) = global_place_at(vm, "RESULT", 5) {
                    writes.push(HostWrite {
                        target,
                        value: VmValue::Integer(result_5),
                    });
                }
            }
        }
        // ISTIMEOUT is only changed by a timed input completion; untimed waits leave it sticky.
        if pending.wait.deadline_ns.is_some()
            && let Some(target) = global_place(vm, "ISTIMEOUT")
        {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(i64::from(timed_out)),
            });
        }
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        let pause_next_wait = !vm.has_runnable_fibers();
        self.close_wait(pending.wait.wait_id)?;
        if let Some(next) = self.operations.pop_queued_input() {
            self.activate_wait(next, pause_next_wait)?;
        } else {
            self.set_phase(RuntimePhase::Running)?;
        }
        if let Some(url) = post_url {
            self.issue_platform_effect(
                ServiceKind::OpenUrl,
                OPEN_URL_OPERATION,
                OPEN_URL_OPERATION_VERSION,
                &OpenUrlRequest { url },
            )?;
        }
        self.emit_projection_state()
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn finish_system_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        if self.controller.step != SystemStep::None && self.system_menu_host_request.is_none() {
            return self.finish_flow_input(pending, value);
        }
        match (self.system_menu, value) {
            (SystemMenuState::Title, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                let prepared = vm
                    .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                vm.commit_runtime_state(prepared)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                let draw_line = vm
                    .vm()
                    .artifact()
                    .project_data
                    .static_data
                    .replace
                    .draw_line_string
                    .clone();
                self.presentation.append_separator(draw_line);
                self.presentation.append_text(String::new(), false);
                self.controller.flow = Some(SystemFlow::First);
                if !self
                    .controller
                    .prepare_event(vm.vm().artifact(), "EVENTFIRST")
                {
                    return Err(RuntimeError::Internal("EVENTFIRST is not defined".into()));
                }
                let entry = self.controller.next().expect("prepared EVENTFIRST entry");
                let fiber = vm
                    .spawn_entry(entry, Vec::new())
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                self.controller.started(fiber);
                self.set_phase(RuntimePhase::Running)
            }
            (SystemMenuState::Title, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                if self
                    .controller
                    .prepare_function(vm.vm().artifact(), "TITLE_LOADGAME")
                {
                    self.controller.flow = Some(SystemFlow::Title);
                    self.controller.step = SystemStep::TitleLoadOverride;
                    let entry = self
                        .controller
                        .next()
                        .expect("prepared TITLE_LOADGAME entry");
                    let fiber = vm
                        .spawn_entry(entry, Vec::new())
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    self.controller.started(fiber);
                    return self.set_phase(RuntimePhase::Running);
                }
                self.issue_storage(
                    PendingStorage::ListLoadSlots,
                    StorageNamespace::Save,
                    StorageOperation::List {
                        pattern: Some("save*.sav".into()),
                        recursive: false,
                    },
                    String::new(),
                )
            }
            (
                SystemMenuState::LoadSlots | SystemMenuState::SaveSlots,
                VmValue::Integer(selection),
            ) if *selection <= -1_000 => {
                let index = usize::try_from(selection.saturating_neg().saturating_sub(1_000))
                    .unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown delete slot");
                };
                let Some(revision) = self.slot_revisions.get(&path).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidState,
                        "save slot revision is unavailable",
                    );
                };
                let save = self.system_menu == SystemMenuState::SaveSlots;
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::DeleteMenuSlot {
                        save,
                        path: path.clone(),
                    },
                    StorageNamespace::Save,
                    StorageOperation::Delete {
                        precondition: StoragePrecondition::Revision(revision),
                    },
                    path,
                )
            }
            (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots, VmValue::Integer(100)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.resume_system_menu_host()
            }
            (
                menu @ (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots),
                VmValue::Integer(selection),
            ) if *selection >= 0 && (*selection != 99 || menu == SystemMenuState::SaveSlots) => {
                let slot_count = self
                    .project_snapshot
                    .as_ref()
                    .map_or(20, |snapshot| snapshot.save_slot_count)
                    .max(20);
                let Ok(slot) = u32::try_from(*selection) else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                if slot >= slot_count {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                }
                let target_page = slot / 20;
                if target_page != self.system_menu_page {
                    self.close_wait(pending.wait.wait_id)?;
                    self.system_menu_page = target_page;
                    return self.render_slot_menu(menu == SystemMenuState::SaveSlots);
                }
                let path = save_slot_path(slot);
                if menu == SystemMenuState::SaveSlots {
                    self.close_wait(pending.wait.wait_id)?;
                    if self.occupied_slot_paths.contains(&path) {
                        self.system_menu = SystemMenuState::ConfirmOverwrite { slot };
                        self.presentation.append_system_text(
                            localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::OverwriteQuestion,
                            ),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(i64::from(slot))],
                            false,
                        );
                        let yes = self.allocate_interaction();
                        let no = self.allocate_interaction();
                        self.presentation.append_system_button(
                            "Yes".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(0)],
                            yes,
                        );
                        self.presentation.append_system_button(
                            "No".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(1)],
                            no,
                        );
                        let submission = self.allocate_interaction();
                        let mut wait = self.system_wait(submission);
                        wait.kind = WaitKind::IntegerValue;
                        return self.open_wait(
                            PendingInput {
                                host_request: self.system_menu_host_request,
                                wait,
                                result_name: None,
                                choices: BTreeMap::from([
                                    (yes, VmValue::Integer(0)),
                                    (no, VmValue::Integer(1)),
                                ]),
                                timeout_duration_ns: None,
                                post_input: None,
                            },
                            true,
                        );
                    }
                    return self.begin_system_menu_candidate(slot);
                }
                if !self.occupied_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "save slot is empty");
                }
                if self.invalid_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "save slot is incompatible or corrupt",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::LoadSlots, VmValue::Integer(99)) => {
                let path = save_slot_path(99);
                if !self.occupied_slot_paths.contains(&path)
                    || self.invalid_slot_paths.contains(&path)
                {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "autosave is unavailable",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot: 99 },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::ConfirmOverwrite { slot }, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.begin_system_menu_candidate(slot)
            }
            (SystemMenuState::ConfirmOverwrite { .. }, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
            _ => {
                if self.presentation.last_line_is_temporary()
                    && self.presentation.last_line_is_empty()
                {
                    self.presentation.delete_last_lines(2);
                    self.presentation.append_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        true,
                    );
                } else {
                    self.presentation
                        .replace_last_temporary(localized_system_text(
                            &self.selected_locale,
                            SystemTextKey::InvalidValue,
                        ));
                }
                self.operations.restore_active_input(pending);
                self.emit_presentation()?;
                self.reject(
                    0,
                    CommandErrorCode::InvalidValue,
                    "unknown system menu item",
                )
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn finish_flow_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        let VmValue::Integer(selection) = value else {
            self.operations.restore_active_input(pending);
            return self.reject(
                0,
                CommandErrorCode::InvalidValue,
                "system input must be integer",
            );
        };
        let previous_choices = pending.choices.clone();
        self.close_wait(pending.wait.wait_id)?;
        self.set_phase(RuntimePhase::Running)?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("system flow input has no VM".into()))?;
        let result = match self.controller.step {
            SystemStep::TrainShowUser => {
                if let Some(command) = usize::try_from(*selection)
                    .ok()
                    .and_then(|index| self.controller.train_commands.get(index))
                    .copied()
                {
                    self.controller.selected_command = Some(command);
                    write_runtime_integer(&mut vm, "SELECTCOM", &[], None, command)?;
                    fill_runtime_variable(&mut vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if self.dispatch_system_event(&mut vm, "EVENTCOM")? {
                        Ok(())
                    } else {
                        self.continue_system_flow(&mut vm)
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::TrainUserCom;
                    self.dispatch_system_function(&mut vm, "USERCOM", true)?;
                    Ok(())
                }
            }
            SystemStep::AblupShowSelect => {
                self.controller.step = SystemStep::AblupAction;
                if (0..100).contains(selection) {
                    if self.dispatch_system_function(
                        &mut vm,
                        &format!("ABLUP{selection}"),
                        false,
                    )? {
                        Ok(())
                    } else {
                        self.presentation
                            .replace_last_temporary(localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::InvalidValue,
                            ));
                        self.command_intents = previous_choices.clone();
                        self.open_system_command_wait()
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.dispatch_system_function(&mut vm, "USERABLUP", true)?;
                    Ok(())
                }
            }
            SystemStep::ShopShow => {
                let maximum = self
                    .project_snapshot
                    .as_ref()
                    .map_or(100, |snapshot| snapshot.maximum_shop_items);
                if *selection >= 0 && *selection < i64::from(maximum) {
                    let purchase = purchase_item(
                        &mut vm,
                        usize::try_from(*selection).unwrap_or(usize::MAX),
                        maximum,
                    )?;
                    match purchase {
                        PurchaseResult::Purchased => {
                            self.controller.step = SystemStep::ShopAction;
                            if !self.dispatch_system_event(&mut vm, "EVENTBUY")? {
                                self.continue_system_flow(&mut vm)?;
                            }
                            Ok(())
                        }
                        PurchaseResult::OutOfStock | PurchaseResult::NotEnoughMoney => {
                            let key = if purchase == PurchaseResult::NotEnoughMoney {
                                SystemTextKey::NotEnoughMoney
                            } else {
                                SystemTextKey::OutOfStock
                            };
                            self.presentation
                                .replace_last_temporary(localized_system_text(
                                    &self.selected_locale,
                                    key,
                                ));
                            self.command_intents = previous_choices.clone();
                            self.open_system_command_wait()
                        }
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::ShopAction;
                    self.dispatch_system_function(&mut vm, "USERSHOP", true)?;
                    Ok(())
                }
            }
            SystemStep::TrainEventComEndWait => {
                self.controller.step = SystemStep::TrainShowStatus;
                self.dispatch_system_function(&mut vm, "SHOW_STATUS", true)?;
                Ok(())
            }
            SystemStep::ShopAutosaveFailureWait => {
                if let Some(flow) = self.controller.deferred_flow.take() {
                    self.controller.flow = Some(flow);
                    return self.begin_flow(&mut vm, flow).map(|()| {
                        self.vm = Some(vm);
                    });
                }
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(&mut vm, "SHOW_SHOP", true)?;
                Ok(())
            }
            _ => Err(RuntimeError::Internal(
                "system flow received input outside an input step".into(),
            )),
        };
        self.vm = Some(vm);
        result
    }

    pub(super) fn spawn_next_event(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        if let Some(entry) = self.controller.next() {
            let fiber = vm
                .spawn_entry(entry, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.started(fiber);
        }
        Ok(())
    }

    pub(super) fn begin_flow(
        &mut self,
        vm: &mut RuntimeVm,
        flow: SystemFlow,
    ) -> Result<(), RuntimeError> {
        self.skip_print = false;
        if flow == SystemFlow::Train {
            reset_training_state(vm)?;
            self.controller.train_scan = 0;
            self.controller.train_commands.clear();
            self.controller.clear_continuous_train();
        }
        self.controller.step = match flow {
            SystemFlow::Train => SystemStep::TrainEvent,
            SystemFlow::Ablup => SystemStep::AblupShowJuel,
            SystemFlow::Shop => SystemStep::ShopEvent,
            _ => SystemStep::None,
        };
        let (entry, event, required) = match flow {
            SystemFlow::Title => ("SYSTEM_TITLE", false, false),
            SystemFlow::First => ("EVENTFIRST", true, true),
            SystemFlow::Train => ("EVENTTRAIN", true, false),
            SystemFlow::AfterTrain => ("EVENTEND", true, true),
            SystemFlow::Ablup => ("SHOW_JUEL", false, true),
            SystemFlow::TurnEnd => ("EVENTTURNEND", true, true),
            SystemFlow::Shop => ("EVENTSHOP", true, false),
            SystemFlow::Normal => {
                return self.fault(
                    FaultCode::VmFault,
                    "NORMAL is an internal system state and is not a BEGIN target",
                    None,
                );
            }
        };
        if event {
            if self.controller.prepare_event(vm.vm().artifact(), entry) {
                return self.spawn_next_event(vm);
            }
        } else if self.controller.prepare_function(vm.vm().artifact(), entry) {
            return self.spawn_next_event(vm);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {entry} is not defined"),
                None,
            )
        } else if self.controller.step != SystemStep::None {
            self.continue_system_flow(vm)
        } else {
            Ok(())
        }
    }

    pub(super) fn dispatch_system_function(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
        required: bool,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_function(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            return Ok(true);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {name} is not defined"),
                None,
            )?;
        }
        Ok(false)
    }

    pub(super) fn dispatch_system_event(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_event(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn open_system_command_wait(&mut self) -> Result<(), RuntimeError> {
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        if !self.flow_input_string {
            wait.kind = WaitKind::IntegerValue;
        }
        let choices = std::mem::take(&mut self.command_intents);
        self.reusable_system_intents.clone_from(&choices);
        self.open_wait(
            PendingInput {
                host_request: None,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn continue_system_flow(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        match self.controller.step {
            SystemStep::TrainEvent => {
                let next = read_runtime_integer(vm, "NEXTCOM", &[], None)?;
                if next >= 0 {
                    write_runtime_integer(vm, "SELECTCOM", &[], None, next)?;
                    write_runtime_integer(vm, "NEXTCOM", &[], None, 0)?;
                    self.controller.selected_command = Some(next);
                    fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if !self.dispatch_system_event(vm, "EVENTCOM")? {
                        return self.continue_system_flow(vm);
                    }
                } else {
                    if self.controller.continuous_train {
                        // Emuera suppresses SHOW_STATUS and the command table while it
                        // rebuilds COM_ABLE for a continuous command.
                        self.skip_print = true;
                    }
                    self.controller.step = SystemStep::TrainShowStatus;
                    self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                }
            }
            SystemStep::TrainShowStatus => {
                self.controller.step = SystemStep::TrainComAble;
                self.controller.train_scan = 0;
                self.controller.train_commands.clear();
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainComAble => {
                let command = self.controller.train_scan.saturating_sub(1);
                if read_runtime_integer(vm, "RESULT", &[], None)? != 0 {
                    self.controller
                        .train_commands
                        .push(i64::try_from(command).unwrap_or(i64::MAX));
                }
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainShowUser if self.controller.continuous_train => {
                reset_after_show_user(vm)?;
                self.skip_print = false;
                if let Some(command) = self.controller.continuous_commands.pop_front() {
                    self.controller.continuous_executed =
                        self.controller.continuous_executed.saturating_add(1);
                    let current =
                        i64::try_from(self.controller.continuous_executed).unwrap_or(i64::MAX);
                    let total = i64::try_from(self.controller.continuous_total).unwrap_or(i64::MAX);
                    let text = localized_system_text(
                        &self.selected_locale,
                        SystemTextKey::ContinuousTrainProgress,
                    )
                    .replace("{0}", &current.to_string())
                    .replace("{1}", &total.to_string());
                    self.presentation.append_system_text(
                        text,
                        SystemTextKey::ContinuousTrainProgress,
                        vec![
                            SystemTextArgument::Integer(current),
                            SystemTextArgument::Integer(total),
                        ],
                        false,
                    );
                    if self.controller.train_commands.contains(&command) {
                        self.controller.selected_command = Some(command);
                        write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
                        fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                        self.controller.step = SystemStep::TrainEventCom;
                        if !self.dispatch_system_event(vm, "EVENTCOM")? {
                            return self.continue_system_flow(vm);
                        }
                    } else {
                        self.presentation.append_system_text(
                            localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::ContinuousTrainCommandFailed,
                            ),
                            SystemTextKey::ContinuousTrainCommandFailed,
                            Vec::new(),
                            false,
                        );
                        write_runtime_integer(vm, "RESULT", &[], None, command)?;
                        self.controller.step = SystemStep::TrainUserCom;
                        self.dispatch_system_function(vm, "USERCOM", true)?;
                    }
                } else {
                    return self.finish_continuous_train(vm);
                }
            }
            SystemStep::TrainShowUser => {
                reset_after_show_user(vm)?;
                return self.open_system_command_wait();
            }
            SystemStep::AblupShowSelect | SystemStep::ShopShow => {
                return self.open_system_command_wait();
            }
            SystemStep::TrainUserCom => {
                if self.controller.continuous_train
                    && self.controller.continuous_commands.is_empty()
                {
                    return self.finish_continuous_train(vm);
                }
                self.controller.step = SystemStep::TrainShowStatus;
                self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
            }
            SystemStep::TrainEventComEnd => {
                if self.controller.continuous_train
                    && self.controller.continuous_commands.is_empty()
                {
                    return self.finish_continuous_train(vm);
                }
                return self.finish_event_com_end(vm);
            }
            SystemStep::TrainEventCom => {
                let command = self.controller.selected_command.ok_or_else(|| {
                    RuntimeError::Internal("training command selection disappeared".into())
                })?;
                self.controller.step = SystemStep::TrainCommand;
                self.dispatch_system_function(vm, &format!("COM{command}"), true)?;
            }
            SystemStep::TrainCommand => {
                let result = read_runtime_integer(vm, "RESULT", &[], None)?;
                if result == 0 {
                    self.controller.step = SystemStep::TrainShowStatus;
                    self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                } else {
                    self.controller.step = SystemStep::TrainSourceCheck;
                    self.dispatch_system_function(vm, "SOURCE_CHECK", true)?;
                }
            }
            SystemStep::TrainSourceCheck => {
                fill_runtime_variable(vm, "SOURCE", VmValue::Integer(0), true)?;
                self.controller.step = SystemStep::TrainEventComEnd;
                self.controller.event_com_end_wait_required = true;
                if !self.dispatch_system_event(vm, "EVENTCOMEND")? {
                    return self.continue_system_flow(vm);
                }
            }
            SystemStep::AblupShowJuel => {
                self.controller.step = SystemStep::AblupShowSelect;
                self.dispatch_system_function(vm, "SHOW_ABLUP_SELECT", true)?;
            }
            SystemStep::AblupAction => {
                if self.presentation.last_line_is_temporary() {
                    self.command_intents
                        .clone_from(&self.reusable_system_intents);
                    return self.open_system_command_wait();
                }
                self.controller.step = SystemStep::AblupShowJuel;
                self.dispatch_system_function(vm, "SHOW_JUEL", true)?;
            }
            SystemStep::ShopEvent => {
                if self
                    .project_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.auto_save)
                    && self.controller.shop_called_when_normal
                {
                    self.controller.step = SystemStep::ShopAutosave;
                    if !self.dispatch_system_function(vm, "SYSTEM_AUTOSAVE", false)? {
                        return self.begin_candidate_save(
                            vm,
                            99,
                            CandidateSaveContinuation::Autosave,
                        );
                    }
                } else {
                    if let Some(flow) = self.controller.deferred_flow.take() {
                        self.controller.flow = Some(flow);
                        return self.begin_flow(vm, flow);
                    }
                    self.controller.step = SystemStep::ShopShow;
                    self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
                }
            }
            SystemStep::ShopAutosave | SystemStep::ShopAction | SystemStep::PostLoadShop => {
                if let Some(flow) = self.controller.deferred_flow.take() {
                    self.controller.flow = Some(flow);
                    return self.begin_flow(vm, flow);
                }
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
            }
            SystemStep::TitleLoadOverride => {
                self.controller.step = SystemStep::None;
                return self.open_title_menu();
            }
            SystemStep::TrainCallTrainEnd => return self.finish_event_com_end(vm),
            SystemStep::TrainBeginAfterCallTrainEnd => {
                self.skip_print = false;
                let flow = self.controller.deferred_flow.take().ok_or_else(|| {
                    RuntimeError::Internal("deferred BEGIN target disappeared".into())
                })?;
                self.controller.flow = Some(flow);
                return self.begin_flow(vm, flow);
            }
            SystemStep::TrainEventComEndWait
            | SystemStep::ShopAutosaveFailureWait
            | SystemStep::None => {}
        }
        Ok(())
    }

    pub(super) fn prepare_next_comable(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        let names = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .name_tables
            .get(&erabasic_data::NameTableKind::Train)
            .map(|table| table.names.clone())
            .unwrap_or_default();
        let default_enabled = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .replace
            .com_able_default
            != 0;
        while self.controller.train_scan < names.len() {
            let command = self.controller.train_scan;
            self.controller.train_scan += 1;
            if names[command].is_none() {
                continue;
            }
            if self.dispatch_system_function(vm, &format!("COM_ABLE{command}"), false)? {
                return Ok(());
            }
            if default_enabled {
                self.controller
                    .train_commands
                    .push(i64::try_from(command).unwrap_or(i64::MAX));
            }
        }
        if !self.controller.continuous_train {
            for (display, command) in self
                .controller
                .train_commands
                .clone()
                .into_iter()
                .enumerate()
            {
                let name = usize::try_from(command)
                    .ok()
                    .and_then(|index| names.get(index))
                    .and_then(Option::as_deref)
                    .unwrap_or("");
                let token = self.allocate_interaction();
                let display = i64::try_from(display).unwrap_or(i64::MAX);
                self.presentation.append_button(
                    format!("{name}[{display:>3}]"),
                    era_runtime_protocol::ProtocolValue::Integer(display),
                    token,
                    None,
                );
                self.command_intents
                    .insert(token, VmValue::Integer(display));
            }
        }
        // The reference system flushes the partial SHOW_STATUS/COM_ABLE row before
        // entering SHOW_USERCOM. In eraTW this separates the final PALAM entry from
        // the following Look section without requiring a script-specific newline.
        self.presentation.flush_pending_line();
        self.controller.step = SystemStep::TrainShowUser;
        self.dispatch_system_function(vm, "SHOW_USERCOM", true)?;
        Ok(())
    }

    fn finish_continuous_train(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        self.controller.clear_continuous_train();
        self.skip_print = false;
        self.controller.step = SystemStep::TrainCallTrainEnd;
        if !self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
            return self.finish_event_com_end(vm);
        }
        Ok(())
    }

    fn finish_event_com_end(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        if self.controller.event_com_end_wait_required {
            self.controller.event_com_end_wait_required = false;
            self.controller.step = SystemStep::TrainEventComEndWait;
            let submission = self.allocate_interaction();
            let mut wait = self.system_wait(submission);
            wait.kind = WaitKind::EnterKey;
            wait.mouse_input = false;
            wait.default_value = None;
            return self.open_wait(
                PendingInput {
                    host_request: None,
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
                    timeout_duration_ns: None,
                    post_input: None,
                },
                true,
            );
        }
        self.controller.step = SystemStep::TrainShowStatus;
        self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
        Ok(())
    }

    pub(super) fn open_wait(
        &mut self,
        pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.operations.active_input().is_some() {
            self.operations.queue_input(pending);
            return Ok(());
        }
        self.activate_wait(pending, pause_runtime)
    }

    pub(super) fn activate_wait(
        &mut self,
        mut pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.restart_queued_input_undo()? {
            return Ok(());
        }
        if let Some(duration) = pending.timeout_duration_ns {
            pending.wait.deadline_ns = Some(self.logical_time_ns.saturating_add(duration));
            if pending.wait.display_time {
                pending.wait.countdown_remaining_ms = Some(duration / 1_000_000);
            }
        }
        if let Some(submission) = self.replay_submission(&pending.wait) {
            self.operations.activate_input(pending);
            return self.finish_input(submission, false);
        }
        if self.undo_replay.is_none() {
            self.undo_token = None;
            self.emit_input_undo_state()?;
        }
        let automatic_system_value = (pending.wait.system_input
            && (self.flow_input_force_skip || (self.flow_input_can_skip && self.message_skip)))
            .then(|| {
                if self.flow_input_string {
                    VmValue::String(self.flow_input_default_string.clone())
                } else {
                    VmValue::Integer(self.flow_input_default)
                }
            });
        let count = self.presentation.pending_auto_button_values().len();
        let tokens = (0..count)
            .map(|_| self.allocate_interaction())
            .collect::<Vec<_>>();
        for (token, value) in self.presentation.bind_pending_auto_buttons(&tokens) {
            self.command_intents.insert(token, VmValue::Integer(value));
        }
        // Emuera flushes its print buffer whenever script execution yields for input.
        // Keeping that boundary canonical prevents output after a resumed INPUT from
        // being appended to the menu line that opened the wait.
        self.presentation.flush_pending_line();
        self.presentation.set_wait(Some(pending.wait.clone()));
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Opened(pending.wait.clone())),
            None,
        )?;
        self.operations.activate_input(pending);
        self.emit_presentation()?;
        if let Some(value) = automatic_system_value {
            return self.finish_input(InputSubmission::Value(value), false);
        }
        if pause_runtime {
            self.set_phase(RuntimePhase::WaitingInput)
        } else {
            Ok(())
        }
    }

    pub(super) fn close_wait(&mut self, wait_id: u64) -> Result<(), RuntimeError> {
        self.presentation.set_wait(None);
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)),
            None,
        )?;
        self.emit_presentation()
    }
}
