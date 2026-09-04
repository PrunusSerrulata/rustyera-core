use super::*;
#[test]
fn call_text_six_modes_accept_both_argument_syntaxes_and_preserve_jump_return() {
    for command in [
        "CALLSTR",
        "JUMPSTR",
        "TRYCALLSTR",
        "TRYJUMPSTR",
        "TRYCCALLSTR",
        "TRYCJUMPSTR",
    ] {
        for call in ["TAKE(7)", "TAKE, 7"] {
            let catch = if command.starts_with("TRYC") && !command.starts_with("TRYCALL") {
                "CATCH\nFLAG:2 = 99\nENDCATCH\n"
            } else {
                ""
            };
            let source = format!(
                "@SYSTEM_TITLE\nCALL OUTER\nFLAG:9 = 1\nRETURN\n@OUTER\n{command} {quoted}\nFLAG:1 = 1\n{catch}FLAG:3 = 1\nRETURN\n@TAKE(ARG)\nFLAG:0 = ARG\nRETURN\n",
                quoted = serde_json::to_string(call).unwrap(),
            );
            let artifact = compile_source_with_options(&source, &method_options(true));
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{command} {call}: {report:?}"
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{command} {call}: {report:?}"
            );
            let jumps = command.contains("JUMP");
            for (index, expected) in [
                (0, 7),
                (1, i64::from(!jumps)),
                (2, 0),
                (3, i64::from(!jumps)),
                (9, 1),
            ] {
                assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(expected));
            }
        }
    }
}

#[test]
fn blank_call_text_jump_is_successful_fallthrough_including_catch_mode() {
    for command in ["JUMPSTR", "TRYJUMPSTR", "TRYCJUMPSTR"] {
        let catch = if command == "TRYCJUMPSTR" {
            "CATCH\nFLAG:1 = 99\nENDCATCH\n"
        } else {
            ""
        };
        let artifact = compile_source_with_options(
            &format!("@SYSTEM_TITLE\n{command} \"   \"\nFLAG:0 = 1\n{catch}FLAG:2 = 2\nRETURN\n"),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for (index, value) in [(0, 1), (1, 0), (2, 2)] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
        }
    }
}

#[test]
fn snake_statement_extra_actuals_skip_side_effects_and_original_dynamic_calls_are_strict() {
    for (statement, method, continued) in [
        ("CALL TAKE, 7, SIDE()", false, 1),
        ("CALLFORM TAKE(7, SIDE())", false, 1),
        ("TRYCALLFORM TAKE(7, SIDE())", false, 1),
        ("CALLFORMF TAKE(7, SIDE())", true, 1),
        ("TRYCALLFORMF TAKE(7, SIDE())", true, 1),
        (
            "TRYCALLLIST\nFUNC MISSING, SIDE()\nFUNC TAKE, 7, SIDE()\nENDFUNC",
            false,
            1,
        ),
        (
            "TRYJUMPLIST\nFUNC MISSING, SIDE()\nFUNC TAKE, 7, SIDE()\nENDFUNC",
            false,
            0,
        ),
    ] {
        let kind = if method { "#FUNCTION\n" } else { "" };
        let returned = if method { "RETURNF ARG" } else { "RETURN" };
        let source = format!(
            "@SYSTEM_TITLE\n{statement}\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\n{kind}FLAG:1 = ARG\n{returned}\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF FLAG:9999999\n"
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{statement}: {report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for (index, value) in [(0, 0), (1, 7), (2, continued)] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
        }
    }
    // Dynamic resolution keeps the strict original policy observable at execution.
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nCALLFORM TAKE(7, SIDE())\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\nFLAG:1 = ARG\nRETURN\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 8\n",
        &method_options(false),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(matches!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Argument)
    ));
    for index in 0..3 {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(0));
    }
}

#[test]
fn call_text_jump_keeps_caller_local_ref_alive_through_recursive_forwarding() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
CALL OWNER
FLAG:9 = 1
RETURN
@OWNER
#LOCALSIZE 2
LOCAL:0 = 40
JUMPSTR "FORWARD(LOCAL:SIDE())"
FLAG:8 = 99
RETURN
@FORWARD(VALUES)
#DIM REF VALUES
CALLSTR "RECURSE(VALUES, 2, SIDE())"
FLAG:1 = VALUES:0
RETURN
@RECURSE(VALUES, ARG)
#DIM REF VALUES
IF ARG > 0
CALLSTR "RECURSE(VALUES, ARG - 1)"
ENDIF
VALUES:0 += 1
RETURN
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for (index, value) in [(0, 0), (1, 43), (8, 0), (9, 1)] {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
    }
}

