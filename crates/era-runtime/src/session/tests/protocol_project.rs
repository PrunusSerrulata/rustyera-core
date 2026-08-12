use super::*;

#[test]
fn loadtext_decodes_bom_marked_unicode_assets_and_removes_carriage_returns() {
    let source = "<data>温柔</data>\r\n";
    let mut little_endian = vec![0xff, 0xfe];
    little_endian.extend(
        source
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut big_endian = vec![0xfe, 0xff];
    big_endian.extend(
        source
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>(),
    );

    assert_eq!(
        decode_load_text(&little_endian),
        Some("<data>温柔</data>\n".into())
    );
    assert_eq!(
        decode_load_text(&big_endian),
        Some("<data>温柔</data>\n".into())
    );
    assert_eq!(
        decode_load_text(b"\xef\xbb\xbf<data>ok</data>\r\n"),
        Some("<data>ok</data>\n".into())
    );
    assert_eq!(decode_load_text(&[0xff, 0xfe, 0x3c]), None);
}

#[test]
#[allow(clippy::too_many_lines)]
fn generated_configuration_can_be_confirmed_before_the_next_edit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "configuration-upgrade-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["zh-CN".into()],
            configuration_profile: Some(ConfigurationClientProfile::Tui),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let original = "[meta]\nschema_version = 1\n[text]\nfont_size = 20\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
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
                    payload: FilePayload::Utf8(original.into()),
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
        .expect("version 1 project publishes upgraded configuration");
    assert!(initial.generated_source.is_some());

    submit(
        &mut session,
        2,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: initial.project_revision,
            expected_source_digest: initial.source_digest,
            changes: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let prepared = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdatePrepared(value) => Some(value),
            _ => None,
        })
        .expect("empty transaction confirms generated contents");
    assert!(prepared.contents.contains("schema_version = 2"));
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
        .expect("generated contents are committed to the runtime manifest");
    assert!(committed.generated_source.is_none());
    assert_eq!(committed.source_digest, prepared.prepared_source_digest);

    submit(
        &mut session,
        4,
        RuntimeMessage::PrepareConfigurationUpdate(PrepareConfigurationUpdate {
            project_revision: committed.project_revision,
            expected_source_digest: committed.source_digest,
            changes: vec![ConfigurationChange {
                code: "MaxLog".into(),
                value: "777".into(),
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(value)
            if value.contents.contains("history_lines = 777")
    )));
}

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
        39
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
    assert_eq!(
        initial
            .entries
            .iter()
            .filter(|entry| entry.applicability & CONFIG_BROWSER != 0)
            .count(),
        45
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

#[test]
fn html_pop_matches_the_reference_fixture_and_writes_the_string_result() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "html-pop-oracle".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "html-pop.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTPLAIN A<&\nPRINTBUTTON \"choose\", 42\nRESULTS:30 = %HTML_POPPRINTINGSTR()%\nPRINT [0x10] hex [1e2] exponent\nRESULTS:31 = %HTML_POPPRINTINGSTR()%\nPRINT_IMG \"missing\", \"hover\", \"mask\", 20, 10 px, 7 px\nRESULTS:32 = %HTML_POPPRINTINGSTR()%\nPRINT_RECT 1 px, 2, 3 px, 4\nRESULTS:33 = %HTML_POPPRINTINGSTR()%\nPRINT_SPACE 5 px\nRESULTS:34 = %HTML_POPPRINTINGSTR()%\nWAIT\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    let values = vm
        .read_runtime_state(
            &(30..=34)
                .map(|index| erabasic_vm::VmRuntimeRead {
                    variable: results,
                    indices: vec![index],
                    character: None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(
        values,
        [
            VmValue::String("A&lt;&amp;<button value='42'>choose</button>".into()),
            VmValue::String(
                "<button value='16'>[0x10] hex </button><button value='100'>[1e2] exponent</button>".into()
            ),
            VmValue::String(
                "<img src='missing' srcb='hover' srcm='mask' height='10px' width='3' ypos='7px'>".into()
            ),
            VmValue::String("<shape type='rect' param='1px, 2, 3px, 4'>".into()),
            VmValue::String("<shape type='space' param='5px'>".into()),
        ]
    );
}

const ERAFL_HTML_CROSSING_FIXTURE: &str = include_str!(
    "../../../../../tools/runtime-tester/fixture-reference/erb/erafl-html-crossing.erb"
);
const ERAFL_HTML_CANONICAL: &str = "<button value='[MODE:TITLE_POINT]'><font color='#EE7800'>[称号点]　</font></button><button value='[MODE:TITLE_BONUS]'><font color='#C0C0C0'>[称号加成]　</font></button>";
const ERAFL_UIC_SOURCE: &str = "ERB/SYSTEM/UI/CONTAINER/UI_CONTAINER_MAIN.ERB";

fn run_erafl_html_entry(entry: &str) -> (RuntimeSession, Vec<RuntimeMessage>) {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.html = true;
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "erafl-crossed-html-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["zh-CN".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(format!(
                        "@SYSTEM_TITLE\nCALL {entry}\nWAIT\nRETURN\n"
                    )),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: ERAFL_UIC_SOURCE.into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(ERAFL_HTML_CROSSING_FIXTURE.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut observed = Vec::new();
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        observed.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{observed:#?}");
    (session, observed)
}

fn runtime_result_string(session: &RuntimeSession, index: u64) -> String {
    let vm = session.vm.as_ref().expect("runtime VM");
    let variable = runtime_variable_key(vm, "RESULTS").expect("RESULTS variable");
    let values = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable,
            indices: vec![index],
            character: None,
        }])
        .expect("RESULTS read");
    let [VmValue::String(value)] = values.as_slice() else {
        panic!("RESULTS:{index} must be a string");
    };
    value.clone()
}

fn assert_two_erafl_tab_intents(session: &RuntimeSession) {
    assert_eq!(session.command_intents.len(), 2);
    let values = session.command_intents.values().collect::<Vec<_>>();
    assert_eq!(
        values
            .iter()
            .filter(|value| matches!(value, VmValue::String(text) if text == "[MODE:TITLE_POINT]"))
            .count(),
        1
    );
    assert_eq!(
        values
            .iter()
            .filter(|value| matches!(value, VmValue::String(text) if text == "[MODE:TITLE_BONUS]"))
            .count(),
        1
    );
}

fn html_button_interaction_values(document: &erabasic_html::HtmlDocument) -> Vec<String> {
    fn visit(nodes: &[erabasic_html::HtmlNode], values: &mut Vec<String>) {
        for node in nodes {
            let erabasic_html::HtmlNode::Element {
                kind,
                children,
                interaction,
                ..
            } = node
            else {
                continue;
            };
            if *kind == erabasic_html::HtmlElementKind::Button {
                let interaction = interaction.as_ref().expect("button interaction");
                values.push(
                    interaction
                        .string_value
                        .clone()
                        .expect("eraFL tab uses a string value"),
                );
            }
            visit(children, values);
        }
    }

    let mut values = Vec::new();
    visit(&document.nodes, &mut values);
    values
}

fn warning_positions(messages: &[RuntimeMessage], command: &str) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "runtime.html.nonstandard_crossed_closing_tag"
                    && diagnostic.message.starts_with(command) =>
            {
                Some(index)
            }
            _ => None,
        })
        .collect()
}

