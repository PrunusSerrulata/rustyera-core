pub(super) fn start_input_project(source: &str) -> RuntimeSession {
    start_input_project_with(
        source,
        era_runtime_protocol::CompatibilityIdentity::default(),
        capabilities(),
        false,
    )
}

pub(super) fn start_snake_input_project(
    source: &str,
    client: ClientCapabilities,
) -> RuntimeSession {
    start_input_project_with(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        client,
        false,
    )
}

fn start_input_project_with(
    source: &str,
    identity: era_runtime_protocol::CompatibilityIdentity,
    client: ClientCapabilities,
    undo: bool,
) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "input-set-test".into(),
            features: if undo {
                vec![RuntimeFeature::TimedInput, RuntimeFeature::InputUndo]
            } else {
                vec![RuntimeFeature::TimedInput]
            },
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let mut config = profile_configuration_file(identity.profile);
    if undo && let FilePayload::Utf8(text) = &mut config.payload {
        text.push_str("[input]\nundo_enabled = true\n");
    }
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: identity,
            project_revision: 1,
            files: vec![
                config,
                SubmittedFile {
                    relative_path: "input-set.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    drive_until_wait(&mut session);
    session
}

fn submit_text(session: &mut RuntimeSession, sequence: u64, text: &str) {
    let pending = session.operations.active_input().expect("active input");
    submit_frontend_input(
        session,
        sequence,
        pending.wait.wait_id,
        pending.wait.submission_token,
        InputIntent::CommitText(text.into()),
    );
}

fn submit_frontend_input(
    session: &mut RuntimeSession,
    sequence: u64,
    wait_id: u64,
    token: InteractionToken,
    intent: InputIntent,
) {
    submit(
        session,
        sequence,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 0,
            intent,
            message_skip: false,
        }),
    );
}

fn drive_input_set(session: &mut RuntimeSession) {
    for _ in 0..128 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput && session.queued_input.is_empty() {
            return;
        }
    }
    panic!("input set did not reach its final wait");
}

fn drive_until_wait(session: &mut RuntimeSession) {
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput
            && session.operations.active_input().is_some()
        {
            return;
        }
    }
    panic!("project did not reach its first wait");
}

fn assert_wait(session: &RuntimeSession, kind: WaitKind) {
    assert_eq!(
        session
            .operations
            .active_input()
            .map(|pending| pending.wait.kind),
        Some(kind)
    );
}

fn queued_text(session: &RuntimeSession) -> Option<&str> {
    session
        .queued_input
        .front()
        .map(|segment| segment.text.as_str())
}

fn assert_rejection(session: &mut RuntimeSession, code: CommandErrorCode) {
    assert!(drain(session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(value) if value.code == code
    )));
}

fn single_transition_budget() -> RuntimeDriveBudget {
    RuntimeDriveBudget {
        maximum_runtime_transitions: 1,
        ..RuntimeDriveBudget::default()
    }
}

fn runtime_integer(session: &RuntimeSession, name: &str) -> i64 {
    read_runtime_integer(session.vm.as_ref().expect("runtime VM"), name, &[], None).unwrap()
}

fn runtime_string(session: &RuntimeSession, name: &str) -> String {
    read_runtime_string(session.vm.as_ref().expect("runtime VM"), name).unwrap()
}

fn snake_input_capabilities() -> ClientCapabilities {
    let mut client = capabilities();
    client.environment = [
        INPUT_DEVICE_LATCH_CAPABILITY,
        INPUT_DEVICE_PUMP_CAPABILITY,
        INPUT_TIMED_VIEWPORT_CAPABILITY,
    ]
    .into_iter()
    .map(|name| era_runtime_protocol::EnvironmentCapability {
        name: name.into(),
        versions: VersionRange::exact(era_runtime_protocol::INPUT_ENVIRONMENT_VERSION),
    })
    .collect();
    client.services.push(ServiceCapability {
        kind: ServiceKind::InputState,
        operation: DEVICE_PUMP_OPERATION.into(),
        versions: VersionRange::exact(DEVICE_PUMP_OPERATION_VERSION),
    });
    client
}

fn send_focus(session: &mut RuntimeSession, sequence: u64, focused: bool) {
    submit(
        session,
        sequence,
        RuntimeMessage::ClientStateChanged(era_runtime_protocol::ClientStateChanged {
            focused,
            visible: true,
            audio_available: false,
            reduce_motion: false,
            high_contrast: false,
            screen_reader: false,
        }),
    );
}
fn send_key(session: &mut RuntimeSession, sequence: u64, event_sequence: u64, pressed: bool) {
    submit(
        session,
        sequence,
        RuntimeMessage::DeviceStateChanged(era_runtime_protocol::DeviceStateChanged {
            device: era_runtime_protocol::InputDeviceKind::Keyboard,
            code: 65,
            pressed,
            event_sequence,
            toggle: false,
            repeat: false,
            x: 0,
            y: 0,
            monotonic_time_ns: event_sequence,
        }),
    );
}
