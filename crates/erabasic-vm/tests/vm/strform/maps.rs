use super::*;
#[test]
fn map_staged_capture_precedes_recreate_in_bytecode_and_runtime_form() {
    for dynamic in [false, true] {
        let expression = "MAP_TOSTRING(\"m\", RECREATE_MAP())";
        let expression = if dynamic {
            format!(
                "STRFORM({})",
                serde_json::to_string(&format!("%{expression}%")).unwrap()
            )
        } else {
            expression.into()
        };
        let source = format!(
            r#"@SYSTEM_TITLE
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "a", "1")
RESULT = MAP_SET("m", "b", "2")
RESULTS:10 '= {expression}
RESULTS:11 '= MAP_TOSTRING("m")
RETURN RESULT
@RECREATE_MAP
#FUNCTIONS
FLAG:0 += 1
RESULT = MAP_RELEASE("m")
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "fresh", "new")
RETURNF "|"
"#
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{dynamic}: {report:?}"
        );
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            10,
            VmValue::String("a=1|b=2".into()),
        );
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            11,
            VmValue::String("fresh=new".into()),
        );
        assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    }
}

#[test]
fn map_missing_name_and_disabled_output_skip_tail_expressions() {
    for dynamic in [false, true] {
        let expressions = [
            "MAP_TOSTRING(\"missing\", MAP_TAIL())",
            "MAP_VALUES(\"m\", OUT:MAP_INDEX(), 0)",
        ];
        let body = expressions
            .iter()
            .enumerate()
            .map(|(index, expression)| {
                let expression = if dynamic {
                    format!(
                        "STRFORM({})",
                        serde_json::to_string(&format!("%{expression}%")).unwrap()
                    )
                } else {
                    (*expression).into()
                };
                format!("RESULTS:{} '= {expression}\n", index + 10)
            })
            .collect::<Vec<_>>()
            .concat();
        let source = format!(
            r#"@SYSTEM_TITLE
#DIMS OUT, 2
RESULT = MAP_CREATE("m")
{body}RETURN RESULT
@MAP_TAIL
#FUNCTIONS
FLAG:0 += 1
RETURNF ","
@MAP_INDEX
#FUNCTION
FLAG:1 += 1
RETURNF 999999
"#
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{dynamic}: {report:?}"
        );
        for index in [0, 1] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(0));
        }
        for index in [10, 11] {
            assert_method_watch(
                &vm,
                &artifact,
                "RESULTS",
                index,
                VmValue::String(String::new()),
            );
        }
    }
}

#[test]
fn original_profile_rejects_all_six_map_extensions() {
    for expression in [
        "MAP_VALUES(\"m\")",
        "MAP_MERGE(\"m\", \"n\")",
        "MAP_REMOVEIF(\"m\", \"x\", \"KEY_PREFIX\")",
        "MAP_FINDKEY(\"m\", \"x\", \"KEY_PREFIX\")",
        "MAP_TOSTRING(\"m\")",
        "MAP_FROMSTRING(\"m\", \"a=1\")",
    ] {
        let source = format!("@SYSTEM_TITLE\nIF {expression} == {expression}\nENDIF\nRETURN\n");
        let report = analyze_project(
            AnalysisInput {
                project_data: project_data(),
                sources: vec![ProjectSource {
                    relative_path: "map-profile.erb".into(),
                    payload: SourcePayload::Utf8(source),
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
            "{expression}: {report:?}"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Corruptions and the matching valid restore share one fixture.
fn map_pending_bytecode_capture_snapshot_rejects_missing_or_forged_lease_owner() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "a", "old")
RESULT:9 = RAND:1000000
RESULTS:10 '= MAP_TOSTRING("m", MAP_WAIT())
RETURN RESULT
@MAP_WAIT
#FUNCTIONS
RESULT = MAP_RELEASE("m")
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "a", "new")
INPUT
RETURNF "|"
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
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    for attack in ["drop", "frame", "generation", "begin"] {
        let mut corrupted = saved.clone();
        let frame = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0];
        match attack {
            "drop" => frame["map_calls"] = serde_json::json!([]),
            "frame" => {
                frame["map_calls"][0]["lease"]["owner"]["frame"] = serde_json::json!(999_999);
            }
            "generation" => {
                frame["map_calls"][0]["lease"]["owner"]["generation"] = serde_json::json!(999_999);
            }
            "begin" => frame["map_calls"][0]["begin"] = serde_json::json!(999_999),
            _ => unreachable!(),
        }
        let mut rejected_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        // Use a no-active-call VM to snapshot the independent registry; the saved live VM
        // must not be paired with a registry which intentionally has none of its leases.
        let control = Vm::new(validated(&artifact), VmConfig::default());
        let before = control
            .encode_unrestricted_snapshot(&rejected_natives)
            .unwrap();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                serde_json::from_value(corrupted).unwrap(),
                &mut rejected_host,
                &mut rejected_natives
            )
            .is_err(),
            "{attack}"
        );
        assert!(rejected_host.rebound.is_empty());
        assert_eq!(
            control
                .encode_unrestricted_snapshot(&rejected_natives)
                .unwrap(),
            before
        );
    }
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("MAP argument did not wait");
    };
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(saved).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::FiberCompleted {
                fiber: completed,
                value: Some(VmValue::Integer(1))
            } if *completed == fiber
        )),
        "{report:#?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:#?}"
    );
    assert_method_watch(
        &restored,
        &artifact,
        "RESULTS",
        10,
        VmValue::String("a=old".into()),
    );
}