fn first_presentation_after(messages: &[RuntimeMessage], index: usize) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .skip(index + 1)
        .find_map(|(index, message)| {
            matches!(
                message,
                RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
            )
            .then_some(index)
        })
}

fn assert_erafl_crossing_warnings(messages: &[RuntimeMessage], command: &str, fixture_line: usize) {
    let positions = warning_positions(messages, &format!("{command} "));
    assert_eq!(positions.len(), 2, "{messages:#?}");
    let source_line = ERAFL_HTML_CROSSING_FIXTURE
        .lines()
        .nth(fixture_line - 1)
        .expect("host fixture line");
    let (_, quoted) = source_line
        .split_once('"')
        .expect("HTML host call has a string argument");
    let (source_markup, _) = quoted
        .rsplit_once('"')
        .expect("HTML host call closes its string argument");
    let closes = source_markup
        .match_indices("</font>")
        .map(|(start, tag)| (start, start + tag.len()))
        .collect::<Vec<_>>();
    for (position, (start, end)) in positions.iter().zip(closes) {
        let RuntimeMessage::Diagnostic(diagnostic) = &messages[*position] else {
            unreachable!()
        };
        assert_eq!(diagnostic.level, RuntimeLogLevel::Warning);
        assert_eq!(
            diagnostic.message,
            format!(
                "{command} normalized non-standard crossed closing tag </font> at UTF-8 bytes {start}..{end} across open <button>; use properly nested markup"
            )
        );
        let origin = diagnostic.source.as_ref().expect("ERB origin");
        assert_eq!(origin.relative_path, ERAFL_UIC_SOURCE);
        assert_eq!(origin.line, Some(fixture_line as u64));
    }
    assert!(
        first_presentation_after(messages, *positions.last().unwrap()).is_some(),
        "warnings must precede the corresponding presentation update: {messages:#?}"
    );
}

