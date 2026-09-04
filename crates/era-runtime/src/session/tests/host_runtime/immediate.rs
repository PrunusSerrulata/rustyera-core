use super::*;
use era_runtime_protocol::{AudioEffectAction, AudioPlaybackStateV1};

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
    let choices = &session
        .operations
        .active_input()
        .expect("FORCEWAIT")
        .choices;
    assert_eq!(
        choices.values().cloned().collect::<Vec<_>>(),
        [VmValue::Integer(7), VmValue::String("word".into())]
    );
    let tokens = choices.keys().copied().collect::<Vec<_>>();
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
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("FORCEWAIT")
            .choices
            .len(),
        128
    );
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
fn prefixed_hex_html_colors_are_shared_by_both_profiles() {
    let source =
        "@SYSTEM_TITLE\nHTML_PRINT \"<font color='#0x90EE90'>ok</font>\"\nFORCEWAIT\nRETURN\n";
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let (session, _, messages) = run_immediate_query_project_with_profile(
            source,
            erabasic_compat::CompatibilityIdentity::for_profile(profile),
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{profile:?}");
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::Fault(_))),
            "{profile:?}: {messages:#?}"
        );
        assert!(projected_presentation_text(&session.presentation.snapshot()).contains("ok"));
    }
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
