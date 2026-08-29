use super::*;

#[test]
fn goto_into_case_body_emits_a_nonfatal_warning_and_continues() {
    let mut session = negotiated_session();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nGOTO CHOICE\nSELECTCASE 0\nCASE 0\n$CHOICE\nPRINTL reached\nENDSELECT\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let messages = drain(&mut session);

    let diagnostic = messages.iter().find_map(|message| match message {
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "vm.control_flow.goto_into_structured_block" =>
        {
            Some(diagnostic)
        }
        _ => None,
    });
    let diagnostic = diagnostic.expect("cross-block GOTO warning");
    assert_eq!(diagnostic.level, RuntimeLogLevel::Warning);
    assert_eq!(diagnostic.notification, DiagnosticNotification::LogOnly);
    assert!(diagnostic.message.contains("avoid jumping"));
    assert_eq!(
        diagnostic
            .source
            .as_ref()
            .map(|source| source.relative_path.as_str()),
        Some("main.erb")
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    assert!(projected_presentation_text(&session.presentation.snapshot()).contains("reached"));
}

fn run_immediate_query_project(
    source: &str,
) -> (RuntimeSession, RuntimeDriveReport, Vec<RuntimeMessage>) {
    run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::default(),
    )
}

fn run_immediate_query_project_with_profile(
    source: &str,
    compatibility: erabasic_compat::CompatibilityIdentity,
) -> (RuntimeSession, RuntimeDriveReport, Vec<RuntimeMessage>) {
    run_immediate_query_project_with_budget(
        source,
        compatibility,
        RuntimeDriveBudget {
            maximum_vm_instructions: 1_000_000,
            maximum_runtime_transitions: 1_024,
        },
    )
}

fn run_immediate_query_project_with_budget(
    source: &str,
    compatibility: erabasic_compat::CompatibilityIdentity,
    budget: RuntimeDriveBudget,
) -> (RuntimeSession, RuntimeDriveReport, Vec<RuntimeMessage>) {
    let mut session = negotiated_session();
    let config = profile_configuration_file(compatibility.profile);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility,
            project_revision: 1,
            files: vec![
                config,
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )),
        "{loaded:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let report = session.drive(budget).unwrap();
    let messages = drain(&mut session);
    (session, report, messages)
}

#[test]
fn immediate_queries_observe_latest_runtime_state_without_host_boundaries() {
    let source = "@SYSTEM_TITLE\n\
        PRINTL oldest\n\
        ALIGNMENT CENTER\n\
        SETFONT \"query-font\"\n\
        REDRAW 0\n\
        SETBGCOLOR 4, 5, 6\n\
        SETCOLOR 1, 2, 3\n\
        FONTBOLD\n\
        PRINTFORM pending\n\
        FLAG:0 = CURRENTALIGN() == \"CENTER\"\n\
        FLAG:1 = GETFONT() == \"query-font\"\n\
        FLAG:2 = CURRENTREDRAW() == 0\n\
        FLAG:3 = GETBGCOLOR() == COLOR_FROMRGB(4, 5, 6)\n\
        FLAG:4 = GETCOLOR() == COLOR_FROMRGB(1, 2, 3)\n\
        FLAG:5 = GETSTYLE() == 1\n\
        FLAG:6 = LINEISEMPTY() == 0\n\
        SKIPDISP 1\n\
        FLAG:7 = ISSKIP()\n\
        SKIPLOG 1\n\
        FLAG:8 = MESSKIP()\n\
        FLAG:9 = MOUSESKIP()\n\
        FLAG:10 = HTML_TOPLAINTEXT(\"a&nbsp;b\") == \"a b\"\n\
        FLAG:11 = HTML_ESCAPE(\"<\") == \"&lt;\"\n\
        RESULTS '= HTML_GETPRINTEDSTR(0)\n\
        FLAG:12 = RESULTS == \"<p align='center'><nobr><b>p</b><b>e</b><b>n</b><b>d</b><b>i</b><b>n</b><b>g</b></nobr></p>\"\n\
        FLAG:13 = GETDISPLAYLINE(0) == \"oldest\"\n\
        FLAG:14 = GETDISPLAYLINE(1) == \"pending\"\n\
        FLAG:15 = HTML_GETPRINTEDSTR(1) == \"<p align='left'><nobr>oldest</nobr></p>\"\n\
        FLAG:16 = GETDISPLAYLINE(-1) == \"\"\n\
        FLAG:17 = GETDISPLAYLINE(4294967296) == \"\"\n\
        FLAG:18 = HTML_GETPRINTEDSTR(4294967296) == \"\"\n\
        FOR LOCAL, 0, 32\n\
            RESULT:40 = GETDEFBGCOLOR()\n\
            RESULT:41 = GETDEFCOLOR()\n\
            RESULT:42 = GETFOCUSCOLOR()\n\
            RESULT:43 = GETBGCOLOR()\n\
            RESULT:44 = GETCOLOR()\n\
            RESULT:45 = GETSTYLE()\n\
            RESULT:46 = CURRENTREDRAW()\n\
            RESULT:47 = LINEISEMPTY()\n\
            RESULT:48 = CURRENTALIGN() == \"CENTER\"\n\
            RESULT:49 = GETFONT() == \"query-font\"\n\
            RESULT:50 = HTML_TOPLAINTEXT(\"a&nbsp;b\") == \"a b\"\n\
            RESULT:51 = HTML_ESCAPE(\"<\") == \"&lt;\"\n\
            RESULT:52 = HTML_GETPRINTEDSTR(0) == \"<p align='center'><nobr><b>p</b><b>e</b><b>n</b><b>d</b><b>i</b><b>n</b><b>g</b></nobr></p>\"\n\
            RESULT:53 = GETDISPLAYLINE(1) == \"pending\"\n\
        NEXT\n\
        SKIPLOG 0\n\
        SKIPDISP 0\n\
        FORCEWAIT\n\
        RETURN\n";
    let (session, report, messages) = run_immediate_query_project(source);

    assert!(
        report.runtime_transitions < 32,
        "repeated read-only queries must stay within VM quanta: {report:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    let vm = session.vm.as_ref().expect("runtime VM");
    let printed_html = read_runtime_string(vm, "RESULTS").unwrap();
    for index in 0..=18 {
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[index], None).unwrap(),
            1,
            "FLAG:{index}; HTML_GETPRINTEDSTR(0)={printed_html:?}"
        );
    }
    assert_eq!(read_runtime_integer(vm, "RESULT", &[52], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[53], None).unwrap(), 1);
    assert!(messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.kind == ServiceKind::PresentationQuery
    )));
}

