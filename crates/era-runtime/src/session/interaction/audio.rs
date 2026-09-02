#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in crate::session) fn has_audio_observation_service(&self) -> bool {
        self.service_capabilities
            .iter()
            .any(|((kind, operation), version)| {
                *kind == ServiceKind::Audio
                    && operation == AUDIO_OBSERVATION_OPERATION
                    && *version == AUDIO_OBSERVATION_OPERATION_VERSION
            })
    }

    pub(in crate::session) fn issue_audio_observation(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        channel: AudioChannelV1,
        purpose: AudioObservationPurpose,
    ) -> Result<(), RuntimeError> {
        if !self.has_audio_observation_service() {
            self.emit_audio_observation_unavailable(request)?;
            return complete_script_fault(
                vm,
                request,
                erabasic_vm::ScriptFaultKind::Operation,
                "frontend audio observation service is unavailable",
            );
        }
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.queue_audio_observation(AudioObservationContinuation {
            request: request.id,
            observation: AudioObservationRequestV1 {
                channel,
                expected_revision: self.audio.expected(channel),
            },
            purpose,
        })
    }

    fn emit_audio_observation_unavailable(
        &mut self,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                context: Some(Box::new(
                    era_runtime_protocol::CompatibilityDiagnosticContext {
                        artifact: None,
                        project_load_id: None,
                        runtime_epoch: Some(self.epoch.0),
                        generation: None,
                        identity: self
                            .project_snapshot
                            .as_ref()
                            .map(|project| project.manifest.compatibility.clone()),
                        stage: "service".into(),
                        api: Some(request.import.import.name.clone()),
                        required_capability: Some(era_runtime_protocol::RequiredCapability {
                            kind: ServiceKind::Audio,
                            operation: AUDIO_OBSERVATION_OPERATION.into(),
                            version: AUDIO_OBSERVATION_OPERATION_VERSION,
                        }),
                    },
                )),
                code: "runtime.audio_observation_unavailable".into(),
                level: RuntimeLogLevel::Error,
                message: "frontend did not negotiate audio_observation@1".into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        )
    }

    pub(in crate::session) fn queue_audio_observation(
        &mut self,
        continuation: AudioObservationContinuation,
    ) -> Result<(), RuntimeError> {
        let request_id = self.allocate_request()?;
        let payload = ProtocolBytes::new(encode_canonical(&continuation.observation)?);
        self.operations
            .insert_service(request_id, PendingService::Audio(continuation));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind: ServiceKind::Audio,
                operation: AUDIO_OBSERVATION_OPERATION.into(),
                operation_version: AUDIO_OBSERVATION_OPERATION_VERSION,
                payload,
                deadline_ns: None,
            }),
            None,
        )
    }

    pub(in crate::session) fn play_sound_on_channel(
        &mut self,
        channel: u8,
        resource_id: String,
        repeat_count: i64,
    ) -> Result<(), RuntimeError> {
        let revision = self.audio.next_revision(0);
        if self.emit_audio_effects(vec![play_effect(
            AudioChannelV1::Sound(channel),
            resource_id,
            repeat_count.max(1),
            self.presentation.sound_volume_millionths(),
            revision,
        )])? {
            self.audio.commit_sound(channel, revision);
        }
        Ok(())
    }

    pub(in crate::session) fn emit_audio_effects(
        &mut self,
        effects: Vec<AudioEffect>,
    ) -> Result<bool, RuntimeError> {
        if self.presentation.projects_audio() && self.client_audio_available {
            self.emit_effects(effects.into_iter().map(EffectKind::Audio).collect())?;
            return Ok(true);
        }
        self.emit_audio_unavailable()?;
        Ok(false)
    }

    pub(in super::super) fn complete_audio_observation(
        &mut self,
        continuation: AudioObservationContinuation,
        result: ServiceResult,
    ) -> Result<(), RuntimeError> {
        let Some(response) =
            self.decode_and_validate_audio_observation(continuation.observation, result)?
        else {
            return Ok(());
        };

        if matches!(
            &continuation.purpose,
            AudioObservationPurpose::SelectSound { .. }
        ) {
            return self.complete_sound_selection(continuation, response);
        }

        let values = audio_observation_values(response);
        let (value, writes) = match continuation.purpose {
            AudioObservationPurpose::GetInfo { selector: None } => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("audio observation completion has no VM".into())
                })?;
                let writes = values
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| {
                        global_place_at(vm, "RESULT", index).map(|target| HostWrite {
                            target,
                            value: VmValue::Integer(*value),
                        })
                    })
                    .collect();
                (values[0], writes)
            }
            AudioObservationPurpose::GetInfo {
                selector: Some(selector),
            } => (
                values[usize::try_from(selector - 1).unwrap_or_default()],
                Vec::new(),
            ),
            AudioObservationPurpose::IsPlayingSound => {
                let AudioChannelV1::Sound(channel) = response.channel else {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "ISPLAYINGSOUND received a BGM observation",
                        None,
                    );
                };
                (
                    if response.state == AudioPlaybackStateV1::Playing {
                        i64::from(channel)
                    } else {
                        -1
                    },
                    Vec::new(),
                )
            }
            AudioObservationPurpose::IsPlayingBgm => (
                i64::from(response.state == AudioPlaybackStateV1::Playing),
                Vec::new(),
            ),
            AudioObservationPurpose::SelectSound { .. } => {
                unreachable!("sound selection handled above")
            }
        };
        let vm = self.vm.as_mut().ok_or_else(|| {
            RuntimeError::Internal("audio observation completion has no VM".into())
        })?;
        commit_completion(
            vm,
            continuation.request,
            VmHostCompletion::Ready(HostReady {
                value: Some(VmValue::Integer(value)),
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    fn complete_sound_selection(
        &mut self,
        continuation: AudioObservationContinuation,
        response: AudioObservationResponseV1,
    ) -> Result<(), RuntimeError> {
        let AudioObservationPurpose::SelectSound {
            resource_id,
            repeat_count,
        } = continuation.purpose
        else {
            unreachable!("sound selection helper requires a selection continuation")
        };
        let AudioChannelV1::Sound(channel) = response.channel else {
            return self.fault(
                FaultCode::ServiceFailure,
                "sound allocation received a BGM observation",
                None,
            );
        };
        if response.state == AudioPlaybackStateV1::Playing
            && channel + 1 < era_runtime_protocol::AUDIO_SOUND_CHANNEL_COUNT
        {
            let next = channel + 1;
            return self.queue_audio_observation(AudioObservationContinuation {
                request: continuation.request,
                observation: AudioObservationRequestV1 {
                    channel: AudioChannelV1::Sound(next),
                    expected_revision: self.audio.expected(AudioChannelV1::Sound(next)),
                },
                purpose: AudioObservationPurpose::SelectSound {
                    resource_id,
                    repeat_count,
                },
            });
        }
        let channel = if response.state == AudioPlaybackStateV1::Playing {
            0
        } else {
            channel
        };
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("audio selection completion has no VM".into()))?;
        commit_completion(
            vm,
            continuation.request,
            VmHostCompletion::Ready(HostReady::empty()),
        )?;
        self.play_sound_on_channel(channel, resource_id, repeat_count)?;
        self.set_phase(RuntimePhase::Running)
    }

    fn decode_and_validate_audio_observation(
        &mut self,
        observation: AudioObservationRequestV1,
        result: ServiceResult,
    ) -> Result<Option<AudioObservationResponseV1>, RuntimeError> {
        let payload = match result {
            ServiceResult::Ready { payload } => payload,
            ServiceResult::Error { error } => {
                self.fault(
                    FaultCode::ServiceFailure,
                    &format!(
                        "audio observation failed: {}: {}",
                        error.code, error.message
                    ),
                    None,
                )?;
                return Ok(None);
            }
        };
        let response: AudioObservationResponseV1 = match decode_canonical(payload.as_slice()) {
            Ok(response) => response,
            Err(error) => {
                self.fault(
                    FaultCode::ServiceFailure,
                    &format!("invalid audio_observation response: {error}"),
                    None,
                )?;
                return Ok(None);
            }
        };
        if !response.is_fresh_for(observation) {
            self.fault(
                FaultCode::ServiceFailure,
                "stale or mismatched audio observation response",
                None,
            )?;
            return Ok(None);
        }
        if !self
            .audio
            .record_observation(response.channel, response.frontend_monotonic_time_ns)
        {
            self.fault(
                FaultCode::ServiceFailure,
                "audio observation timestamp moved backwards",
                None,
            )?;
            return Ok(None);
        }
        Ok(Some(response))
    }
}

fn audio_observation_values(response: AudioObservationResponseV1) -> [i64; 5] {
    [
        i64::try_from(response.duration_ms).unwrap_or(i64::MAX),
        i64::try_from(response.position_ms).unwrap_or(i64::MAX),
        i64::from(response.state == AudioPlaybackStateV1::Playing),
        i64::from(response.volume_millionths / 10_000),
        i64::from(response.rate_millionths / 10_000),
    ]
}