#[test]
fn checked_form_child_failure_preserves_ref_writes_and_allows_a_fresh_call() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 10
RESULT:0 = STRFORMCHECK("{FAIL_WITH_REF(VALUES)}")
RESULT:1 = GOOD_WITH_REF(VALUES)
FLAG:9 = VALUES:0
RETURN
@FAIL_WITH_REF(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 1
RETURNF ITEMS:9999999
@GOOD_WITH_REF(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 2
RETURNF ITEMS:0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(13));
    assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(13));
}

fn checked_input_runtime() -> (
    RuntimeVm,
    BytecodeArtifact,
    FiberId,
    erabasic_vm::VmHostRequest,
) {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:10 = STRFORMCHECK("{WAITING_METHOD()}")
FLAG:9 = 1
RETURN
@WAITING_METHOD
#FUNCTION
FLAG:0 += 1
INPUT
FLAG:1 += 1
RETURNF 7
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    let request = report
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .expect("checked method must reach INPUT through the real runtime port");
    let prepared = runtime
        .validate_host_completion(
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::StableInput,
                rebind_payload: Vec::new(),
            },
        )
        .unwrap();
    runtime.commit_host_completion(prepared).unwrap();
    assert_eq!(
        runtime.fiber_status(fiber),
        Some(FiberStatus::WaitingHost(request.id))
    );
    (runtime, artifact, fiber, request)
}

#[test]
fn checked_form_async_input_success_and_typed_failures_keep_their_categories() {
    use erabasic_vm::{ExecutionFailure, FaultCategory, ScriptFaultKind};
    for category in [
        None,
        Some(FaultCategory::Script(ScriptFaultKind::Operation)),
        Some(FaultCategory::HostContract),
        Some(FaultCategory::Permission),
        Some(FaultCategory::ResourceLimit),
        Some(FaultCategory::Cancellation),
    ] {
        let (mut runtime, artifact, fiber, request) = checked_input_runtime();
        let completion = category.map_or_else(
            || VmHostCompletion::Ready(HostReady::empty()),
            |category| {
                VmHostCompletion::Error(ExecutionFailure::classified(
                    category,
                    VmFaultCode::Host,
                    "same legacy host message",
                ))
            },
        );
        let prepared = runtime
            .validate_host_completion(request.id, completion)
            .unwrap();
        runtime.commit_host_completion(prepared).unwrap();
        let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
        let catchable =
            category.is_some_and(|category| matches!(category, FaultCategory::Script(_)));
        if category.is_none() || catchable {
            assert!(
                matches!(runtime.fiber_status(fiber), Some(FiberStatus::Completed(_))),
                "{category:?}: {report:?}"
            );
            assert_method_watch(
                runtime.vm(),
                &artifact,
                "RESULT",
                10,
                VmValue::Integer(i64::from(!catchable)),
            );
            assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(1));
        } else {
            let Some(FiberStatus::Faulted(fault)) = runtime.fiber_status(fiber) else {
                panic!("{category:?}: {report:?}");
            };
            assert_eq!(Some(fault.category), category);
            assert_method_watch(runtime.vm(), &artifact, "RESULT", 10, VmValue::Integer(73));
            assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(0));
        }
        assert_method_watch(runtime.vm(), &artifact, "FLAG", 0, VmValue::Integer(1));
        assert_method_watch(
            runtime.vm(),
            &artifact,
            "FLAG",
            1,
            VmValue::Integer(i64::from(category.is_none())),
        );
        assert!(
            runtime
                .validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty()))
                .is_err()
        );
    }
}

#[test]
fn cancelling_checked_input_does_not_complete_the_checker_or_accept_late_input() {
    let (mut runtime, artifact, fiber, request) = checked_input_runtime();
    runtime.cancel_fiber(fiber).unwrap();
    assert_eq!(runtime.fiber_status(fiber), Some(FiberStatus::Cancelled));
    assert!(
        runtime
            .validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty()))
            .is_err()
    );
    runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert_method_watch(runtime.vm(), &artifact, "RESULT", 10, VmValue::Integer(73));
    assert_method_watch(runtime.vm(), &artifact, "FLAG", 9, VmValue::Integer(0));
}

