use super::*;
#[test]
fn checked_forms_classify_root_and_nested_source_types_before_execution() {
    let forms = [
        r#"{1 + "x"}"#,
        r#"{-"x"}"#,
        r#"{FLAG:"x"}"#,
        r#"{1,"x"}"#,
        r#"{("x" ? 1 # 2)}"#,
        r"%1%",
        r#"{ABS("x")}"#,
    ];
    for form in forms {
        for nested in [false, true] {
            // A formatted expression nested in an interpolation must receive
            // exactly the same source check as the root template.
            let form = if nested {
                format!("%\\@ 1 ? {form} # unused \\@%")
            } else {
                form.to_owned()
            };
            let form = format!("{{EFFECT()}}{form}");
            let escaped = form.replace('\\', "\\\\").replace('"', "\\\"");
            let source = format!(
                "@SYSTEM_TITLE\nIF FLAG:99\nRESULT:9 = ABS(FLAG:99)\nENDIF\nRESULT:0 = STRFORMCHECK(\"{escaped}\")\nRESULT:1 = STRFORMCHECK(\"{{EFFECT()}}\")\nRETURN\n@ORDINARY\nRESULTS:0 '= STRFORM(\"{escaped}\")\nRETURN\n@EFFECT\n#FUNCTION\nFLAG:0 += 1\nRETURNF 7\n"
            );
            let artifact = compile_source_with_options(&source, &method_options(true));
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
                "{form}: {report:?}"
            );
            assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
            assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(1));
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
            let (vm, report) = run_method_case(&artifact, "ORDINARY", VmConfig::default());
            let fault = take_fault(report);
            assert_eq!(
                fault.category,
                erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Argument),
                "{form}: {fault:?}"
            );
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
        }
    }
}

#[test]
fn checked_forms_distinguish_unknown_names_from_missing_native_providers() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
IF FLAG:99
RESULT:9 = ABS(FLAG:99)
ENDIF
RESULT:0 = STRFORMCHECK("{ABS(-3)}")
RESULT:1 = STRFORMCHECK("{UNKNOWN_PROVIDER_NAME(3)}")
FLAG:0 = 1
RETURN RESULT:0
"#,
        &method_options(true),
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|native| native.import.name.eq_ignore_ascii_case("ABS"))
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::HostContract);
    assert_eq!(fault.code, VmFaultCode::Native);
    assert!(fault.message.to_ascii_uppercase().contains("ABS"));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
}

#[test]
fn existvar_probe_node_budget_is_not_a_catchable_script_failure() {
    let expression = (0..24).fold("1".to_owned(), |inner, _| format!("ABS({inner})"));
    let source = format!(
        "@SYSTEM_TITLE\nRESULT = STRFORMCHECK(\"{{PROBE()}}\")\nFLAG:1 = 1\nRETURN\n@PROBE\n#FUNCTION\nFLAG:0 = 1\nRETURNF EXISTVAR(\"{expression}\", 1)\n"
    );
    let artifact = compile_source_with_options(&source, &method_options(true));
    let (vm, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_operand_stack: 16,
            ..VmConfig::default()
        },
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::ResourceLimit);
    assert_eq!(fault.code, VmFaultCode::ResourceLimit);
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn call_text_restructures_all_outer_actuals_before_any_ordinary_argument() {
    for call in ["TAKE(SIDE(), FLAG:9999999)", "TAKE(7, FLAG:9999999)"] {
        let source = format!(
            "@SYSTEM_TITLE\nTRYCCALLSTR {}\nFLAG:0 = 99\nCATCH\nFLAG:1 = 1\nENDCATCH\nRETURN\n@TAKE(ARG)\nFLAG:2 = 1\nRETURN\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 7\n",
            serde_json::to_string(call).unwrap()
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            matches!(
                take_fault(report).category,
                erabasic_vm::FaultCategory::Script(_)
            ),
            "{call}"
        );
        for index in [0, 1, 2, 8] {
            assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(0));
        }
    }
    // Snake ordinary division by zero reduces to zero and emits a diagnostic;
    // it is not a binder/callee fault even when the outer actual is discarded.
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nTRYCCALLSTR \"TAKE(7, 1 / 0)\"\nFLAG:0 = 99\nCATCH\nFLAG:1 = 1\nENDCATCH\nRETURN\n@TAKE(ARG)\nFLAG:2 = ARG\nRETURN\n",
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_eq!(report.events.iter().filter(|event| matches!(event, VmEvent::Diagnostic { code, .. } if code == "compat.arithmetic.divide_by_zero")).count(), 1);
    for (index, value) in [(0, 99), (1, 0), (2, 7)] {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
    }
}

#[test]
fn call_text_unique_restructure_reads_outer_excess_but_not_converted_nested_excess() {
    for (call, side_reads) in [
        (r#"TAKE(7, REPLACE("a", "a", "b", SIDE()))"#, 1),
        (r#"TAKE(DROP(7, REPLACE("a", "a", "b", SIDE())))"#, 0),
        (r#"TAKES(REPLACE("a", "a", "b", SIDE()))"#, 2),
        ("TAKES(STRFORM(ITEMNAME:SIDE()))", 2),
    ] {
        let source = format!(
            "@SYSTEM_TITLE\nCALLSTR {}\nFLAG:9 = 1\nRETURN\n@TAKE(ARG)\nRETURN\n@TAKES(ARGS)\nRETURN\n@DROP(ARG)\n#FUNCTION\nRETURNF ARG\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n",
            serde_json::to_string(call).unwrap()
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{call}: {report:?}"
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{call}: {report:?}"
        );
        assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(side_reads));
        assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(1));
    }
}

#[test]
fn call_text_retained_constant_bounds_precede_later_index_restructure_and_are_not_catchable() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM GRID, 2, 2
TRYCCALLSTR "TAKE(GRID:999:STRLEN(REPLACE(\"a\", \"a\", \"b\", SIDE())))"
FLAG:0 = 99
CATCH
FLAG:1 = 1
ENDCATCH
RETURN
@TAKE(ARG)
RETURN
@SIDE
#FUNCTION
FLAG:8 += 1
RETURNF 0
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(matches!(
        fault.category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Bounds)
    ));
    assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn call_text_nested_ref_restructure_keeps_variable_root_and_child_effects() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#LOCALSIZE 2
