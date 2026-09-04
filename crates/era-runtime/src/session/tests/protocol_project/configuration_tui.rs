#[test]
#[allow(clippy::too_many_lines)]
fn tui_configuration_profile_applies_defaults_and_commits_atomically() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "tui-configuration-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["zh-CN".into()],
            configuration_profile: Some(ConfigurationClientProfile::Tui),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ServerHello(hello)
            if hello.configuration_profile == Some(ConfigurationClientProfile::Tui)
    )));

    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let initial = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => report.configuration,
            _ => None,
        })
        .expect("TUI project load publishes configuration");
    assert_eq!(
        initial
            .entries
            .iter()
            .filter(|entry| entry.applicability & CONFIG_TUI != 0)
            .count(),
        41
    );
    for (code, expected) in [
        ("MaxLog", "1000"),
        ("PrintCPerLine", "5"),
        ("PrintCLength", "24"),
    ] {
        let entry = initial
            .entries
            .iter()
            .find(|entry| entry.code == code)
            .unwrap();
        assert_eq!(entry.value, expected);
        assert_eq!(entry.effective_value, expected);
        assert_eq!(entry.default_value, expected);
    }

    submit(
        &mut session,
        2,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: initial.project_revision,
            expected_source_digest: initial.source_digest,
            changes: vec![
                ConfigurationChange {
                    code: "UseMouse".into(),
                    value: "NO".into(),
                },
                ConfigurationChange {
                    code: "AutoSave".into(),
                    value: "NO".into(),
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let prepared = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdatePrepared(value) => Some(value),
            _ => None,
        })
        .expect("configuration update is prepared");
    assert!(prepared.restart_required);
    assert_eq!(
        prepared.prepared_source_digest.as_slice(),
        blake3::hash(prepared.contents.as_bytes()).as_bytes()
    );

    submit(
        &mut session,
        3,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 2,
            files: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(rejection)
            if rejection.code == CommandErrorCode::InvalidState
    )));

    submit(
        &mut session,
        4,
        RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
            preparation_message_id: 3,
            outcome: ConfigurationUpdateOutcome::Commit,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let committed = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdateCommitted(value) => Some(value.configuration),
            _ => None,
        })
        .expect("configuration update is committed");
    assert!(committed.restart_pending);
    let use_mouse = committed
        .entries
        .iter()
        .find(|entry| entry.code == "UseMouse")
        .unwrap();
    assert_eq!(use_mouse.value, "NO");
    assert_eq!(use_mouse.effective_value, "NO");
    let auto_save = committed
        .entries
        .iter()
        .find(|entry| entry.code == "AutoSave")
        .unwrap();
    assert_eq!(auto_save.value, "NO");
    assert_eq!(auto_save.effective_value, "YES");
    assert!(
        session
            .start_compiled_cache_build()
            .unwrap_err()
            .contains("requires restarting")
    );

    submit(
        &mut session,
        5,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: committed.project_revision,
            expected_source_digest: committed.source_digest,
            changes: vec![ConfigurationChange {
                code: "AutoSave".into(),
                value: "YES".into(),
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(value) if value.restart_required
    )));
    submit(
        &mut session,
        6,
        RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
            preparation_message_id: 6,
            outcome: ConfigurationUpdateOutcome::Abort,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let aborted = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdateCommitted(value) => Some(value.configuration),
            _ => None,
        })
        .expect("aborted update returns the current configuration");
    assert_eq!(
        aborted
            .entries
            .iter()
            .find(|entry| entry.code == "AutoSave")
            .unwrap()
            .value,
        "NO"
    );

    submit(
        &mut session,
        7,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: aborted.project_revision,
            expected_source_digest: aborted.source_digest,
            changes: vec![ConfigurationChange {
                code: "UseMouse".into(),
                value: "YES".into(),
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(value) if !value.restart_required
    )));
    submit(
        &mut session,
        8,
        RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
            preparation_message_id: 8,
            outcome: ConfigurationUpdateOutcome::Abort,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
}