#[test]
fn negative_printed_html_index_falls_back_to_a_sourced_vm_fault() {
    let (session, _report, messages) =
        run_immediate_query_project("@SYSTEM_TITLE\nRESULTS '= HTML_GETPRINTEDSTR(-1)\nRETURN\n");

    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault {
                code: FaultCode::VmFault,
                message,
                origin: Some(origin),
                ..
        }) if message == "HTML_GETPRINTEDSTR line number must be non-negative"
            && origin.command.eq_ignore_ascii_case("HTML_GETPRINTEDSTR")
                && origin.source.as_ref().is_some_and(|source| source.relative_path == "main.erb")
        )),
        "{messages:#?}"
    );
}

#[test]
fn default_html_tag_split_stays_in_the_immediate_runtime_lane() {
    let source = "@SYSTEM_TITLE\n\
        #DIMS EXPLICIT, 8\n\
        #DIM COUNT\n\
        PRINTL seed\n\
        HTML_TAGSPLIT \"a<b>x</b>\"\n\
        FLAG:0 = RESULT == 4\n\
        FLAG:1 = RESULTS:0 == \"a\"\n\
        FLAG:2 = RESULTS:1 == \"<b>\"\n\
        FLAG:3 = RESULTS:2 == \"x\"\n\
        FLAG:4 = RESULTS:3 == \"</b>\"\n\
        HTML_TAGSPLIT \"z\"\n\
        FLAG:5 = RESULT == 1 && RESULTS:3 == \"</b>\"\n\
        HTML_TAGSPLIT \"\"\n\
        FLAG:6 = RESULT == 0 && RESULTS:3 == \"</b>\"\n\
        HTML_TAGSPLIT \"a<b\"\n\
        FLAG:7 = RESULT == -1 && RESULTS:3 == \"</b>\"\n\
        HTML_TAGSPLIT \"a<b>x</b>\", EXPLICIT, COUNT\n\
        FLAG:8 = COUNT == 4 && EXPLICIT:0 == \"a\" && EXPLICIT:3 == \"</b>\"\n\
        EXPLICIT:7 = \"keep\"\n\
        HTML_TAGSPLIT \"a<b\", EXPLICIT, COUNT\n\
        FLAG:9 = COUNT == -1\n\
        FOR LOCAL, 0, 1302\n\
            HTML_TAGSPLIT HTML_GETPRINTEDSTR(0)\n\
        NEXT\n\
        FLAG:10 = RESULT > 0\n\
        FORCEWAIT\n\
        RETURN\n";
    let (session, report, messages) = run_immediate_query_project(source);

    assert!(
        report.runtime_transitions < 32,
        "default HTML_TAGSPLIT must not create one transition per call: {report:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    let vm = session.vm.as_ref().expect("runtime VM");
    for index in 0..=10 {
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[index], None).unwrap(),
            1,
            "FLAG:{index}"
        );
    }
}

#[test]
fn clean_html_prints_stay_in_one_vm_quantum_and_synchronize_line_count() {
    let source = "@SYSTEM_TITLE\n\
        PRINTL baseline\n\
        RESULT:0 = LINECOUNT\n\
        FOR LOCAL, 0, 31\n\
            FOR LOCAL:1, 0, 42\n\
                HTML_PRINT \"<p align='left'><nobr>line</nobr></p>\"\n\
            NEXT\n\
            IF LOCAL != 30\n\
                CLEARLINE 42\n\
            ENDIF\n\
        NEXT\n\
        RESULT:1 = LINECOUNT\n\
        CLEARLINE 42\n\
        RESULT:2 = LINECOUNT\n\
        HTML_PRINT \"<nobr>inline</nobr>\", 1\n\
        RESULT:3 = LINECOUNT\n\
        FORCEWAIT\n\
        RETURN\n";
    let (session, report, messages) = run_immediate_query_project(source);

    assert!(
        report.runtime_transitions < 64,
        "clean HTML_PRINT calls must not create one transition per call: {report:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    let vm = session.vm.as_ref().expect("runtime VM");
    let baseline = read_runtime_integer(vm, "RESULT", &[0], None).unwrap();
    assert_eq!(
        read_runtime_integer(vm, "RESULT", &[1], None).unwrap(),
        baseline + 42
    );
    assert_eq!(
        read_runtime_integer(vm, "RESULT", &[2], None).unwrap(),
        baseline
    );
    assert_eq!(
        read_runtime_integer(vm, "RESULT", &[3], None).unwrap(),
        baseline,
        "inline HTML_PRINT must not commit a logical line"
    );
}

#[test]
fn immediate_html_print_binds_integer_and_string_buttons_in_order() {
    let source = "@SYSTEM_TITLE\n\
        HTML_PRINT \"<button value='7'>integer</button><button value='word'>string</button>\"\n\
        FORCEWAIT\n\
        RETURN\n";
    let (session, report, messages) = run_immediate_query_project(source);

    assert!(report.runtime_transitions < 8, "{report:?}");
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    assert_eq!(
        session
            .command_intents
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        [VmValue::Integer(7), VmValue::String("word".into())]
    );
    let tokens = session.command_intents.keys().copied().collect::<Vec<_>>();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].epoch, session.epoch.0);
    assert_eq!(tokens[1].id, tokens[0].id + 1);
    let output = projected_presentation_text(&session.presentation.snapshot());
    assert!(output.contains("integer"), "{output:?}");
    assert!(output.contains("string"), "{output:?}");
}