#[test]
fn erafl_crossed_html_print_warns_then_preserves_two_buttons() {
    let (session, messages) = run_erafl_html_entry("ORACLE_ERAFL_HTML_CROSSING");
    assert_eq!(runtime_result_string(&session, 35), ERAFL_HTML_CANONICAL);
    assert_two_erafl_tab_intents(&session);

    assert_erafl_crossing_warnings(&messages, "HTML_PRINT", 96);
}

#[test]
fn properly_nested_erafl_tabs_do_not_warn() {
    let (session, messages) = run_erafl_html_entry("ORACLE_ERAFL_HTML_NESTED");
    assert_eq!(runtime_result_string(&session, 36), ERAFL_HTML_CANONICAL);
    assert_two_erafl_tab_intents(&session);
    assert!(
        warning_positions(&messages, "HTML_PRINT ").is_empty(),
        "{messages:#?}"
    );
}

#[test]
fn erafl_crossed_html_island_warns_before_updating_the_island() {
    let (session, messages) = run_erafl_html_entry("ORACLE_ERAFL_HTML_CROSSING_ISLAND");
    assert_two_erafl_tab_intents(&session);
    let snapshot = session.presentation.snapshot();
    assert_eq!(snapshot.html_island.len(), 1);
    assert_eq!(
        erabasic_html::serialize_document(&snapshot.html_island[0]),
        ERAFL_HTML_CANONICAL
    );
    assert_eq!(
        html_button_interaction_values(&snapshot.html_island[0]),
        ["[MODE:TITLE_POINT]", "[MODE:TITLE_BONUS]"]
    );
    assert_erafl_crossing_warnings(&messages, "HTML_PRINT_ISLAND", 106);
}

#[test]
fn safe_at_commands_use_runtime_lifecycle_effects_and_keep_debug_separate() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session.handle_system_input_command(1, "@CONFIG").unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::EffectBatch(batch)
            if matches!(batch.effects[0].kind, EffectKind::OpenConfiguration)
    )));
    session.handle_system_input_command(2, "@DEBUG").unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.debug_command_requires_debug_channel"
    )));
    session.handle_system_input_command(3, "@REBOOT").unwrap();
    assert_eq!(
        session.exit_requested.map(|exit| exit.reason),
        Some(ExitReason::Restart)
    );
    assert_eq!(session.phase, RuntimePhase::Stopping);
}

#[test]
fn configuration_update_is_validated_and_serialized_by_the_runtime() {
    let build = build_project(
        &ProjectManifest {
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

#[test]
fn ready_project_reload_stages_and_commits_a_normalized_delta() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "reload-test".into(),
            features: vec![RuntimeFeature::ProjectReload],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
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
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "./main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL reloaded\nRETURN\n".into()),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .manifest
            .project_revision,
        2
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(era_runtime_protocol::ProjectLoadReport {
            project_revision: 2,
            success: true,
            ..
        })
    )));
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.input_undo_invalidated"
    )));
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "hot_reload");
    assert_eq!(replay_header["origin"]["before_revision"], "1");
    assert_eq!(replay_header["origin"]["after_revision"], "2");
    assert_eq!(replay_header["step_count"], 0);

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let unchanged_replay = input_replay_records(&session).remove(0);
    assert_eq!(unchanged_replay["origin"], replay_header["origin"]);
}

#[test]
fn state_import_rejects_out_of_order_chunks_and_bad_digests() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TraditionalSave],
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
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: 3,
            digest: ProtocolBytes::new([0; 32]),
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();

    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 1,
            data: ProtocolBytes::new([b'a']),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(*b"abc"),
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn host_staged_compiled_cache_reuses_the_owned_payload() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let payload = vec![7; 4096];

    let transfer_id = session
        .stage_compiled_project_cache(payload.clone())
        .expect("host cache staging should accept an in-limit payload");
    let staged = session
        .consume_state_import(1, transfer_id, StateExportKind::CompiledProjectCache)
        .unwrap()
        .expect("staged cache should be committed immediately");

    assert_eq!(staged, payload);
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn training_reset_updates_shared_and_all_character_state_atomically() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let source = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "SOURCE")
        .expect("SOURCE")
        .key;
    let tflag = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
        .expect("TFLAG")
        .key;
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: source,
                    indices: vec![0],
                    character: Some(0),
                    value: VmValue::Integer(9),
                },
                VmRuntimeWrite {
                    variable: tflag,
                    indices: vec![0],
                    character: None,
                    value: VmValue::Integer(7),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .expect("prepare dirty state");
    vm.commit_runtime_state(dirty).expect("commit dirty state");
    reset_training_state(&mut vm).expect("reset training state");
    assert_eq!(
        vm.read_runtime_state(&[
            erabasic_vm::VmRuntimeRead {
                variable: source,
                indices: vec![0],
                character: Some(0),
            },
            erabasic_vm::VmRuntimeRead {
                variable: tflag,
                indices: vec![0],
                character: None,
            },
        ]),
        Ok(vec![VmValue::Integer(0), VmValue::Integer(0)])
    );
}

