use super::*;
use era_runtime_protocol::AudioEffectAction;

fn audio_capabilities(observation: bool) -> ClientCapabilities {
    let mut capabilities = capabilities();
    capabilities.audio = true;
    if observation {
        capabilities.services.push(ServiceCapability {
            kind: ServiceKind::Audio,
            operation: AUDIO_OBSERVATION_OPERATION.into(),
            versions: VersionRange::exact(AUDIO_OBSERVATION_OPERATION_VERSION),
        });
    }
    capabilities
}

fn start_audio_project(source: &str, observation: bool, resources: &[&str]) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "audio-runtime-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: audio_capabilities(observation),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);

    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let mut files = vec![
        profile_configuration_file(snake.profile),
        SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(source.into()),
            content_hash: None,
        },
    ];
    files.extend(resources.iter().map(|path| SubmittedFile {
        relative_path: (*path).into(),
        category: FileCategory::Resource,
        payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3, 4])),
        content_hash: None,
    }));
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: snake,
            project_revision: 1,
            files,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    ));
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    session
}

fn next_audio_request(session: &mut RuntimeSession) -> ServiceRequest {
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if let Some(request) = drain(session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::Audio
                        && request.operation == AUDIO_OBSERVATION_OPERATION =>
                {
                    Some(request)
                }
                _ => None,
            })
        {
            return request;
        }
    }
    panic!("audio observation request was not emitted");
}

fn decode_audio_request(request: &ServiceRequest) -> AudioObservationRequestV1 {
    decode_canonical(request.payload.as_slice()).unwrap()
}

#[allow(
    clippy::too_many_arguments,
    reason = "the helper names every independently asserted wire observation field"
)]
fn respond_audio(
    session: &mut RuntimeSession,
    sequence: u64,
    request: &ServiceRequest,
    state: AudioPlaybackStateV1,
    duration_ms: u64,
    position_ms: u64,
    volume_millionths: u32,
    rate_millionths: u32,
) {
    respond_audio_at(
        session,
        sequence,
        request,
        state,
        duration_ms,
        position_ms,
        volume_millionths,
        rate_millionths,
        sequence * 1_000,
    );
}

#[allow(clippy::too_many_arguments)]
fn respond_audio_at(
    session: &mut RuntimeSession,
    sequence: u64,
    request: &ServiceRequest,
    state: AudioPlaybackStateV1,
    duration_ms: u64,
    position_ms: u64,
    volume_millionths: u32,
    rate_millionths: u32,
    frontend_monotonic_time_ns: u64,
) {
    let observation = decode_audio_request(request);
    submit(
        session,
        sequence,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&AudioObservationResponseV1 {
                        channel: observation.channel,
                        revision: observation.expected_revision,
                        duration_ms,
                        position_ms,
                        state,
                        volume_millionths,
                        rate_millionths,
                        preserve_pitch: true,
                        frontend_monotonic_time_ns,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
}

fn audio_effects(messages: &[RuntimeMessage]) -> Vec<&AudioEffect> {
    messages
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::EffectBatch(batch) => Some(&batch.effects),
            _ => None,
        })
        .flatten()
        .filter_map(|effect| match &effect.kind {
            EffectKind::Audio(audio) => Some(audio),
            _ => None,
        })
        .collect()
}

#[test]
fn playsound_selects_paused_channel_and_clamps_repeat_to_one() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nPLAYSOUND \"tone.wav\", 0\nWAIT\nRETURN\n",
        true,
        &["sound/tone.wav"],
    );
    let request0 = next_audio_request(&mut session);
    assert_eq!(
        decode_audio_request(&request0),
        AudioObservationRequestV1 {
            channel: AudioChannelV1::Sound(0),
            expected_revision: 0,
        }
    );
    respond_audio(
        &mut session,
        3,
        &request0,
        AudioPlaybackStateV1::Playing,
        1_000,
        100,
        1_000_000,
        1_000_000,
    );
    let request1 = next_audio_request(&mut session);
    assert_eq!(
        decode_audio_request(&request1).channel,
        AudioChannelV1::Sound(1)
    );
    respond_audio(
        &mut session,
        4,
        &request1,
        AudioPlaybackStateV1::Paused,
        1_000,
        200,
        1_000_000,
        1_000_000,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let effects = audio_effects(&messages);
    assert!(effects.iter().any(|effect| {
        effect.channel == AudioChannelV1::Sound(1)
            && effect.action == AudioEffectAction::Play
            && effect.resource_id.as_deref() == Some("sound/tone.wav")
            && effect.repeat_count == 1
            && effect.revision > 0
    }));
    assert!(session.presentation.snapshot().audio.is_empty());
}