#[test]
fn button_style_and_line_edits_stay_in_one_vm_quantum() {
    let source = "@SYSTEM_TITLE\n\
        PRINTL baseline\n\
        RESULT:0 = LINECOUNT\n\
        FOR LOCAL, 0, 128\n\
            SETCOLOR LOCAL, 0, 0\n\
            PRINTBUTTONC \"choice\", LOCAL\n\
            RESETCOLOR\n\
        NEXT\n\
        PRINTL\n\
        FOR LOCAL, 0, 64\n\
            DRAWLINE\n\
            CLEARLINE 1\n\
        NEXT\n\
        RESULT:1 = LINECOUNT\n\
        FORCEWAIT\n\
        RETURN\n";
    let (session, report, messages) = run_immediate_query_project(source);

    assert!(
        report.runtime_transitions < 8,
        "pure presentation commands must not create one transition per call: {report:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    assert_eq!(session.command_intents.len(), 128);
    let vm = session.vm.as_ref().expect("runtime VM");
    assert_eq!(
        read_runtime_integer(vm, "RESULT", &[1], None).unwrap(),
        read_runtime_integer(vm, "RESULT", &[0], None).unwrap() + 1
    );
}

#[test]
fn malformed_immediate_html_print_falls_back_to_a_sourced_vm_fault() {
    let (session, _report, messages) =
        run_immediate_query_project("@SYSTEM_TITLE\nHTML_PRINT \"<unknown>\"\nRETURN\n");

    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault {
                context: _,
                code: FaultCode::VmFault,
                message,
                origin: Some(origin),
                ..
        }) if message.contains("HTML_PRINT UnknownTag")
            && origin.command.eq_ignore_ascii_case("HTML_PRINT")
                && origin.source.as_ref().is_some_and(|source| source.relative_path == "main.erb")
        )),
        "{messages:#?}"
    );
}

#[test]
fn malformed_immediate_html_query_falls_back_to_a_sourced_vm_fault() {
    let (session, _report, messages) = run_immediate_query_project(
        "@SYSTEM_TITLE\nRESULT = HTML_TOPLAINTEXT(\"&#xD800;\") == \"\"\nRETURN\n",
    );

    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault {
                context: _,
                code: FaultCode::VmFault,
                message,
                origin: Some(origin),
                ..
        }) if message == "malformed HTML text"
            && origin.command.eq_ignore_ascii_case("HTML_TOPLAINTEXT")
                && origin.source.as_ref().is_some_and(|source| source.relative_path == "main.erb")
        )),
        "{messages:#?}"
    );
}

#[test]
fn moneystr_invalid_format_keeps_vm_fault_priority_without_project_context() {
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nRESULTS '= MONEYSTR(1, \"invalid[\")\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let entry = vm
        .vm()
        .artifact()
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let request = vm
        .drive(RunBudget::default(), VmDriveMode::Normal)
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .expect("real MONEYSTR host request");
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.handle_host_call(&mut vm, &request).unwrap();
    assert!(
        !drain(&mut session)
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
    for event in vm.drive(RunBudget::default(), VmDriveMode::Normal).events {
        session.handle_vm_event(&mut vm, event).unwrap();
    }
    let messages = drain(&mut session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault {
                context: _,
                code: FaultCode::VmFault,
                message,
                origin: Some(origin),
                ..
            }) if message.contains("MONEYSTR format is invalid") && origin.command.eq_ignore_ascii_case("MONEYSTR")
        )),
        "{messages:#?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn gcreatefromfile_defaults_to_content_directory_and_replays_dynamic_sprite() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.html = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "content-directory-graphics-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
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
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nRESULT = GCREATEFROMFILE(1, \"dummy.webp\")\nRESULT:1 = SPRITECREATE(\"FACE_1\", 1)\nHTML_PRINT \"<img src='FACE_1'>\"\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/dummy.webp".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let metadata_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("image metadata request");
    submit(
        &mut session,
        2,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: metadata_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 300,
                        height: 300,
                        format: "webp".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
    submit(
        &mut session,
        3,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }

    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        1
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        1
    );
    let replay = session.presentation.snapshot().resources;
    let face = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "FACE_1")
        .expect("dynamic face sprite");
    assert_eq!(face.canvas_id, Some(1));
    let canvas = replay
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == 1)
        .expect("content image canvas");
    let CanvasReplayCommand::DrawSprite { name, .. } = &canvas.commands[0] else {
        panic!("content image canvas must reference a frontend-owned resource");
    };
    assert!(replay.sprites.iter().any(|sprite| {
        sprite.name == *name && sprite.frames[0].resource_id == "resources/dummy.webp"
    }));
}

