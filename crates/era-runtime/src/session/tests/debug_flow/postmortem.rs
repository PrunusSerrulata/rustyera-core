fn postmortem_exchange(session: &mut RuntimeSession, message: &DebugMessage) -> Vec<DebugMessage> {
    let sequence = session.expected_debug_sequence;
    submit_debug(session, sequence, message);
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let mut messages = Vec::new();
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
        if envelope.channel == Channel::Debug {
            messages.push(DebugMessage::from_envelope(&envelope).unwrap());
        }
    }
    messages
}

fn postmortem_request(
    session: &mut RuntimeSession,
    grant: GrantToken,
    command: DebugCommand,
) -> Vec<DebugMessage> {
    postmortem_exchange(
        session,
        &DebugMessage::Request(AuthorizedDebugRequest { grant, command }),
    )
}

fn postmortem_grant(session: &mut RuntimeSession, scopes: Vec<DebugScope>) -> GrantToken {
    let messages = postmortem_exchange(
        session,
        &DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: scopes,
        }),
    );
    let [DebugMessage::Grant(grant)] = messages.as_slice() else {
        panic!("expected debug grant: {messages:?}");
    };
    grant.token
}

fn postmortem_pause(session: &mut RuntimeSession, grant: GrantToken) -> StopToken {
    let messages = postmortem_request(session, grant, DebugCommand::Pause);
    assert!(messages.contains(&DebugMessage::Response(DebugResponse::Accepted)));
    messages
        .into_iter()
        .find_map(|message| match message {
            DebugMessage::Stopped(stopped) => Some(stopped.stop),
            _ => None,
        })
        .expect("postmortem stop")
}

fn postmortem_error(messages: &[DebugMessage], expected: DebugErrorCode) {
    assert!(
        matches!(messages, [DebugMessage::Error(error)] if error.code == expected),
        "expected {expected:?}, got {messages:?}"
    );
}

fn faulted_debug_session(fault_expression: &str) -> (RuntimeSession, GrantToken) {
    faulted_debug_session_with_profile(fault_expression, false)
}

fn faulted_debug_session_with_profile(
    fault_expression: &str,
    snake: bool,
) -> (RuntimeSession, GrantToken) {
    let mut session = negotiated_session();
    session.options.debug_scope_mask = u64::MAX;
    let mut files = vec![SubmittedFile {
        relative_path: "main.erb".into(),
        category: FileCategory::Erb,
        payload: FilePayload::Utf8(format!(
            "@SYSTEM_TITLE\nRESULT:10 = 777\nRESULT:10 = {fault_expression}\nRESULT:11 = 999\nRETURN\n",
        )),
        content_hash: None,
    }];
    let compatibility = if snake {
        files.push(SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(
                "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n"
                    .into(),
            ),
            content_hash: None,
        });
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        )
    } else {
        erabasic_compat::CompatibilityIdentity::reference()
    };
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility,
            project_revision: 1,
            files,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message, RuntimeMessage::ProjectLoadReport(report) if report.success
    )));
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(messages.iter().any(|message| matches!(
        message, RuntimeMessage::Fault(fault) if fault.code == FaultCode::VmFault
    )));
    let grant = postmortem_grant(
        &mut session,
        vec![
            DebugScope::VariablesRead,
            DebugScope::VariablesWrite,
            DebugScope::GameFieldsRead,
            DebugScope::GameFieldsWrite,
            DebugScope::ExecutionRead,
            DebugScope::ExecutionControl,
            DebugScope::ConsoleEvaluate,
            DebugScope::ConsoleExecute,
            DebugScope::BreakpointsManage,
            DebugScope::ScriptOutput,
        ],
    );
    (session, grant)
}

#[test]
fn postmortem_console_uses_active_snake_policy_without_mutating_stop() {
    let (mut session, grant) = faulted_debug_session_with_profile("GCREATE(752, 0, 1)", true);
    let stop = postmortem_pause(&mut session, grant);
    for (source, expected, warning) in [
        (
            "9223372036854775807 + 1",
            i64::MAX,
            Some("compat.arithmetic.overflow"),
        ),
        (
            "9223372036854775807 + 1",
            i64::MAX,
            Some("compat.arithmetic.overflow"),
        ),
        ("TOINT(\"9223372036854775808\")", 0, None),
        ("UNCHECKED_MUL(9223372036854775807, 2)", -2, None),
        ("1 || (1 / 0)", 1, None),
    ] {
        let messages = postmortem_request(
            &mut session,
            grant,
            DebugCommand::Console {
                stop,
                command: ConsoleCommand::Evaluate {
                    source: source.into(),
                },
            },
        );
        let [DebugMessage::Response(DebugResponse::Console(outcome))] = messages.as_slice() else {
            panic!("expected safe snake evaluation: {messages:?}");
        };
        assert_eq!(
            outcome.value,
            Some(DebugValue::Integer(expected)),
            "{source}"
        );
        assert_eq!(
            outcome
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            warning.into_iter().collect::<Vec<_>>()
        );
        assert!(outcome.changed_variables.is_empty());
        assert_eq!(outcome.stop, stop);
    }
    assert_eq!(session.revision, stop.runtime_revision);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[10], None).unwrap(),
        777
    );
}