#[test]
fn playsound_overwrites_channel_zero_when_all_channels_are_playing() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nPLAYSOUND \"tone.wav\", 3\nWAIT\nRETURN\n",
        true,
        &["sound/tone.wav"],
    );
    for channel in 0..era_runtime_protocol::AUDIO_SOUND_CHANNEL_COUNT {
        let request = next_audio_request(&mut session);
        assert_eq!(
            decode_audio_request(&request).channel,
            AudioChannelV1::Sound(channel)
        );
        respond_audio(
            &mut session,
            u64::from(channel) + 3,
            &request,
            AudioPlaybackStateV1::Playing,
            1_000,
            100,
            1_000_000,
            1_000_000,
        );
    }
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let effects = audio_effects(&messages);
    assert!(effects.iter().any(|effect| {
        effect.channel == AudioChannelV1::Sound(0)
            && effect.action == AudioEffectAction::Play
            && effect.repeat_count == 3
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn audio_queries_write_exact_results_and_controls_emit_revisioned_effects() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT:10 = GETSOUNDORBGMINFO(3, 3)\nRESULT:11 = ISPLAYINGSOUND(3)\nRESULT:12 = GETSOUNDORBGMINFO(3)\nRESULT:20 = RESULT:0\nRESULT:21 = RESULT:1\nRESULT:22 = RESULT:2\nRESULT:23 = RESULT:3\nRESULT:24 = RESULT:4\nRESULT:25 = ISPLAYINGBGM()\nRESULT:30 = SOUNDCONTROL(3, 3, 1, 1)\nRESULT:31 = SOUNDCONTROL(-1, 0)\nRESULT:32 = SOUNDCONTROL(3, 9)\nRESULT:33 = BGMCONTROL(0)\nRESULT:34 = BGMCONTROL(3, 2000, 0)\nWAIT\nRETURN\n",
        true,
        &[],
    );
    let query = next_audio_request(&mut session);
    respond_audio(
        &mut session,
        3,
        &query,
        AudioPlaybackStateV1::Playing,
        1_200,
        250,
        370_000,
        2_500_000,
    );
    let is_sound = next_audio_request(&mut session);
    respond_audio(
        &mut session,
        4,
        &is_sound,
        AudioPlaybackStateV1::Playing,
        1_200,
        260,
        370_000,
        2_500_000,
    );
    let omitted = next_audio_request(&mut session);
    respond_audio(
        &mut session,
        5,
        &omitted,
        AudioPlaybackStateV1::Paused,
        1_200,
        300,
        370_000,
        2_500_000,
    );
    let bgm = next_audio_request(&mut session);
    assert_eq!(decode_audio_request(&bgm).channel, AudioChannelV1::Bgm);
    respond_audio(
        &mut session,
        6,
        &bgm,
        AudioPlaybackStateV1::Stopped,
        0,
        0,
        1_000_000,
        1_000_000,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let vm = session.vm.as_ref().unwrap();
    for (index, expected) in [
        (10, 1),
        (11, 3),
        (12, 1_200),
        (20, 1_200),
        (21, 300),
        (22, 0),
        (23, 37),
        (24, 250),
        (25, 0),
        (30, 1),
        (31, -1),
        (32, -2),
        (33, 1),
        (34, 1),
    ] {
        assert_eq!(
            read_runtime_integer(vm, "RESULT", &[index], None).unwrap(),
            expected,
            "RESULT:{index}"
        );
    }
    let effects = audio_effects(&messages);
    assert!(effects.iter().any(|effect| {
        effect.channel == AudioChannelV1::Sound(3)
            && effect.action == AudioEffectAction::SetRate
            && effect.rate_millionths == 100_000
            && !effect.preserve_pitch
    }));
    assert!(effects.iter().any(|effect| {
        effect.channel == AudioChannelV1::Bgm
            && effect.action == AudioEffectAction::SetRate
            && effect.rate_millionths == 10_000_000
            && effect.preserve_pitch
    }));
}

#[test]
fn bgm_controls_preserve_recoverable_expected_state() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nPLAYBGM \"theme.wav\"\nRESULT:10 = BGMCONTROL(0)\nRESULT:11 = BGMCONTROL(3, 250, 1)\nWAIT\nRETURN\n",
        false,
        &["sound/theme.wav"],
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let audio = session.presentation.snapshot().audio;
    assert_eq!(audio.len(), 1);
    assert_eq!(audio[0].channel, AudioChannelV1::Bgm);
    assert_eq!(audio[0].state, AudioPlaybackStateV1::Paused);
    assert_eq!(audio[0].rate_millionths, 2_500_000);
    assert!(!audio[0].preserve_pitch);
    assert!(audio[0].revision > 0);
}

#[test]
fn missing_audio_observation_service_emits_stable_diagnostic_and_script_fault() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT = ISPLAYINGBGM()\nWAIT\nRETURN\n",
        false,
        &[],
    );
    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::Faulted {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.audio_observation_unavailable"
                && diagnostic.level == RuntimeLogLevel::Error
    )));
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(fault) if fault.code == FaultCode::VmFault
    )));
}