#[test]
fn retired_drawing_backend_queries_keep_the_reference_compatibility_value() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "drawing-query-test".into(),
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
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTFORML %GETCONFIGS(\"TextDrawingMode\")%|%GETCONFIGS(\"Drawing interface\")%|%GETCONFIGS(\"描画インターフェース\")%|%GETCONFIGS(\"  textdrawingmode  \")%\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert!(
        projected_presentation_text(&session.presentation.snapshot())
            .contains("TEXTRENDERER|TEXTRENDERER|TEXTRENDERER|TEXTRENDERER")
    );
}

#[test]
fn audio_commands_project_canonical_sound_directory_resources() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client_capabilities = capabilities();
    client_capabilities.audio = true;
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "audio-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
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
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPLAYBGM \"theme.mp3\"\nPLAYSOUND \"door.mp3\"\nSETSOUNDVOLUME 25\nPLAYSOUND \"knock.mp3\"\nSTOPSOUND\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/theme.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/door.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/knock.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
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

    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        let audio_effects = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::EffectBatch(batch) => Some(batch.effects.len()),
                _ => None,
            })
            .sum::<usize>();
        if audio_effects >= 5 {
            break;
        }
    }

    assert_audio_effect(
        &messages,
        1,
        AudioEffectAction::Play,
        Some("sound/theme.mp3"),
    );
    for resource in ["sound/door.mp3", "sound/knock.mp3"] {
        assert_audio_effect(&messages, 0, AudioEffectAction::Play, Some(resource));
    }
    for action in [AudioEffectAction::SetVolume, AudioEffectAction::Stop] {
        assert_audio_effect(&messages, 0, action, None);
    }
    let audio = session.presentation.snapshot().audio;
    assert_eq!(audio.len(), 1);
    assert_eq!(audio[0].channel_id, 1);
    assert_eq!(audio[0].resource_id, "sound/theme.mp3");
    assert_eq!(audio[0].volume_millionths, 1_000_000);
    assert!(audio[0].playing);
}

fn assert_audio_effect(
    messages: &[RuntimeMessage],
    channel_id: u64,
    action: AudioEffectAction,
    resource_id: Option<&str>,
) {
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::EffectBatch(batch)
                if batch.effects.iter().any(|effect| matches!(
                    &effect.kind,
                    EffectKind::Audio(audio)
                        if audio.channel_id == channel_id
                            && audio.action == action
                            && resource_id.is_none_or(|expected| {
                                audio.resource_id.as_deref() == Some(expected)
                            })
                ))
        )),
        "missing channel {channel_id} {action:?} audio effect for {resource_id:?}: {messages:#?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_message_skip_input_drains_non_value_waits_until_forcewait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "message-skip-test".into(),
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
                relative_path: "message-skip.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTL first\nWAIT\nPRINTL second\nWAITANYKEY\nPRINTL third\nTWAIT 100, 1\nPRINTL fourth\nFORCEWAIT\nPRINTL after\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    drain(&mut session);
    let (initial_wait_id, initial_token) = {
        let pending = session.operations.active_input().unwrap();
        (pending.wait.wait_id, pending.wait.submission_token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: initial_wait_id,
            token: initial_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );

    let mut messages = Vec::new();
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.stop_message_skip)
        {
            break;
        }
    }

    let pending = session.operations.active_input().expect("force wait");
    assert!(pending.wait.stop_message_skip);
    assert!(!session.message_skip);
    let output = session.presentation.log_text(false);
    assert!(output.contains("fourth"));
    assert!(!output.contains("after"));
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::CommandRejected(_)))
    );
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) if !wait.stop_message_skip
    )));
    assert_eq!(
        messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)) => Some(*wait_id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![initial_wait_id]
    );
    assert_eq!(
        messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::StateChanged(change) => Some(change.phase),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![RuntimePhase::Running, RuntimePhase::WaitingInput],
        "automatically skipped waits must not publish redundant running phases"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::ProjectionState(_)))
            .count(),
        1,
        "automatically skipped waits must not republish unchanged projection state"
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::InputUndoStateChanged(_)))
            .count(),
        1,
        "only the final visible wait should publish input-undo availability"
    );
    let position = |predicate: fn(&RuntimeMessage) -> bool| {
        messages
            .iter()
            .position(predicate)
            .expect("expected message in skip sequence")
    };
    let running = position(|message| {
        matches!(
            message,
            RuntimeMessage::StateChanged(change) if change.phase == RuntimePhase::Running
        )
    });
    let projection = position(|message| matches!(message, RuntimeMessage::ProjectionState(_)));
    let undo = position(|message| matches!(message, RuntimeMessage::InputUndoStateChanged(_)));
    let presentation = position(|message| {
        matches!(
            message,
            RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
        )
    });
    let opened =
        position(|message| matches!(message, RuntimeMessage::WaitChanged(WaitChange::Opened(_))));
    let waiting = position(|message| {
        matches!(
            message,
            RuntimeMessage::StateChanged(change) if change.phase == RuntimePhase::WaitingInput
        )
    });
    assert!(running < projection);
    assert!(projection < presentation);
    assert!(presentation < undo);
    assert!(undo < opened);
    assert!(opened < waiting);
}

