#[test]
#[allow(clippy::too_many_lines)]
fn browser_configuration_profile_hot_applies_and_tracks_restart_values() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "browser-configuration-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["zh-CN".into()],
            configuration_profile: Some(ConfigurationClientProfile::Browser),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ServerHello(hello)
            if hello.configuration_profile == Some(ConfigurationClientProfile::Browser)
    )));

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
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "reraconfig.toml".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8(
                        "[meta]\nschema_version = 3\nlocked_settings = [\"input.mouse_enabled\"]\n\n[input]\nmouse_enabled = false\n\n[text]\nfont_size = 21\nline_height = 21\n"
                            .into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let initial = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => report.configuration,
            _ => None,
        })
        .unwrap();
    assert_eq!(
        initial
            .entries
            .iter()
            .filter(|entry| entry.applicability & CONFIG_BROWSER != 0)
            .count(),
        47
    );
    assert_eq!(
        initial
            .entries
            .iter()
            .find(|entry| entry.code == "PrintCPerLine")
            .unwrap()
            .value,
        "5"
    );
    assert!(
        initial
            .entries
            .iter()
            .find(|entry| entry.code == "FontSize")
            .is_some_and(|entry| entry.preference_eligible)
    );
    let menu = initial
        .entries
        .iter()
        .find(|entry| entry.code == "UseMenu")
        .unwrap();
    assert_eq!(menu.kind, ConfigurationValueKind::Enum);
    assert_eq!(menu.value, "AUTO");
    assert_eq!(menu.default_value, "AUTO");
    assert_eq!(menu.allowed, ["SHOW", "AUTO", "HIDE"].map(str::to_owned));
    assert_eq!(
        initial
            .entries
            .iter()
            .filter(|entry| entry.preference_eligible)
            .map(|entry| entry.code.as_str())
            .collect::<BTreeSet<_>>(),
        [
            "AudioVolume",
            "BackColor",
            "ButtonWrap",
            "FocusColor",
            "FontName",
            "FontSize",
            "ForeColor",
            "LineHeight",
            "ReplaceFullWidthSpaces",
            "ScrollHeight",
            "UseMenu",
            "UseMouse",
        ]
        .into_iter()
        .collect(),
        "browser preferences must expose only the planned client-only surface"
    );
    let identity_before_preferences = session.project_snapshot.as_ref().unwrap().project_identity;

    session
        .handle_message(
            100,
            RuntimeMessage::ApplyClientPreferences(ClientPreferenceLayers {
                project_revision: initial.project_revision,
                global: vec![
                    ConfigurationChange {
                        code: "FontSize".into(),
                        value: "20".into(),
                    },
                    ConfigurationChange {
                        code: "UseMouse".into(),
                        value: "NO".into(),
                    },
                ],
                project: vec![
                    ConfigurationChange {
                        code: "UseMouse".into(),
                        value: "YES".into(),
                    },
                    ConfigurationChange {
                        code: "LineHeight".into(),
                        value: "23".into(),
                    },
                ],
            }),
        )
        .unwrap();
    let preferred = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ClientPreferencesApplied(value) => Some(value.configuration),
            _ => None,
        })
        .expect("client preference layers are applied");
    for (code, project_value, client_value) in [
        ("FontSize", "21", "21"),
        ("UseMouse", "NO", "YES"),
        ("LineHeight", "21", "23"),
    ] {
        let entry = preferred
            .entries
            .iter()
            .find(|entry| entry.code == code)
            .unwrap();
        assert_eq!(entry.effective_value, project_value);
        assert_eq!(entry.client_effective_value, client_value, "{code}");
    }
    let snapshot_after_preferences = session.project_snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot_after_preferences.project_identity,
        identity_before_preferences
    );
    assert_eq!(
        snapshot_after_preferences
            .configuration
            .get_code("FontSize"),
        snapshot_after_preferences
            .editable_configuration
            .get_code("FontSize"),
        "client preferences must not mutate semantic project configuration"
    );

    submit(
        &mut session,
        2,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: initial.project_revision,
            expected_source_digest: initial.source_digest,
            changes: vec![
                ConfigurationChange {
                    code: "FontSize".into(),
                    value: "22".into(),
                },
                ConfigurationChange {
                    code: "LineHeight".into(),
                    value: "24".into(),
                },
                ConfigurationChange {
                    code: "AutoSave".into(),
                    value: "NO".into(),
                },
                ConfigurationChange {
                    code: "CharacterWidthMode".into(),
                    value: "AMBIGUOUS_NARROW".into(),
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
        .unwrap();
    assert!(prepared.restart_required);

    submit(
        &mut session,
        3,
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
        .unwrap();
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "configuration_update");
    assert_ne!(
        replay_header["origin"]["before_identity"],
        replay_header["origin"]["after_identity"]
    );
    assert_eq!(replay_header["step_count"], 0);
    assert!(committed.restart_pending);
    for (code, value) in [("FontSize", "22"), ("LineHeight", "24")] {
        let entry = committed
            .entries
            .iter()
            .find(|entry| entry.code == code)
            .unwrap();
        assert_eq!(entry.value, value);
        assert_eq!(entry.effective_value, value);
    }
    assert_eq!(
        committed
            .entries
            .iter()
            .find(|entry| entry.code == "FontSize")
            .unwrap()
            .client_effective_value,
        "22",
        "an explicit project setting must replace the global preference"
    );
    assert_eq!(
        committed
            .entries
            .iter()
            .find(|entry| entry.code == "LineHeight")
            .unwrap()
            .client_effective_value,
        "23",
        "a project preference must replace an explicit project setting"
    );
    let auto_save = committed
        .entries
        .iter()
        .find(|entry| entry.code == "AutoSave")
        .unwrap();
    assert_eq!(auto_save.value, "NO");
    assert_eq!(auto_save.effective_value, "YES");
    let width_mode = committed
        .entries
        .iter()
        .find(|entry| entry.code == "CharacterWidthMode")
        .unwrap();
    assert_eq!(width_mode.value, "AMBIGUOUS_NARROW");
    assert_eq!(width_mode.effective_value, "AMBIGUOUS_NARROW");
    let snapshot = session.project_snapshot.as_ref().unwrap();
    assert_eq!(snapshot.font_size, 22);
    assert_eq!(snapshot.line_height, 24);
    assert_eq!(
        configured_character_width_mode(Some(snapshot)),
        CharacterWidthMode::AmbiguousNarrow
    );
    session
        .presentation
        .append_print_text("☀❤……".into(), false, true);
    let projected = session.presentation.snapshot();
    let columns = projected.history.logical_lines[0]
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::TextLayout { columns, .. } => Some(*columns),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(columns, [1, 1, 1, 1]);
}

#[test]
fn browser_and_tauri_configuration_abort_preserves_effective_presentation() {
    for profile in [
        ConfigurationClientProfile::Browser,
        ConfigurationClientProfile::Tauri,
    ] {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "web-configuration-abort-test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["zh-CN".into()],
                configuration_profile: Some(profile),
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
            .unwrap();
        let original_presentation = session.presentation.snapshot().settings;

        submit(
            &mut session,
            2,
            RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
                project_revision: initial.project_revision,
                expected_source_digest: initial.source_digest,
                changes: vec![ConfigurationChange {
                    code: "LineHeight".into(),
                    value: "30".into(),
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
            3,
            RuntimeMessage::FinalizeConfigurationUpdate(FinalizeConfigurationUpdate {
                preparation_message_id: 3,
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
            .unwrap();
        let line_height = aborted
            .entries
            .iter()
            .find(|entry| entry.code == "LineHeight")
            .unwrap();
        assert_eq!(line_height.value, "19");
        assert_eq!(line_height.effective_value, "19");
        assert_eq!(session.project_snapshot.as_ref().unwrap().line_height, 19);
        assert_eq!(
            session.presentation.snapshot().settings,
            original_presentation
        );
    }
}