LOCAL:0 = 5
CALLSTR "TAKE(USE(LOCAL:STRLEN(REPLACE(\"a\", \"a\", \"b\", SIDE()))))"
RESULT:10 = LOCAL:0
RETURN
@TAKE(ARG)
RESULT:11 = ARG
RETURN
@USE(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 = 9
RETURNF ITEMS:0
@SIDE
#FUNCTION
FLAG:8 += 1
RETURNF 0
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
    assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(1));
    for index in [10, 11] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(9));
    }
}

pub(in super::super) fn reject_form_snapshot_before_native_restore(
    artifact: &BytecodeArtifact,
    snapshot: VmSnapshot,
    attack: &str,
) {
    let (mut rejected_natives, restore_count) = lease_snapshot_natives(artifact);
    let mut rejected_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    assert!(
        Vm::restore_snapshot(
            validated(artifact),
            VmConfig::default(),
            snapshot,
            &mut rejected_host,
            &mut rejected_natives
        )
        .is_err(),
        "{attack}"
    );
    assert_eq!(
        restore_count.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "{attack}"
    );
    assert!(rejected_host.rebound.is_empty(), "{attack}");
}

pub(in super::super) fn corrupt_native_form_snapshot(
    value: &mut serde_json::Value,
    fiber: FiberId,
    attack: &str,
) {
    let work = value["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["work"]
        .as_array_mut()
        .unwrap();
    let native = work
        .iter_mut()
        .find_map(|task| task.get_mut("FinishNative"))
        .unwrap();
    match attack {
        "family_is_physical" => {
            native["bound"]["service_key"] = native["bound"]["import"]["key"].clone();
        }
        "wrong_parameter" => {
            native["bound"]["import"]["parameters"][0] =
                serde_json::to_value(BytecodeType::String).unwrap();
        }
        "forged_source" => native["source"][0] = serde_json::Value::Null,
        "forged_omission" => native["bound"]["omitted_arguments"] = serde_json::json!([0]),
        _ => {}
    }
    let plans =
        &mut value["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["call_plans"];
    match attack {
        "plan_missing" => *plans = serde_json::json!([]),
        "plan_binding" => {
            plans[0]["calls"][0][1]["Native"]["import"]["parameters"][0] =
                serde_json::to_value(BytecodeType::String).unwrap();
        }
        "plan_types" => {
            plans[0]["types"][0][1] = serde_json::to_value(BytecodeType::String).unwrap();
        }
        "plan_source" => plans[0]["source"] = serde_json::json!({"Arguments": []}),
        _ => {}
    }
}

const CALL_TEXT_RESTRUCTURE_WAIT_ERB: &str = r#"@SYSTEM_TITLE
CALLSTR "TAKE(7, STRLEN(REPLACE(\"a\", \"a\", \"b\", WAITMODE())))"
FLAG:9 = 1
RETURN
@TAKE(ARG)
FLAG:0 = ARG
RETURN
@WAITMODE
#FUNCTION
FLAG:8 += 1
INPUT
RETURNF 0
@IMPORTS
#FUNCTION
RESULTS '= REPLACE(ARGS, ARGS, ARGS, ARG)
RESULT = STRLEN(ARGS)
RETURNF ABS(ARG)
"#;

#[test]
fn call_text_restructure_wait_snapshot_resumes_once_and_rejects_graph_corruption_before_native_restore()
 {
    use std::sync::atomic::Ordering;
    let artifact =
        compile_source_with_options(CALL_TEXT_RESTRUCTURE_WAIT_ERB, &method_options(true));
    let (mut natives, _) = lease_snapshot_natives(&artifact);
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
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("expected unique-method restructuring wait");
    };
    let snapshot = vm.snapshot(&natives).unwrap();
    let saved = serde_json::to_value(&snapshot).unwrap();
    let pending =
        &saved["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["reference_arguments"];
    assert_eq!(pending["preparing"], true);
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 8, VmValue::Integer(1));
    for attack in ["delete_graph", "bad_root", "bad_progress"] {
        let mut corrupted = saved.clone();
        let pending = &mut corrupted["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["reference_arguments"];
        match attack {
            "delete_graph" => *pending = serde_json::Value::Null,
            "bad_root" => pending["graph"]["template"]["roots"][0] = serde_json::json!(u32::MAX),
            "bad_progress" => {
                let task = pending["tasks"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|task| task.get("CaptureChild").is_some())
                    .unwrap();
                task["CaptureChild"]["next"] = serde_json::json!(usize::MAX);
            }
            _ => unreachable!(),
        }
        let snapshot: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        reject_form_snapshot_before_native_restore(&artifact, snapshot, attack);
    }
    let (mut restored_natives, restore_count) = lease_snapshot_natives(&artifact);
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut restored_natives,
    )
    .unwrap();
    assert_eq!(restore_count.load(Ordering::SeqCst), 1);
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut restored_natives,
        RunBudget::default(),
    );
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
    for (index, expected) in [(0, 7), (8, 1), (9, 1)] {
        assert_method_watch(
            &restored,
            &artifact,
            "FLAG",
            index,
            VmValue::Integer(expected),
        );
    }
}