fn postmortem_result_reference(
    session: &mut RuntimeSession,
    grant: GrantToken,
    stop: StopToken,
) -> VariableReference {
    let messages = postmortem_request(
        session,
        grant,
        DebugCommand::ListVariables {
            stop,
            cursor: None,
            limit: 1024,
        },
    );
    let [DebugMessage::Response(DebugResponse::VariablePage(page))] = messages.as_slice() else {
        panic!("expected variables: {messages:?}");
    };
    let result = page
        .variables
        .iter()
        .find(|variable| variable.name == "RESULT")
        .unwrap();
    VariableReference {
        symbol_key: result.symbol_key.clone(),
        storage: VariableStorage::Global,
        fiber_id: None,
        frame_id: None,
        generation: stop.program_generation,
        character: None,
        indices: vec![10],
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn postmortem_debug_reads_the_fault_site_and_continue_keeps_it_faulted() {
    let (mut session, grant) = faulted_debug_session("GCREATE(752, 0, 1)");
    let stop = postmortem_pause(&mut session, grant);
    assert_eq!(session.debug_resume_phase, Some(RuntimePhase::Faulted));
    let reference = postmortem_result_reference(&mut session, grant, stop);
    let read = DebugCommand::ReadVariable {
        stop,
        value: reference.clone(),
    };
    let messages = postmortem_request(&mut session, grant, read.clone());
    assert!(
        matches!(messages.as_slice(), [DebugMessage::Response(DebugResponse::VariableValue(value))]
        if value.value == DebugValue::Integer(777))
    );
    let fibers = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ListFibers {
            stop,
            cursor: None,
            limit: 16,
        },
    );
    let [DebugMessage::Response(DebugResponse::FiberPage(page))] = fibers.as_slice() else {
        panic!("expected fibers: {fibers:?}");
    };
    let fiber = page
        .fibers
        .first()
        .expect("fault-site fiber remains available")
        .fiber_id;
    let stack = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ReadCallStack {
            stop,
            fiber_id: fiber,
        },
    );
    assert!(
        matches!(stack.as_slice(), [DebugMessage::Response(DebugResponse::CallStack(stack))]
        if !stack.frames.is_empty())
    );
    let field = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ReadGameField {
            stop,
            key: "input.message_skip".into(),
        },
    );
    assert!(
        matches!(field.as_slice(), [DebugMessage::Response(DebugResponse::GameFieldValue(value))]
        if value.value == DebugValue::Boolean(false))
    );
    for (source, expected) in [("ABS(-7)", Some(DebugValue::Integer(7))), ("RAND(3)", None)] {
        let messages = postmortem_request(
            &mut session,
            grant,
            DebugCommand::Console {
                stop,
                command: ConsoleCommand::Evaluate {
                    source: source.into(),
                },
            },
        );
        let [DebugMessage::Response(DebugResponse::Console(outcome))] = messages.as_slice() else {
            panic!("expected safe evaluation: {messages:?}");
        };
        assert_eq!(outcome.value, expected);
        assert_eq!(outcome.diagnostics.is_empty(), expected.is_some());
        assert!(outcome.changed_variables.is_empty());
        assert!(outcome.changed_game_fields.is_empty());
        assert_eq!(outcome.stop, stop);
    }
    assert_eq!(
        postmortem_request(&mut session, grant, DebugCommand::Continue { stop }),
        vec![DebugMessage::Response(DebugResponse::Accepted)]
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(session.vm.as_ref().unwrap().stop_token().is_none());
    let report = session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(report.state, RuntimeDriveState::Faulted);
    assert_eq!(report.vm_instructions, 0);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[11], None).unwrap(),
        0
    );
    postmortem_error(
        &postmortem_request(&mut session, grant, read),
        DebugErrorCode::StaleStop,
    );
    let next = postmortem_pause(&mut session, grant);
    assert_ne!(next.pause_epoch, stop.pause_epoch);
    postmortem_error(
        &postmortem_request(&mut session, grant, DebugCommand::Continue { stop }),
        DebugErrorCode::StaleStop,
    );
    let messages = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ReadVariable {
            stop: next,
            value: reference,
        },
    );
    assert!(
        matches!(messages.as_slice(), [DebugMessage::Response(DebugResponse::VariableValue(value))]
        if value.value == DebugValue::Integer(777))
    );
}

