#[test]
fn autosave_failure_prints_both_reference_messages_and_waits_before_shop() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.selected_locale = "en".into();
    session.stage_builtin_autosave_failure().unwrap();
    assert_eq!(session.controller.step, SystemStep::ShopAutosaveFailureWait);
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            }
            | era_runtime_protocol::DisplayRun::TextLayout {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            SystemTextKey::AutoSaveFailed,
            SystemTextKey::AutoSaveSkipped
        ]
    );
}

#[test]
fn stopcalltrain_discards_its_caller_and_resumes_the_train_system_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN RESULT\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 0\nONCE = 1\nCALLTRAIN 1\nENDIF\nRETURN\n@COM0\nSTOPCALLTRAIN\nRESULT:30 = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@CALLTRAINEND\nRESULT:31 = 1\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
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
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[30], None).unwrap(),
        0,
        "the STOPCALLTRAIN caller must not resume"
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[31], None).unwrap(),
        1
    );
    assert_eq!(session.controller.step, SystemStep::TrainEventComEndWait);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
}

#[test]
fn continuous_train_reports_progress_and_routes_unavailable_commands_to_usercom() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-output-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let source = "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 1\nCALLTRAIN 1\nONCE = 1\nENDIF\nRETURN\n@USERCOM\nRETURN\n@CALLTRAINEND\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
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
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            }
            | era_runtime_protocol::DisplayRun::TextLayout {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.contains(&SystemTextKey::ContinuousTrainProgress));
    assert!(keys.contains(&SystemTextKey::ContinuousTrainCommandFailed));
    assert!(!session.controller.continuous_train);
}