#[test]
fn message_skip_stops_when_can_skip_is_explicitly_omitted() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "message-skip-value-test".into(),
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
                relative_path: "message-skip-value.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nWAIT\nINPUTS ,,\nPRINTL after\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    drain(&mut session);
    let (wait_id, token) = {
        let pending = session.operations.active_input().unwrap();
        (pending.wait.wait_id, pending.wait.submission_token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 0,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.kind == WaitKind::StringValue)
        {
            break;
        }
    }
    let pending = session.operations.active_input().expect("value wait");
    assert_eq!(pending.wait.kind, WaitKind::StringValue);
    assert!(!session.message_skip);
    assert!(!session.presentation.log_text(false).contains("after"));
}

#[test]
fn project_load_start_and_print_cross_the_message_boundary() {
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
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
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
                        "@SYSTEM_TITLE\nSKIPDISP 1\nPRINTFORM HIDDEN_BY_SKIPDISP\nSKIPDISP 0\nPRINTCPERLINE RESULT\nPRINTFORM FAST=0\nPRINTFORM 1\nPRINTFORM 2\nPRINTFORM 3\nPRINTFORM 4\nPRINTFORM 5\nPRINTFORM 6\nPRINTFORM 7\nPRINTFORM FMT=%TOSTR(12345, \"+#0;-#0\")%/%TOFULL(\"A1\")%/%TOHALF(\"Ａ１\")%/%MONEYSTR(7)%/%BARSTR(1, 2, 3)%\nPRINTFORML TITLE_CHARANUM={CHARANUM}\nPRINTFORML LAYOUT={RESULT}\nPRINTL ORACLE_READY\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CHARA0.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("番号,0\n名前,initial\n".into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(loaded.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report) if report.success
    )));
    assert_eq!(session.phase(), RuntimePhase::Ready);

    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let initial = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 2,
        })
        .expect("start");
    assert_eq!(initial.runtime_transitions, 2);
    let mut output = drain(&mut session);
    let yielded = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1,
        })
        .expect("bounded ready host call");
    assert_eq!(yielded.state, RuntimeDriveState::MoreWork);
    let report = session.drive(RuntimeDriveBudget::default()).expect("run");
    assert!(
        report.runtime_transitions <= 2,
        "committed PRINT calls must remain inside the current VM quantum: {report:?}"
    );
    assert_eq!(session.random_seed(), Some(1));
    output.extend(drain(&mut session));
    assert_fast_lane_project_output(&session, &output);
}

fn assert_fast_lane_project_output(session: &RuntimeSession, output: &[RuntimeMessage]) {
    assert!(output.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("ORACLE_READY"))
    );
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("TITLE_CHARANUM=0"))
    );
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        projected_line_text(line)
            .contains("FAST=01234567FMT=+12345/Ａ１/A1/$7/[*..]TITLE_CHARANUM=0")
    }));
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| { projected_line_text(line).contains("FMT=+12345/Ａ１/A1/$7/[*..]") })
    );
    assert!(
        !snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| { projected_line_text(line).contains("<place>") })
    );
    assert!(
        !snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| { projected_line_text(line).contains("HIDDEN_BY_SKIPDISP") })
    );
}

#[test]
fn linecount_drives_clearline_and_bounded_padding_loops() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "linecount-test".into(),
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
                        "@SYSTEM_TITLE\nCALL ORACLE_LINECOUNT\nWAIT\nRETURN\n".into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "linecount.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        include_str!(
                            "../../../../../tools/runtime-tester/fixture-reference/erb/linecount.erb"
                        )
                        .into(),
                    ),
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
    for _ in 0..20 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(session.presentation.logical_line_count(), 3);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[50], None).unwrap(), 2);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[51], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[52], None).unwrap(), 3);
    let snapshot = session.presentation.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 3);
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line) == "one")
    );
}

include!("host_runtime_continued.rs");

#[test]
fn host_assert_and_throw_keep_uncaught_fault_source_and_committed_effects() {
    for (statement, command, expected) in [
        ("ASSERT 0", "ASSERT", "ASSERT failed"),
        ("THROW explicit-host-error", "THROW", "explicit-host-error"),
        (
            "SAVEDATA -1, \"invalid\"",
            "SAVEDATA",
            "SAVEDATA argument 1 must be between 0 and 2147483647",
        ),
    ] {
        let source = format!("@SYSTEM_TITLE\nFLAG:0 = 7\n{statement}\nFLAG:0 = 9\nRETURN\n");
        let (session, _, messages) = run_immediate_query_project(&source);
        assert_eq!(session.phase(), RuntimePhase::Faulted);
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
            7
        );
        let faults: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::Fault(fault) => Some(fault),
                _ => None,
            })
            .collect();
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].code, FaultCode::VmFault);
        assert_eq!(faults[0].message, expected);
        let origin = faults[0].origin.as_ref().unwrap();
        assert!(origin.command.eq_ignore_ascii_case(command));
        assert_eq!(origin.source.as_ref().unwrap().relative_path, "main.erb");
    }
}