#[test]
fn postmortem_debug_rejects_mutations_and_revoke_preserves_the_fault() {
    let (mut session, grant) = faulted_debug_session("GCREATE(752, 0, 1)");
    let stop = postmortem_pause(&mut session, grant);
    let reference = postmortem_result_reference(&mut session, grant, stop);
    for command in [
        DebugCommand::Step {
            stop,
            fiber_id: 0,
            kind: StepKind::Instruction,
        },
        DebugCommand::WriteVariables {
            stop,
            writes: vec![VariableWrite {
                reference: reference.clone(),
                value: DebugValue::Integer(123),
                expected_revision: 0,
            }],
        },
        DebugCommand::WriteGameFields {
            stop,
            writes: vec![GameFieldWrite {
                key: "input.message_skip".into(),
                value: DebugValue::Boolean(true),
                expected_revision: stop.runtime_revision,
            }],
        },
        DebugCommand::Console {
            stop,
            command: ConsoleCommand::ExecuteSafe {
                source: "RESULT = 123".into(),
            },
        },
        DebugCommand::UpdateBreakpoints {
            update: BreakpointUpdate {
                requested: Vec::new(),
                remove: Vec::new(),
            },
        },
    ] {
        postmortem_error(
            &postmortem_request(&mut session, grant, command),
            DebugErrorCode::InvalidState,
        );
        assert_eq!(session.phase(), RuntimePhase::DebugPaused);
        assert_eq!(session.revision, stop.runtime_revision);
    }
    postmortem_error(
        &postmortem_request(
            &mut session,
            grant,
            DebugCommand::Step {
                stop: StopToken {
                    session_epoch: stop.session_epoch + 1,
                    ..stop
                },
                fiber_id: 0,
                kind: StepKind::Instruction,
            },
        ),
        DebugErrorCode::StaleStop,
    );
    let messages = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ReadVariable {
            stop,
            value: reference,
        },
    );
    assert!(
        matches!(messages.as_slice(), [DebugMessage::Response(DebugResponse::VariableValue(value))]
        if value.value == DebugValue::Integer(777))
    );
    assert!(!session.message_skip);
    postmortem_exchange(
        &mut session,
        &DebugMessage::Revoke(DebugRevoke {
            grant_id: grant.grant_id,
            reason: "close postmortem inspection".into(),
        }),
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(session.vm.as_ref().unwrap().stop_token().is_none());
    postmortem_error(
        &postmortem_request(&mut session, grant, DebugCommand::Pause),
        DebugErrorCode::PermissionDenied,
    );
}

#[test]
fn postmortem_debug_requires_granted_control_a_current_stop_and_an_active_vm() {
    let (mut session, old_grant) = faulted_debug_session("GCREATE(752, 0, 1)");
    let grant = postmortem_grant(&mut session, vec![DebugScope::VariablesRead]);
    for token in [old_grant, grant] {
        postmortem_error(
            &postmortem_request(&mut session, token, DebugCommand::Pause),
            DebugErrorCode::PermissionDenied,
        );
    }
    let grant = postmortem_grant(
        &mut session,
        vec![DebugScope::ExecutionControl, DebugScope::VariablesRead],
    );
    let stop = postmortem_pause(&mut session, grant);
    for stale in [
        StopToken {
            session_epoch: stop.session_epoch + 1,
            ..stop
        },
        StopToken {
            program_generation: stop.program_generation + 1,
            ..stop
        },
        StopToken {
            runtime_revision: stop.runtime_revision + 1,
            ..stop
        },
    ] {
        postmortem_error(
            &postmortem_request(
                &mut session,
                grant,
                DebugCommand::ListVariables {
                    stop: stale,
                    cursor: None,
                    limit: 1,
                },
            ),
            DebugErrorCode::StaleStop,
        );
    }
    postmortem_request(&mut session, grant, DebugCommand::Continue { stop });
    session.vm = None;
    postmortem_error(
        &postmortem_request(&mut session, grant, DebugCommand::Pause),
        DebugErrorCode::InvalidState,
    );
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert!(session.debug_resume_phase.is_none());
}

#[test]
fn postmortem_debug_continue_preserves_the_vm_fiber_fault() {
    let (mut session, grant) = faulted_debug_session("1 / RESULT:11");
    let stop = postmortem_pause(&mut session, grant);
    let fibers = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ListFibers {
            stop,
            cursor: None,
            limit: 16,
        },
    );
    let [DebugMessage::Response(DebugResponse::FiberPage(page))] = fibers.as_slice() else {
        panic!("expected faulted fiber: {fibers:?}");
    };
    let fiber = page
        .fibers
        .iter()
        .find(|fiber| fiber.state == era_debug_protocol::FiberState::Faulted)
        .expect("VM arithmetic fault remains inspectable")
        .fiber_id;
    let fault = session
        .vm
        .as_ref()
        .unwrap()
        .fiber_status(erabasic_vm::FiberId(fiber));
    let reference = postmortem_result_reference(&mut session, grant, stop);
    let messages = postmortem_request(
        &mut session,
        grant,
        DebugCommand::ReadVariable {
            stop,
            value: reference,
        },
    );
    assert!(
        matches!(messages.as_slice(), [DebugMessage::Response(DebugResponse::VariableValue(value))]
        if value.value == DebugValue::Integer(777))
    );
    postmortem_request(&mut session, grant, DebugCommand::Continue { stop });
    assert_eq!(session.phase(), RuntimePhase::Faulted);
    assert_eq!(
        session
            .vm
            .as_ref()
            .unwrap()
            .fiber_status(erabasic_vm::FiberId(fiber)),
        fault
    );
    assert_eq!(
        session
            .drive(RuntimeDriveBudget::default())
            .unwrap()
            .vm_instructions,
        0
    );
}