#[test]
fn map_values_character_getarray_failure_is_delayed_until_enabled() {
    for enabled in [0, 1] {
        let artifact = compile_source_with_options(
            &format!(
                r#"@SYSTEM_TITLE
RESULT = MAP_CREATE("m")
RESULTS:10 '= MAP_VALUES("m", CSTR:MAP_CHAR_INDEX(), {enabled})
FLAG:9 = 1
RETURN RESULT
@MAP_CHAR_INDEX
#FUNCTION
FLAG:0 += 1
RETURNF 999999
"#
            ),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
        if enabled == 0 {
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{report:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(1));
        } else {
            assert_eq!(
                take_fault(report).category,
                erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Operation)
            );
            assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(0));
        }
    }
}

#[test]
fn whole_character_ref_captures_before_later_actual_and_preserves_profile_disposal() {
    for snake in [false, true] {
        for mode in ["static", "method", "form"] {
            let invocation = match mode {
                "static" => {
                    "CALL CHANGE_ARRAY, CFLAG:SELECT_CHAR():SKIPPED_INDEX(), DELETE_SELECTED()\nRESULT:0 = FLAG:2"
                }
                "method" => {
                    "RESULT:0 = CHANGE_METHOD(CFLAG:SELECT_CHAR():SKIPPED_INDEX(), DELETE_SELECTED())"
                }
                _ => {
                    "RESULTS:0 '= STRFORM(\"{CHANGE_METHOD(CFLAG:SELECT_CHAR():SKIPPED_INDEX(), DELETE_SELECTED())}\")"
                }
            };
            let source = format!(
                r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
CFLAG:0:0 = 17
CFLAG:1:0 = 99
{invocation}
RESULT:1 = FLAG:0
RESULT:2 = CFLAG:0:0
RETURN RESULT
@SELECT_CHAR
#FUNCTION
FLAG:0 += 1
RETURNF 0
@SKIPPED_INDEX
#FUNCTION
FLAG:0 += 100
RETURNF 0
@DELETE_SELECTED
#FUNCTION
DELCHARA 0
RETURNF 0
@CHANGE_ARRAY(VALUES, DUMMY)
#DIM REF VALUES
#DIM DUMMY
VALUES:0 += 3
FLAG:2 = VALUES:0
RETURN FLAG:2
@CHANGE_METHOD(VALUES, DUMMY)
#FUNCTION
#DIM REF VALUES
#DIM DUMMY
VALUES:0 += 3
RETURNF VALUES:0
"
            );
            let artifact = compile_source_with_options(&source, &method_options(snake));
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{snake}/{mode}: {report:?}"
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{snake}/{mode}: {report:?}"
            );
            let expected = if snake { 3 } else { 20 };
            if mode == "form" {
                assert_method_watch(
                    &vm,
                    &artifact,
                    "RESULTS",
                    0,
                    VmValue::String(expected.to_string()),
                );
            } else {
                assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(expected));
            }
            assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(1));
            assert_method_watch(&vm, &artifact, "RESULT", 2, VmValue::Integer(99));
        }
    }
}