#[test]
fn snake_before_throw_reports_the_original_throw_without_before_error() {
    let source = "@SYSTEM_TITLE\nFLAG:0 = 7\nTHROW original-throw\nFLAG:0 = 9\nRETURN\n\
        @BEFORE_THROW\nFLAG:1 += 1\nRETURN\n\
        @BEFORE_ERROR\nFLAG:2 += 1\nRETURN\n";
    let (session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 7);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[2], None).unwrap(), 0);
    let fault = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::Fault(fault) => Some(fault),
            _ => None,
        })
        .expect("original throw fault");
    assert_eq!(fault.message, "original-throw");
    let vm_fault = fault.vm.as_ref().expect("structured VM fault");
    assert_ne!(vm_fault.primary.correlation_id, 0);
    assert!(vm_fault.secondary.is_none());
    assert!(
        fault
            .origin
            .as_ref()
            .unwrap()
            .command
            .eq_ignore_ascii_case("THROW")
    );
}

#[test]
fn snake_before_error_keeps_original_fault_and_attaches_hook_failure() {
    let source = "@SYSTEM_TITLE\nFLAG:0 = 7\nASSERT 0\nFLAG:0 = 9\nRETURN\n\
        @BEFORE_ERROR\nFLAG:1 += 1\nTHROW hook-failed\nRETURN\n\
        @BEFORE_THROW\nFLAG:2 += 1\nRETURN\n";
    let (session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 7);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[2], None).unwrap(), 0);
    let fault = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::Fault(fault) => Some(fault),
            _ => None,
        })
        .expect("original assertion fault");
    assert_eq!(fault.message, "ASSERT failed");
    let vm_fault = fault.vm.as_ref().expect("structured VM fault");
    let secondary = vm_fault.secondary.as_ref().expect("secondary hook fault");
    assert_eq!(secondary.message, "hook-failed");
    assert_eq!(
        secondary.parent_correlation_id,
        Some(vm_fault.primary.correlation_id)
    );
    assert!(
        secondary
            .origin
            .as_ref()
            .unwrap()
            .command
            .eq_ignore_ascii_case("THROW")
    );
}

#[test]
fn snake_before_error_normal_completion_still_reports_original_fault() {
    let source = "@SYSTEM_TITLE\nFLAG:0 = 7\nASSERT 0\nFLAG:0 = 9\nRETURN\n\
        @BEFORE_ERROR\nFLAG:1 += 1\nRETURN\n";
    let (session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 7);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
    let fault = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::Fault(fault) => Some(fault),
            _ => None,
        })
        .expect("original assertion fault");
    assert_eq!(fault.message, "ASSERT failed");
    assert!(
        fault
            .vm
            .as_ref()
            .is_some_and(|vm_fault| vm_fault.secondary.is_none())
    );
}

#[test]
fn disabled_and_reference_profiles_do_not_run_final_fault_hooks() {
    let source = "@SYSTEM_TITLE\nASSERT 0\nRETURN\n@BEFORE_ERROR\nFLAG:1 += 1\nRETURN\n";
    let compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let mut session = negotiated_session();
    let mut config = profile_configuration_file(compatibility.profile);
    let FilePayload::Utf8(contents) = &mut config.payload else {
        unreachable!("profile configuration is UTF-8")
    };
    contents.push_str("[runtime]\ndisable_before_error_throw = true\n");
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility,
            project_revision: 1,
            files: vec![
                config,
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(message,
        RuntimeMessage::ProjectLoadReport(report) if report.success)));
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[1], None).unwrap(),
        0
    );
    assert!(
        !session
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .artifact()
            .call_compatibility
            .before_error_throw_hooks
    );

    let (reference, _, _) = run_immediate_query_project(source);
    assert_eq!(
        read_runtime_integer(reference.vm.as_ref().unwrap(), "FLAG", &[1], None).unwrap(),
        0
    );
    assert!(
        !reference
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .artifact()
            .call_compatibility
            .before_error_throw_hooks
    );
    let (enabled, _, _) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let disabled_artifact = session.vm.as_ref().unwrap().artifact_id();
    let enabled_vm = enabled.vm.as_ref().unwrap();
    assert!(
        enabled_vm
            .vm()
            .artifact()
            .call_compatibility
            .before_error_throw_hooks
    );
    assert_ne!(disabled_artifact, enabled_vm.artifact_id());
}

#[test]
fn stable_snapshot_is_rejected_while_before_error_waits_for_input() {
    let source = "@SYSTEM_TITLE\nASSERT 0\nRETURN\n@BEFORE_ERROR\nINPUT\nRETURN\n";
    let (mut session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:#?}");
    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    let messages = drain(&mut session);
    assert!(
        messages.iter().any(|message| matches!(message,
        RuntimeMessage::StateExportReady(StateExportReady {
            result: StateExportResult::Ineligible { reasons }, ..
        }) if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable))),
        "{messages:#?}"
    );
}

#[test]
fn runaway_resource_fault_does_not_enter_before_error() {
    let source = "@SYSTEM_TITLE\nWHILE 1\nWEND\nRETURN\n@BEFORE_ERROR\nFLAG:1 += 1\nRETURN\n";
    let (mut session, _, _) = run_immediate_query_project_with_budget(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        RuntimeDriveBudget {
            maximum_vm_instructions: 1,
            maximum_runtime_transitions: 1,
        },
    );
    for _ in 0..140 {
        if session.phase() == RuntimePhase::Faulted {
            break;
        }
        session
            .drive(RuntimeDriveBudget {
                maximum_vm_instructions: 1,
                maximum_runtime_transitions: 1,
            })
            .unwrap();
        drain(&mut session);
    }
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[1], None).unwrap(),
        0
    );
}

