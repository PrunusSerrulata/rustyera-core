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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    "../../../../../../tools/runtime-tester/fixture-reference/erb/erafl-html-crossing.erb"
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    let pending = session
        .operations
        .active_input()
        .expect("eraFL fixture WAIT must be active");
    assert_eq!(pending.choices.len(), 2);
    let values = pending.choices.values().collect::<Vec<_>>();
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
        assert_eq!(diagnostic.notification, DiagnosticNotification::LogOnly);
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