#[test]
fn stale_audio_observation_revision_is_rejected() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT = ISPLAYINGBGM()\nWAIT\nRETURN\n",
        true,
        &[],
    );
    let request = next_audio_request(&mut session);
    let observation = decode_audio_request(&request);
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&AudioObservationResponseV1 {
                        channel: observation.channel,
                        revision: observation.expected_revision + 1,
                        duration_ms: 0,
                        position_ms: 0,
                        state: AudioPlaybackStateV1::Stopped,
                        volume_millionths: 1_000_000,
                        rate_millionths: 1_000_000,
                        preserve_pitch: true,
                        frontend_monotonic_time_ns: 1,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(fault)
            if fault.code == FaultCode::ServiceFailure
                && fault.message.contains("stale or mismatched audio")
    )));
}

#[test]
fn audio_info_selectors_and_playback_states_are_independent() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT:10 = GETSOUNDORBGMINFO(2, 1)\nRESULT:11 = GETSOUNDORBGMINFO(2, 2)\nRESULT:12 = GETSOUNDORBGMINFO(2, 3)\nRESULT:13 = GETSOUNDORBGMINFO(2, 4)\nRESULT:14 = GETSOUNDORBGMINFO(2, 5)\nRESULT:20 = ISPLAYINGSOUND(0)\nRESULT:21 = ISPLAYINGSOUND(1)\nRESULT:22 = ISPLAYINGSOUND(2)\nRESULT:23 = GETSOUNDORBGMINFO(99, 1)\nRESULT:24 = GETSOUNDORBGMINFO(0, 9)\nWAIT\nRETURN\n",
        true,
        &[],
    );
    for (sequence, state) in [
        (3, AudioPlaybackStateV1::Playing),
        (4, AudioPlaybackStateV1::Playing),
        (5, AudioPlaybackStateV1::Paused),
        (6, AudioPlaybackStateV1::Playing),
        (7, AudioPlaybackStateV1::Playing),
        (8, AudioPlaybackStateV1::Playing),
        (9, AudioPlaybackStateV1::Paused),
        (10, AudioPlaybackStateV1::Stopped),
    ] {
        let request = next_audio_request(&mut session);
        respond_audio(
            &mut session,
            sequence,
            &request,
            state,
            9_001,
            321,
            420_000,
            1_750_000,
        );
    }
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let vm = session.vm.as_ref().unwrap();
    for (index, expected) in [
        (10, 9_001),
        (11, 321),
        (12, 0),
        (13, 42),
        (14, 175),
        (20, 0),
        (21, -1),
        (22, -1),
        (23, 0),
        (24, 0),
    ] {
        assert_eq!(
            read_runtime_integer(vm, "RESULT", &[index], None).unwrap(),
            expected,
            "RESULT:{index}"
        );
    }
}