#[test]
fn host_completion_remains_work_at_transition_and_instruction_boundaries() {
    let (mut session, report, _) = run_immediate_query_project_with_budget(
        "@SYSTEM_TITLE\nTHROW boundary\nRETURN\n",
        erabasic_compat::CompatibilityIdentity::default(),
        RuntimeDriveBudget {
            maximum_vm_instructions: 1_000,
            maximum_runtime_transitions: 2,
        },
    );
    assert_eq!(report.runtime_transitions, 2);
    assert_eq!(session.phase(), RuntimePhase::Running);
    assert!(session.vm.as_ref().unwrap().has_pending_events());
    assert!(!session.vm.as_ref().unwrap().has_runnable_fibers());
    let completion = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 0,
            maximum_runtime_transitions: 1,
        })
        .unwrap();
    assert_eq!(completion.vm_instructions, 0);
    assert_eq!(completion.state, RuntimeDriveState::Faulted);
    let messages = drain(&mut session);
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message,
                RuntimeMessage::Fault(fault) if fault.message == "boundary"
            ))
            .count(),
        1
    );
}

#[test]
fn snake_strformcheck_catches_host_assert_and_throw_without_rollback() {
    for statement in ["ASSERT 0", "THROW explicit-host-error"] {
        let source = format!(
            "@SYSTEM_TITLE\nFLAG:0 = STRFORMCHECK(\"{{FAIL()}}\")\nFLAG:2 = 1\nWAIT\nRETURN\n@FAIL\n#FUNCTION\nFLAG:1 += 1\n{statement}\nFLAG:1 = 99\nRETURNF 1\n"
        );
        let (session, _, messages) = run_immediate_query_project_with_profile(
            &source,
            erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let vm = session.vm.as_ref().unwrap();
        assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 0);
        assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
        assert_eq!(read_runtime_integer(vm, "FLAG", &[2], None).unwrap(), 1);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::Fault(_)))
        );
    }
}

#[test]
fn html_error_wire_data_cannot_claim_script_provenance() {
    let error = erabasic_html::decode_query_entities(
        "&unknown;",
        erabasic_html::HtmlQueryEntityPolicy::ReferenceQuery,
        erabasic_html::HtmlQueryLimits::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.origin(),
        erabasic_html::HtmlQueryErrorOrigin::ScriptInput
    );
    let mut serialized = serde_json::to_value(&error).unwrap();
    assert!(serialized.get("origin").is_none());
    serialized["origin"] = serde_json::json!("ScriptInput");
    let decoded: erabasic_html::HtmlQueryError = serde_json::from_value(serialized).unwrap();
    assert_eq!(
        decoded.origin(),
        erabasic_html::HtmlQueryErrorOrigin::NonScript
    );
    assert_eq!(
        (decoded.kind, &decoded.range, &decoded.message),
        (error.kind, &error.range, &error.message)
    );
}

#[test]
fn host_scalar_and_read_failures_only_preserve_explicit_script_sources() {
    assert!(matches!(
        i32_argument_value(&[VmValue::Integer(i64::MAX)], 0),
        Err(RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            ..
        })
    ));
    assert!(matches!(
        i32_argument_value(&[VmValue::String("bad".into())], 0),
        Err(RuntimeError::Internal(_))
    ));
    assert!(matches!(
        checked_argb(-1),
        Err(RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Argument,
            ..
        })
    ));
    let explicit = erabasic_vm::ExecutionFailure::script(
        erabasic_vm::ScriptFaultKind::Bounds,
        erabasic_vm::VmFaultCode::InvalidInstruction,
        "bounds",
    );
    assert!(matches!(
        runtime_script_read_error(erabasic_vm::VmError::ScriptFailure(explicit)),
        RuntimeError::Script {
            kind: erabasic_vm::ScriptFaultKind::Bounds,
            ..
        }
    ));
    let internal =
        erabasic_vm::ExecutionFailure::new(erabasic_vm::VmFaultCode::InvalidInstruction, "bounds");
    assert!(matches!(
        runtime_script_read_error(erabasic_vm::VmError::ScriptFailure(internal)),
        RuntimeError::Internal(_)
    ));
    assert!(matches!(
        runtime_script_read_error(erabasic_vm::VmError::InvalidArguments("bounds".into())),
        RuntimeError::Internal(_)
    ));
}

#[test]
fn direct_runtime_host_uses_existing_domain_errors_and_unsupported_boundary() {
    for (expression, faulted) in [
        ("{HOTKEY_STATE(0,0)}", false),
        ("{GETMEMORYUSAGE()}", true),
        ("{SPRITECREATE(\"x\",0,0,0,1,1,1,1)}", true),
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nRESULTS:0 '= \"{expression}\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:1 = 1\nWAIT\nRETURN\n"
        );
        let (session, _, messages) = run_immediate_query_project_with_profile(
            &source,
            erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
        );
        let vm = session.vm.as_ref().unwrap();
        assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 0);
        if faulted {
            assert_eq!(session.phase(), RuntimePhase::Faulted);
            assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 0);
            assert!(messages.iter().any(|message| matches!(
                message,
                RuntimeMessage::Fault(RuntimeFault {
                    code: FaultCode::UnsupportedRuntimeFeature,
                    ..
                })
            )));
        } else {
            assert_eq!(session.phase(), RuntimePhase::WaitingInput);
            assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
            assert!(
                !messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::Fault(_)))
            );
        }
    }
}