#[test]
fn checked_form_does_not_catch_malformed_native_ready_value_or_write() {
    struct MalformedReady(bool);
    impl NativeService for MalformedReady {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            Ok(if self.0 {
                NativeReady::value(VmValue::String("wrong return type".into()))
            } else {
                NativeReady {
                    value: Some(VmValue::Integer(1)),
                    writes: vec![erabasic_vm::HostWrite {
                        target: erabasic_vm::PlaceDescriptor {
                            variable: SymbolKey::derive("test", b"missing-host-write"),
                            ..Default::default()
                        },
                        value: VmValue::Integer(2),
                    }],
                }
            })
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = 73\nRESULT:10 = STRFORMCHECK(\"{ABS(FLAG:0)}\")\nFLAG:9 = 1\nFLAG:8 = ABS(FLAG:0)\nRETURN\n",
        &method_options(true),
    );
    let key = artifact
        .runtime_native_authorizations
        .iter()
        .find(|family| family.name.eq_ignore_ascii_case("abs"))
        .unwrap()
        .key;
    for wrong_type in [true, false] {
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        natives.register(key, MalformedReady(wrong_type));
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert_eq!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::HostContract
        );
        assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(73));
        assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(0));
    }
}

#[test]
fn call_text_snapshot_rejects_deleted_child_origin_and_forged_root_before_native_restore() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#LOCALSIZE 2
LOCAL:0 = 5
RESULT:9 = RAND:1000000
CALLSTR "WAIT_CALL(LOCAL)"
FLAG:9 = 1
RETURN
@WAIT_CALL(VALUES)
#DIM REF VALUES
INPUT
VALUES:0 += 1
RETURN
"#,
        &method_options(true),
    );
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::WaitingHost(_))
    ));
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    let frames = &saved["fibers"][fiber.0.to_string()]["frames"];
    assert_eq!(frames.as_array().unwrap().len(), 2);
    assert!(!frames[1]["user_call"].is_null());
    for attack in [
        "delete_child_origin",
        "forge_bytecode_origin",
        "delete_call_root",
        "forge_value_root",
        "forge_jump_mode",
    ] {
        let mut corrupted = saved.clone();
        let frames = &mut corrupted["fibers"][fiber.0.to_string()]["frames"];
        match attack {
            "delete_child_origin" => frames[1]["user_call"] = serde_json::Value::Null,
            "forge_bytecode_origin" => {
                frames[1]["user_call"]["origin"] =
                    serde_json::json!({"Bytecode": {"resolve": 0, "invoke": 0}});
            }
            "delete_call_root" => frames[0]["runtime_form"] = serde_json::Value::Null,
            "forge_value_root" => {
                frames[0]["runtime_form"]["completion"] =
                    serde_json::json!({"Value": BytecodeType::String});
            }
            "forge_jump_mode" => {
                frames[0]["runtime_form"]["completion"]["Call"]["spec"]["mode"] =
                    serde_json::to_value(erabasic_bytecode::CallTextMode::Jump).unwrap();
            }
            _ => unreachable!(),
        }
        let snapshot: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        let mut rejected_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        let before = vm.encode_snapshot(&rejected_natives).unwrap();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                snapshot,
                &mut rejected_host,
                &mut rejected_natives
            )
            .is_err(),
            "{attack}"
        );
        assert!(rejected_host.rebound.is_empty(), "{attack}");
        assert_eq!(
            vm.encode_snapshot(&rejected_natives).unwrap(),
            before,
            "{attack}"
        );
    }
}