#[test]
fn sound_and_bgm_controls_cover_actions_clamps_and_pitch_mapping() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT:0 = SOUNDCONTROL(0, 0)\nRESULT:1 = SOUNDCONTROL(1, 1)\nRESULT:2 = SOUNDCONTROL(2, 2)\nRESULT:3 = SOUNDCONTROL(3, 3, 1)\nRESULT:4 = SOUNDCONTROL(4, 3, 500, 0)\nRESULT:5 = SOUNDCONTROL(5, 3, 500, 1)\nRESULT:6 = BGMCONTROL(0)\nRESULT:7 = BGMCONTROL(1)\nRESULT:8 = BGMCONTROL(2)\nWAIT\nRETURN\n",
        false,
        &[],
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let effects = audio_effects(&messages);
    for (channel, action) in [
        (0, AudioEffectAction::Pause),
        (1, AudioEffectAction::Resume),
        (2, AudioEffectAction::Stop),
    ] {
        assert!(
            effects
                .iter()
                .any(|effect| effect.channel == AudioChannelV1::Sound(channel)
                    && effect.action == action)
        );
        assert!(
            effects
                .iter()
                .any(|effect| effect.channel == AudioChannelV1::Bgm && effect.action == action)
        );
    }
    assert!(
        effects
            .iter()
            .any(|effect| effect.channel == AudioChannelV1::Sound(3)
                && effect.rate_millionths == 100_000
                && effect.preserve_pitch)
    );
    assert!(
        effects
            .iter()
            .any(|effect| effect.channel == AudioChannelV1::Sound(4)
                && effect.rate_millionths == 5_000_000
                && effect.preserve_pitch)
    );
    assert!(
        effects
            .iter()
            .any(|effect| effect.channel == AudioChannelV1::Sound(5)
                && effect.rate_millionths == 5_000_000
                && !effect.preserve_pitch)
    );
    let vm = session.vm.as_ref().unwrap();
    for index in 0..9 {
        assert_eq!(
            read_runtime_integer(vm, "RESULT", &[index], None).unwrap(),
            1
        );
    }
}

#[test]
fn every_observation_api_reports_capability_context_when_unavailable() {
    for (api, source) in [
        ("getsoundorbgminfo", "RESULT = GETSOUNDORBGMINFO(0, 1)"),
        ("isplayingsound", "RESULT = ISPLAYINGSOUND(0)"),
        ("isplayingbgm", "RESULT = ISPLAYINGBGM()"),
    ] {
        let mut session =
            start_audio_project(&format!("@SYSTEM_TITLE\n{source}\nRETURN\n"), false, &[]);
        let mut messages = Vec::new();
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            messages.extend(drain(&mut session));
            if session.phase() == RuntimePhase::Faulted {
                break;
            }
        }
        assert!(messages.iter().any(|message| matches!(message,
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "runtime.audio_observation_unavailable"
                    && diagnostic.context.as_ref().and_then(|context| context.api.as_deref()) == Some(api)
                    && diagnostic.context.as_ref().and_then(|context| context.required_capability.as_ref()).is_some_and(|required| required.kind == ServiceKind::Audio && required.operation == AUDIO_OBSERVATION_OPERATION)
        )), "{api}: {messages:#?}");
    }
}

#[test]
fn unavailable_device_preserves_control_codes_and_warns_once_per_call() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT:0 = SOUNDCONTROL(0, 0)\nRESULT:1 = SOUNDCONTROL(1, 1)\nRESULT:2 = BGMCONTROL(0)\nRESULT:3 = BGMCONTROL(1)\nWAIT\nRETURN\n",
        false,
        &[],
    );
    session.client_audio_available = false;
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(messages.iter().filter(|message| matches!(message,
        RuntimeMessage::Diagnostic(diagnostic) if diagnostic.code == "runtime.audio_device_unavailable"
    )).count(), 4);
    let vm = session.vm.as_ref().unwrap();
    for index in 0..4 {
        assert_eq!(
            read_runtime_integer(vm, "RESULT", &[index], None).unwrap(),
            1
        );
    }
    assert_eq!(session.audio.expected(AudioChannelV1::Sound(0)), 0);
    assert_eq!(session.audio.expected(AudioChannelV1::Sound(1)), 0);
}