#[test]
fn dynamic_varsize_uses_host_defaults_but_callstr_unique_restructure_dereferences_null() {
    let source = "@SYSTEM_TITLE\nFLAG:10 = VARSIZE(\"FLAG\")\nRESULTS:10 '= \"{VARSIZE(\\\"FLAG\\\")}|{VARSIZE(\\\"FLAG\\\",0)}|{VARSIZE(\\\"FLAG\\\",,)}\"\nRESULTS:12 '= STRFORM(RESULTS:10)\nRESULTS:11 '= \"TAKE(VARSIZE(\\\"FLAG\\\",,))\"\nFLAG:0 = STRFORMCHECK(\"{CALLER()}\")\nWAIT\nRETURN\n@CALLER\n#FUNCTION\nFLAG:1 += 1\nTRYCCALLSTR RESULTS:11\nCATCH\nFLAG:2 = 1\nENDCATCH\nFLAG:3 = 1\nRETURNF 1\n@TAKE(ARG)\nFLAG:4 = 1\nRETURN\n";
    let (session, _, messages) = run_immediate_query_project_with_profile(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    let vm = session.vm.as_ref().unwrap();
    let length = read_runtime_integer(vm, "FLAG", &[10], None).unwrap();
    let text = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, "RESULTS").unwrap(),
            indices: vec![12],
            character: None,
        }])
        .unwrap()
        .remove(0);
    assert_eq!(text, VmValue::String(format!("{length}|{length}|{length}")));
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 1);
    for index in [2, 3, 4] {
        assert_eq!(read_runtime_integer(vm, "FLAG", &[index], None).unwrap(), 0);
    }
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn varsize_dimension_narrowing_is_shared_by_static_and_dynamic_calls_in_both_profiles() {
    let source = "@SYSTEM_TITLE\nFLAG:10 = VARSIZE(\"FLAG\")\nFLAG:11 = VARSIZE(\"FLAG\",4294967296)\nRESULTS:10 = {VARSIZE(\"FLAG\",4294967296)}|{VARSIZE(\"FLAG\",(-9223372036854775807 - 1))}\nRESULTS:12 '= STRFORM(RESULTS:10)\nWAIT\nRETURN\n";
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let (session, _, messages) = run_immediate_query_project_with_profile(
            source,
            erabasic_compat::CompatibilityIdentity::for_profile(profile),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
        let vm = session.vm.as_ref().unwrap();
        let length = read_runtime_integer(vm, "FLAG", &[10], None).unwrap();
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[11], None).unwrap(),
            length
        );
        let value = vm
            .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
                variable: runtime_variable_key(vm, "RESULTS").unwrap(),
                indices: vec![12],
                character: None,
            }])
            .unwrap()
            .remove(0);
        assert_eq!(value, VmValue::String(format!("{length}|{length}")));
    }
}

#[test]
fn animation_timer_preserves_profile_forms_and_snake_command_result() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nRESULT = 77\nSETANIMETIMER 1\nFLAG:0 = RESULT\nFLAG:1 = GETANIMETIMER()\nBITMAP_CACHE_ENABLE 1\nBITMAP_CACHE_ENABLE 0\nFLAG:2 = RESULT\nWAIT\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 77);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[1], None).unwrap(), 10);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[2], None).unwrap(), 77);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .resource_graph
            .animation_timer(),
        10
    );
    let notices = messages
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::Diagnostic(diagnostic)
                if diagnostic.code == "compat.bitmap_cache_enable_noop" =>
            {
                Some(diagnostic)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 1, "{messages:#?}");
    assert_eq!(notices[0].level, RuntimeLogLevel::Warning);
    assert_eq!(notices[0].notification, DiagnosticNotification::LogOnly);
    assert_eq!(
        notices[0]
            .context
            .as_ref()
            .and_then(|context| context.api.as_deref()),
        Some("bitmap_cache_enable")
    );

    let (session, _, messages) = run_immediate_query_project(
        "@SYSTEM_TITLE\nRESULT = SETANIMETIMER(1)\nFLAG:0 = RESULT\nWAIT\nRETURN\n",
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        1
    );
}

#[test]
fn snake_display_queries_and_whole_line_background_use_canonical_history() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nPRINTL oldest\nPRINT pending\nRESULTS '= GETDISPLAYLINE(-1)\nTEXT_BGC_ON 1122867, 50\nWAIT\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        read_runtime_string(session.vm.as_ref().unwrap(), "RESULTS").unwrap(),
        "oldest"
    );
    let snapshot = session.presentation.snapshot();
    assert_eq!(
        snapshot.settings.text_line_background,
        Some(era_runtime_protocol::Color {
            red: 0x11,
            green: 0x22,
            blue: 0x33,
            alpha: 127,
        })
    );
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .all(|line| line.text_background_eligible)
    );
}

#[test]
fn invalid_animation_timer_is_atomic() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let (session, _, messages) = run_immediate_query_project_with_profile(
        "@SYSTEM_TITLE\nSETANIMETIMER 20\nSETANIMETIMER 32768\nRETURN\n",
        snake,
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted, "{messages:?}");
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .resource_graph
            .animation_timer(),
        20
    );
    assert!(
        messages.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => {
                snapshot.resources.animation_timer_ms == 20
            }
            RuntimeMessage::PresentationDelta(delta) => {
                delta.operations.iter().any(|operation| {
                    matches!(
                        operation,
                        PresentationOperation::SetResources { resources }
                            if resources.animation_timer_ms == 20
                    )
                })
            }
            _ => false,
        }),
        "{messages:#?}"
    );
}