#[test]
fn original_profile_rejects_snake_call_text_and_checked_form_at_load() {
    for body in ["CALLSTR \"TARGET()\"", "RESULT = STRFORMCHECK(\"plain\")"] {
        let report = analyze_project(
            AnalysisInput {
                project_data: project_data(),
                sources: vec![ProjectSource {
                    relative_path: "profile-gate.erb".into(),
                    payload: SourcePayload::Utf8(format!(
                        "@SYSTEM_TITLE\n{body}\nRETURN\n@TARGET\nRETURN\n"
                    )),
                }],
            },
            &method_options(false),
            &ExtensionRegistry::default(),
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            "{body}: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn call_text_try_catches_only_argument_reduction_missing_target_and_binding_stages() {
    for (text, caught) in [
        ("TAKE(1 +)", true),
        ("MISSING(7)", true),
        ("TAKE(\"bad\")", true),
        ("TAKE(UNKNOWN_VARIABLE)", true),
        ("TAKE(1, UNKNOWN_VARIABLE)", true),
        ("TAKE(SIDE(), UNKNOWN_VARIABLE)", true),
        ("METHOD(UNKNOWN_VARIABLE)", true),
        ("TAKE(UNKNOWN_METHOD())", true),
        ("TAKE(1 + \"bad\")", true),
        ("METHOD()", false),
        ("TAKE(FLAG:9999999)", false),
        ("BROKEN()", false),
        ("TAKE(\"unterminated)", false),
    ] {
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nTRYCCALLSTR {}\nFLAG:0 = 99\nCATCH\nFLAG:1 = 1\nENDCATCH\nFLAG:2 = 1\nRETURN\n@TAKE(ARG)\nRETURN\n@METHOD\n#FUNCTION\nRETURNF 1\n@BROKEN\nRESULT = FLAG:9999999\nRETURN\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 1\n",
                serde_json::to_string(text).unwrap(),
            ),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        if caught {
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{text}: {report:?}"
            );
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(1));
            assert_method_watch(&vm, &artifact, "FLAG", 2, VmValue::Integer(1));
        } else {
            let fault = report.events.iter().find_map(|event| match event {
                VmEvent::FiberFaulted { fault, .. } => Some(fault),
                _ => None,
            });
            assert!(
                matches!(
                    fault.map(|fault| &fault.category),
                    Some(erabasic_vm::FaultCategory::Script(_))
                ),
                "{text}: {report:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
            assert_method_watch(&vm, &artifact, "FLAG", 2, VmValue::Integer(0));
        }
        assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(0));
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn checked_native_failure_restores_checkpoint_and_failed_rollback_is_uncatchable() {
    use std::sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    };
    struct MutatingFailure {
        state: Arc<AtomicU8>,
        fail_restore: bool,
    }
    impl NativeService for MutatingFailure {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            self.state.store(9, Ordering::SeqCst);
            Err(erabasic_vm::ExecutionFailure::script(
                erabasic_vm::ScriptFaultKind::Operation,
                VmFaultCode::Native,
                "script domain failure",
            ))
        }
        fn snapshot(&self) -> Result<Option<Vec<u8>>, String> {
            Ok(Some(vec![self.state.load(Ordering::SeqCst)]))
        }
        fn restore(&mut self, bytes: &[u8]) -> Result<(), String> {
            if self.fail_restore {
                return Err("rollback deliberately unavailable".into());
            }
            let [value] = bytes else {
                return Err("invalid test snapshot".into());
            };
            self.state.store(*value, Ordering::SeqCst);
            Ok(())
        }
    }
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nIF FLAG:7\nRESULT:8 = ABS(FLAG:0)\nENDIF\nRESULT:10 = 73\nRESULT:10 = STRFORMCHECK(\"{ABS(FLAG:0)}\")\nFLAG:9 = 1\nRETURN\n",
        &method_options(true),
    );
    let key = artifact
        .runtime_native_authorizations
        .iter()
        .find(|family| family.name.eq_ignore_ascii_case("abs"))
        .unwrap()
        .key;
    for fail_restore in [false, true] {
        let state = Arc::new(AtomicU8::new(1));
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        natives.register(
            key,
            MutatingFailure {
                state: Arc::clone(&state),
                fail_restore,
            },
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        if fail_restore {
            assert_eq!(
                take_fault(report).category,
                erabasic_vm::FaultCategory::HostContract
            );
            assert_eq!(state.load(Ordering::SeqCst), 9);
            assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(73));
        } else {
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            assert_eq!(state.load(Ordering::SeqCst), 1);
            assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(0));
        }
        assert_method_watch(
            &vm,
            &artifact,
            "FLAG",
            9,
            VmValue::Integer(i64::from(!fail_restore)),
        );
    }
}