#[test]
fn mismatched_invalid_and_backwards_observations_never_commit_vm_results() {
    for channel in [AudioChannelV1::Sound(1), AudioChannelV1::Sound(10)] {
        let mut session = start_audio_project(
            "@SYSTEM_TITLE\nRESULT:7 = ISPLAYINGSOUND(0)\nRETURN\n",
            true,
            &[],
        );
        let request = next_audio_request(&mut session);
        let observation = decode_audio_request(&request);
        submit(
            &mut session,
            3,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: request.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&AudioObservationResponseV1 {
                            channel,
                            revision: observation.expected_revision,
                            duration_ms: 0,
                            position_ms: 0,
                            state: AudioPlaybackStateV1::Playing,
                            volume_millionths: 1_000_000,
                            rate_millionths: 1_000_000,
                            preserve_pitch: true,
                            frontend_monotonic_time_ns: 1,
                        })
                        .unwrap(),
                    ),
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[7], None).unwrap(),
            0
        );
    }

    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nRESULT:0 = ISPLAYINGSOUND(0)\nRESULT:1 = ISPLAYINGSOUND(0)\nRETURN\n",
        true,
        &[],
    );
    let first = next_audio_request(&mut session);
    respond_audio_at(
        &mut session,
        3,
        &first,
        AudioPlaybackStateV1::Stopped,
        0,
        0,
        0,
        1_000_000,
        20,
    );
    let second = next_audio_request(&mut session);
    respond_audio_at(
        &mut session,
        4,
        &second,
        AudioPlaybackStateV1::Playing,
        0,
        0,
        0,
        1_000_000,
        19,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(message,
        RuntimeMessage::Fault(fault) if fault.message.contains("timestamp moved backwards")
    )));
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        0
    );
}

#[test]
fn resynchronization_discards_sound_effects_but_retains_bgm_recovery() {
    let mut session = start_audio_project(
        "@SYSTEM_TITLE\nPLAYBGM \"theme.wav\"\nPLAYSOUND \"tone.wav\"\nSETSOUNDVOLUME 25\nRESULT = SOUNDCONTROL(0, 0)\nSTOPSOUND\nWAIT\nRETURN\n",
        false,
        &["sound/theme.wav", "sound/tone.wav"],
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    assert!(session.effect_journal.values().any(|event| matches!(
        &event.kind,
        EffectKind::Audio(AudioEffect {
            channel: AudioChannelV1::Sound(_),
            ..
        })
    )));
    assert!(session.effect_journal.values().any(|event| matches!(
        &event.kind,
        EffectKind::Audio(AudioEffect {
            channel: AudioChannelV1::Bgm,
            ..
        })
    )));
    for action in [
        AudioEffectAction::Play,
        AudioEffectAction::SetVolume,
        AudioEffectAction::Pause,
        AudioEffectAction::Stop,
    ] {
        assert!(session.effect_journal.values().any(|event| matches!(&event.kind,
            EffectKind::Audio(AudioEffect { channel: AudioChannelV1::Sound(_), action: actual, .. }) if *actual == action
        )), "{action:?}");
    }

    submit(
        &mut session,
        3,
        RuntimeMessage::Resynchronize(era_runtime_protocol::ResynchronizeRequest {
            after_sequence: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let replayed = drain(&mut session);
    let effects = audio_effects(&replayed);
    assert!(
        effects
            .iter()
            .all(|effect| effect.channel == AudioChannelV1::Bgm)
    );
    assert!(session.effect_journal.values().all(|event| !matches!(
        &event.kind,
        EffectKind::Audio(AudioEffect {
            channel: AudioChannelV1::Sound(_),
            ..
        })
    )));
    assert_eq!(session.audio.expected(AudioChannelV1::Sound(0)), 0);
    assert_eq!(
        session.audio.expected(AudioChannelV1::Bgm),
        session.presentation.bgm_revision().unwrap()
    );
}

#[test]
fn effect_batches_fail_without_partial_journal_or_identifier_mutation() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.options.limits.maximum_journal_entries = 9;
    let next_effect_id = session.next_effect_id;
    let result = session.emit_effects(
        (0..era_runtime_protocol::AUDIO_SOUND_CHANNEL_COUNT)
            .map(|channel| EffectKind::Audio(stop_effect(AudioChannelV1::Sound(channel), 1)))
            .collect(),
    );
    assert!(matches!(
        result,
        Err(RuntimeError::ResourceLimit("effect journal is full"))
    ));
    assert!(session.effect_journal.is_empty());
    assert_eq!(session.next_effect_id, next_effect_id);
    assert!(session.outbound.is_empty());
}
