#[test]
fn malformed_pointer_and_canvas_replies_fault_without_losing_the_host_wait() {
    let queries = [
        (
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
            "",
            "RESULT = MOUSEX()",
        ),
        (
            ServiceKind::InputState,
            POINTER_STATE_OPERATION,
            POINTER_STATE_OPERATION_VERSION,
            "",
            "RESULTS '= MOUSEB()",
        ),
        (
            ServiceKind::Canvas,
            SAMPLE_CANVAS_PIXEL_OPERATION,
            SAMPLE_CANVAS_PIXEL_OPERATION_VERSION,
            "RESULT = GCREATE(1, 2, 2)\n",
            "RESULT = GGETCOLOR(1, 0, 0)",
        ),
    ];
    for (kind, operation, version, setup, assignment) in queries {
        for payload in [
            vec![0xa1, 0x00],                   // Truncated field value.
            vec![0xa1, 0x00, 0x61, b'x'],       // Wrong type for the first field.
            vec![0xa2, 0x00, 0x00, 0x00, 0x00], // Duplicate deterministic map key.
            vec![0xbf, 0xff],                   // Indefinite maps are not deterministic.
            vec![0xa0, 0x00],                   // Trailing data after a complete map.
        ] {
            let source = format!(
                "@SYSTEM_TITLE\n{setup}RESULT = 777\nRESULTS '= \"kept\"\nRESULT:9 = 991\n{assignment}\nRESULT:9 = 0\nWAIT\nRETURN\n"
            );
            let (mut session, request, _) =
                start_projection_service_with_messages(&source, kind, operation, version);
            complete_projection_reply(&mut session, &request, payload.clone());
            assert_service_failure(&mut session);
            let vm = session.vm.as_ref().unwrap();
            assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 777);
            assert_eq!(read_runtime_integer(vm, "RESULT", &[9], None).unwrap(), 991);
            assert_eq!(
                vm.vm()
                    .read_variable(runtime_variable_key(vm, "RESULTS").unwrap(), &[0], None)
                    .unwrap(),
                VmValue::String("kept".into()),
            );

            // Exercise the public drive path again: the failed request cannot be
            // rebound, and the session must remain faulted rather than waiting.
            submit(
                &mut session,
                4,
                RuntimeMessage::ServiceResponse(ServiceResponse {
                    request_id: request.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(payload),
                    },
                }),
            );
            session.drive(RuntimeDriveBudget::default()).unwrap();
            let messages = drain(&mut session);
            assert_eq!(session.phase(), RuntimePhase::Faulted);
            assert!(
                messages.iter().any(|message| matches!(
                    message,
                    RuntimeMessage::CommandRejected(CommandRejected {
                        code: CommandErrorCode::StaleRequest,
                        ..
                    })
                )),
                "{operation}: {messages:#?}"
            );
        }
    }
}

#[test]
fn snake_strformcheck_catches_later_html_parser_fault_after_service_completion() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 = old-head\nRESULTS:1 = old-tail\nFLAG:0 = STRFORMCHECK(\"%BAD_HTML()%\")\nFLAG:1 = 1\nWAIT\nRETURN\n@BAD_HTML\n#FUNCTIONS\nRETURNF HTML_SUBSTRING(\"a</b>\", 100)\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!((html_flag(&session, 0), html_flag(&session, 1)), (0, 1));
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServiceRequest(_)))
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(html_result(vm, 0), VmValue::String("old-head".into()));
    assert_eq!(html_result(vm, 1), VmValue::String("old-tail".into()));
    assert!(session.operations.html_lines.is_empty());
}

#[test]
fn snake_strformcheck_cannot_catch_frontend_claimed_script_failure() {
    let source = "@SYSTEM_TITLE\nFLAG:0 = STRFORMCHECK(\"%MEASURE_HTML()%\")\nFLAG:1 = 1\nWAIT\nRETURN\n@MEASURE_HTML\n#FUNCTIONS\nRETURNF HTML_SUBSTRING(\"abc\", 100)\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .expect("measurement request");
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "script.parse".into(),
                    message: "frontend cannot declare ScriptInput".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(html_flag(&session, 1), 0);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            ..
        })
    )));
}

#[test]
fn direct_html_host_failure_catches_and_abandons_only_its_live_flow_scope() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"{HTML_STRINGLINES(\\\"abc\\\", WIDTH())}\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:2 = 1\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:1 += 1\nTHROW width-failed\nRETURNF 1\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(
        (
            html_flag(&session, 0),
            html_flag(&session, 1),
            html_flag(&session, 2)
        ),
        (0, 1, 1)
    );
    assert!(session.operations.html_lines.is_empty());
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn direct_html_outer_scope_survives_inner_check_and_repeated_width_evaluation() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"{HTML_STRINGLINES(\\\"abc\\\", WIDTH())}\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nWAIT\nRETURN\n@WIDTH\n#FUNCTION\nFLAG:1 += 1\nFLAG:2 = STRFORMCHECK(\"{FAIL()}\")\nRETURNF 1\n@FAIL\n#FUNCTION\nTHROW inner-failed\nRETURNF 0\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    let (_, messages) = start_html_execution(&mut session);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{messages:?}");
    assert_eq!(html_flag(&session, 0), 1);
    assert!(
        html_flag(&session, 1) > 1,
        "width must run again for each nonempty tail"
    );
    assert_eq!(html_flag(&session, 2), 0);
    assert!(session.operations.html_lines.is_empty());
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_)))
    );
}

#[test]
fn direct_html_host_cannot_catch_frontend_claimed_script_failure() {
    let source = "@SYSTEM_TITLE\nRESULTS:0 '= \"%HTML_SUBSTRING(\\\"abc\\\",100)%\"\nFLAG:0 = STRFORMCHECK(RESULTS:0)\nFLAG:1 = 1\nWAIT\nRETURN\n";
    let mut session = prepare_html_execution_with_profile(
        source,
        Some(ProtocolVersion::new(2, 0)),
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .expect("direct measurement request");
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "script.parse".into(),
                    message: "untrusted frontend failure".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(html_flag(&session, 1), 0);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            ..
        })
    )));
    assert!(session.operations.html_lines.is_empty());
}