#[test]
fn show_user_reset_clears_shared_and_character_deltas() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let mut vm = RuntimeVm::new(build.artifact.expect("valid project"), VmConfig::default());
    for (name, character) in [
        ("UP", None),
        ("DOWN", None),
        ("LOSEBASE", None),
        ("DOWNBASE", Some(0)),
        ("CUP", Some(0)),
        ("CDOWN", Some(0)),
    ] {
        write_runtime_integer(&mut vm, name, &[0], character, 9).unwrap();
    }

    reset_after_show_user(&mut vm).unwrap();

    for (name, character) in [
        ("UP", None),
        ("DOWN", None),
        ("LOSEBASE", None),
        ("DOWNBASE", Some(0)),
        ("CUP", Some(0)),
        ("CDOWN", Some(0)),
    ] {
        assert_eq!(
            read_runtime_integer(&vm, name, &[0], character).unwrap(),
            0,
            "{name} was not reset"
        );
    }
}

#[test]
fn shop_purchase_validates_stock_and_commits_money_item_and_bought_together() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "ITEM.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("5,potion,120\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let sales = runtime_variable_key(&vm, "ITEMSALES").unwrap();
    let money = runtime_variable_key(&vm, "MONEY").unwrap();
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: sales,
                    indices: vec![5],
                    character: None,
                    value: VmValue::Integer(1),
                },
                VmRuntimeWrite {
                    variable: money,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(200),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .unwrap();
    vm.commit_runtime_state(dirty).unwrap();
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::Purchased
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(&vm, "BOUGHT", &[], None).unwrap(), 5);
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::NotEnoughMoney
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
}

#[test]
#[allow(clippy::too_many_lines)]
fn train_controller_consumes_runtime_button_intent_and_loops_after_eventcomend() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    // RESETDATA removes every character in the reference runtime, so a standalone
    // SYSTEM_TITLE fixture must explicitly create the character used by training.
    let source = "@SYSTEM_TITLE\nRESETDATA\nADDVOIDCHARA\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nPRINT 抑鬱\nRETURN\n@COM_ABLE0\nRETURN 1\n@SHOW_USERCOM\nPRINT ▼[－][Look]----------\nRETURN\n@EVENTCOM\nRETURN\n@COM0\nFLAG:0 += 1\nRETURN 1\n@SOURCE_CHECK\nRETURN\n@EVENTCOMEND\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
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
            mode: StartMode::NewGame { seed: Some(3) },
        }),
    );
    for _ in 0..12 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let pending = session
        .operations
        .active_input()
        .expect("training command wait");
    let snapshot = session.presentation.snapshot();
    let flattened_lines = snapshot
        .history
        .logical_lines
        .iter()
        .map(|line| {
            fn text(runs: &[DisplayRun], output: &mut String) {
                for run in runs {
                    match run {
                        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
                            output.push_str(text);
                        }
                        DisplayRun::Button { runs, .. }
                        | DisplayRun::ColumnCell { content: runs, .. } => text(runs, output),
                        _ => {}
                    }
                }
            }
            let mut output = String::new();
            text(&line.runs, &mut output);
            output
        })
        .collect::<Vec<_>>();
    let status_line = flattened_lines
        .iter()
        .position(|line| line.contains("抑鬱"))
        .expect("SHOW_STATUS output");
    let look_line = flattened_lines
        .iter()
        .position(|line| line.contains("[Look]"))
        .expect("SHOW_USERCOM output");
    assert!(status_line < look_line, "{flattened_lines:#?}");
    let token = *pending.choices.keys().next().expect("PRINTBUTTON token");
    let wait_id = pending.wait.wait_id;
    let submission_token = pending.wait.submission_token;
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            intent: InputIntent::Activate(token),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.controller.step == SystemStep::TrainEventComEndWait {
            break;
        }
    }
    let (event_end_wait_id, event_end_token) = session
        .operations
        .active_input()
        .map(|pending| (pending.wait.wait_id, pending.wait.submission_token))
        .expect("EVENTCOMEND wait");
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id: event_end_wait_id,
            token: event_end_token,
            intent: InputIntent::Continue,
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some()
            && session.controller.step == SystemStep::TrainShowUser
        {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:#?}");
    let vm = session.vm.as_ref().expect("running VM");
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 1);
    assert_eq!(
        read_runtime_integer(vm, "SOURCE", &[0], Some(0)).unwrap(),
        0
    );
    assert_eq!(
        session.controller.step,
        SystemStep::TrainShowUser,
        "phase={:?}",
        session.phase
    );
}
