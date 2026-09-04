
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
    for (expression, checked, faulted) in [
        ("{HOTKEY_STATE(0,0)}", 0, false),
        ("{GETMEMORYUSAGE()}", 0, true),
        ("{SPRITECREATE(\"x\",0,0,0,1,1,1,1)}", 1, false),
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
        assert_eq!(
            read_runtime_integer(vm, "FLAG", &[0], None).unwrap(),
            checked
        );
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
