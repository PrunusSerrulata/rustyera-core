#[test]
fn configuration_update_is_validated_and_serialized_by_the_runtime() {
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 7,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("フォントサイズ:18\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "_fixed.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("ウィンドウ幅:900\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.project_snapshot = build.snapshot;
    let configuration = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .configuration_snapshot();
    assert!(
        configuration.source_digest.as_slice().is_empty(),
        "legacy migration must retain the missing-file write precondition"
    );
    assert!(
        configuration
            .entries
            .iter()
            .any(|entry| entry.code == "WindowX" && entry.fixed)
    );
    assert!(
        configuration
            .entries
            .iter()
            .all(|entry| entry.code != "DebugWindowWidth" && entry.code != "DrawLineString")
    );

    session
        .handle_message(
            1,
            RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
                project_revision: 7,
                expected_source_digest: configuration.source_digest,
                changes: vec![era_runtime_protocol::ConfigurationChange {
                    code: "FontSize".into(),
                    value: "22".into(),
                }],
            }),
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(prepared)
            if prepared.contents.contains("font_size = 22")
                && prepared.contents.contains("width = 900")
                && prepared.restart_required
    )));

    session
        .handle_message(
            2,
            RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
                project_revision: 7,
                expected_source_digest: session
                    .project_snapshot
                    .as_ref()
                    .unwrap()
                    .configuration_snapshot()
                    .source_digest,
                changes: vec![era_runtime_protocol::ConfigurationChange {
                    code: "WindowX".into(),
                    value: "1000".into(),
                }],
            }),
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(rejected)
            if rejected.code == CommandErrorCode::InvalidValue
    )));
}

#[test]
fn debug_channel_has_independent_sequence_and_cannot_widen_creator_policy() {
    let mut session = RuntimeSession::new(RuntimeOptions {
        debug_scope_mask: (1 << 2) | (1 << 5),
        ..RuntimeOptions::default()
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "debug-test".into(),
            features: Vec::new(),
            capabilities: capabilities(),
            requested_limits: RuntimeOptions::default().limits,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);

    submit_debug(
        &mut session,
        0,
        &DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: vec![
                DebugScope::ExecutionControl,
                DebugScope::VariablesWrite,
                DebugScope::GameFieldsRead,
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let bytes = session.poll_envelope().expect("debug grant");
    let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
    let DebugMessage::Grant(grant) = DebugMessage::from_envelope(&envelope).unwrap() else {
        panic!("expected debug grant");
    };
    assert_eq!(
        grant.scopes,
        vec![DebugScope::GameFieldsRead, DebugScope::ExecutionControl]
    );
    assert_eq!(grant.token.session_epoch, session.epoch.0);
}

#[test]
fn debugger_pause_freezes_frontend_time_until_resume() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::DebugPaused;
    session.logical_time_ns = 500;
    session.frontend_time_origin = Some((10, 500));
    session
        .handle_message(
            1,
            RuntimeMessage::AdvanceTime(AdvanceTime {
                monotonic_time_ns: 1_000,
            }),
        )
        .unwrap();
    assert_eq!(session.logical_time_ns, 500);
    session.resume_debug_time();
    assert_eq!(session.frontend_time_origin, Some((1_000, 500)));
}

#[test]
fn revoking_the_active_debugger_resumes_a_debug_paused_runtime() {
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nWAIT\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session.vm = Some(RuntimeVm::new(
        build.artifact.expect("valid project"),
        VmConfig::default(),
    ));
    let grant = GrantToken {
        grant_id: SessionId { high: 7, low: 9 },
        session_epoch: session.epoch.0,
        program_generation: 0,
        issued_runtime_revision: session.revision,
    };
    session.active_debug_grant = Some(ActiveDebugGrant {
        token: grant,
        scopes: BTreeSet::from([DebugScope::ExecutionControl]),
    });

    session
        .handle_debug_message(
            1,
            DebugMessage::Request(AuthorizedDebugRequest {
                grant,
                command: DebugCommand::Pause,
            }),
        )
        .unwrap();
    assert_eq!(session.phase, RuntimePhase::DebugPaused);
    assert!(session.vm.as_ref().unwrap().stop_token().is_some());

    session
        .handle_debug_message(
            2,
            DebugMessage::Revoke(DebugRevoke {
                grant_id: grant.grant_id,
                reason: "frontend disabled debugging".into(),
            }),
        )
        .unwrap();

    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert!(session.active_debug_grant.is_none());
    assert!(session.vm.as_ref().unwrap().stop_token().is_none());
}
