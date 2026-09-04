use super::*;
#[test]
fn dynamic_native_text_only_scalar_defaults_omissions_and_variadic_calls() {
    let template = r#"{ABS(-7)}|%REPLACE("aba", "a", "x")%|%SUBSTRING("abc")%|%SUBSTRING("abc",,2)%|{MAX(1,9,2,3,4)}|%STRFORM("literal")%"#;
    let source = format!(
        "@SYSTEM_TITLE\nRESULTS:10 '= STRFORM({})\nRETURN\n",
        serde_json::to_string(template).unwrap()
    );
    let artifact = compile_source_with_options(&source, &method_options(true));
    for name in ["abs", "replace", "substring", "max"] {
        assert!(
            !artifact
                .native_imports
                .iter()
                .any(|entry| entry.import.name == name),
            "{name} must occur only inside dynamic text"
        );
        assert!(
            artifact
                .runtime_native_authorizations
                .iter()
                .any(|family| family.name == name)
        );
    }
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(
        &vm,
        &artifact,
        "RESULTS",
        10,
        VmValue::String("7|xbx|abc|ab|9|literal".into()),
    );
}

#[test]
fn dynamic_native_callstr_array_token_uses_authoritative_variable_place() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC_ITEMS, 3
DYNAMIC_ITEMS:0 = 4
DYNAMIC_ITEMS:1 = 5
DYNAMIC_ITEMS:2 = 6
CALLSTR "TAKE(SUMARRAY(DYNAMIC_ITEMS))"
RETURN
@TAKE(ARG)
RESULT:10 = ARG
RETURN
"#,
        &method_options(true),
    );
    assert!(
        !artifact
            .native_imports
            .iter()
            .any(|entry| entry.import.name == "sumarray")
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(15));
}

#[test]
fn dynamic_native_host_registry_denial_and_missing_provider_are_distinct() {
    let source = "@SYSTEM_TITLE\nRESULT = STRFORMCHECK(\"{ABS(-7)}\")\nFLAG:0 = 1\nRETURN\n";
    let analysis = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &method_options(true),
        &ExtensionRegistry::default(),
    );
    let project = analysis.project.unwrap();
    let mut registry = default_host_registry();
    registry.register_execution(
        "ABS",
        erabasic_compiler::ExecutionBinding::Unsupported {
            reason: "test denies dynamic ABS".into(),
        },
    );
    let denied = compile_project(&project, &CompilerOptions::default(), &registry, None)
        .artifact
        .unwrap();
    assert!(
        denied
            .runtime_builtins
            .iter()
            .any(|symbol| symbol.name == "ABS")
    );
    assert!(
        !denied
            .runtime_native_authorizations
            .iter()
            .any(|family| family.name == "abs")
    );
    let (vm, report) = run_entry(&denied, VmConfig::default());
    assert_eq!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::Permission
    );
    assert_method_watch(&vm, &denied, "FLAG", 0, VmValue::Integer(0));

    let granted = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .unwrap();
    let entry = granted
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&granted), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    assert_eq!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::HostContract
    );
    assert_method_watch(&vm, &granted, "FLAG", 0, VmValue::Integer(0));
}

#[test]
fn checked_method_callstr_omitted_replace_mode_preserves_prior_side_effects() {
    // Fixed Creator.Method.cs REPLACE unique restructure dereferences omitted mode;
    // outer STRFORMCHECK catches this source-proved script error, not arbitrary CLR failures.
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = STRFORMCHECK("{PROBE()}")
FLAG:9 = 1
RETURN
@PROBE
#FUNCTION
FLAG:8 += 1
CALLSTR "TAKES(REPLACE(\"a\", \"a\", \"b\",,))"
FLAG:7 = 1
RETURNF 1
@TAKES(ARGS)
FLAG:6 = 1
RETURN
"#,
        &method_options(true),
    );
    assert!(
        !artifact
            .native_imports
            .iter()
            .any(|entry| entry.import.name == "replace")
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for (name, index, expected) in [
        ("RESULT", 10, 0),
        ("FLAG", 8, 1),
        ("FLAG", 9, 1),
        ("FLAG", 7, 0),
        ("FLAG", 6, 0),
    ] {
        assert_method_watch(&vm, &artifact, name, index, VmValue::Integer(expected));
    }
}

#[test]
fn dynamic_native_and_static_rand_share_one_stream() {
    let dynamic = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = RAND(1000000)
RESULT:11 = TOINT(STRFORM("{RAND(1000000)}"))
RETURN
"#,
        &method_options(true),
    );
    let ordinary = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = RAND(1000000)\nRESULT:11 = RAND(1000000)\nRETURN\n",
        &method_options(true),
    );
    let (actual, report) = run_entry(&dynamic, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    let (expected, report) = run_entry(&ordinary, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    let key = ordinary
        .globals
        .iter()
        .find(|variable| variable.name == "RESULT")
        .unwrap()
        .key;
    for index in [10, 11] {
        let value = expected.read_variable(key, &[index], None).unwrap();
        assert_method_watch(&actual, &dynamic, "RESULT", index, value);
    }
}

#[test]
fn dynamic_native_wait_snapshot_rejects_forged_family_before_restore() {
    use std::sync::atomic::Ordering;
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULTS:10 '= STRFORM("{ABS(WAITVALUE())}")
RETURN
@WAITVALUE
#FUNCTION
FLAG:8 += 1
INPUT
RETURNF -7
@SNAPSHOT_COUNTER_IMPORT
#FUNCTION
RETURNF ABS(ARG)
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let (mut natives, _) = lease_snapshot_natives(&artifact);
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("Native argument must suspend in user method INPUT");
    };
    let snapshot = vm.snapshot(&natives).unwrap();
    let saved = serde_json::to_value(&snapshot).unwrap();
    for attack in [
        "family_is_physical",
        "wrong_parameter",
        "forged_source",
        "forged_omission",
        "plan_missing",
        "plan_binding",
        "plan_types",
        "plan_source",
    ] {
        let mut value = saved.clone();
        corrupt_native_form_snapshot(&mut value, fiber, attack);
        let corrupt: VmSnapshot = serde_json::from_value(value).unwrap();
        reject_form_snapshot_before_native_restore(&artifact, corrupt, attack);
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
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(
        &restored,
        &artifact,
        "RESULTS",
        10,
        VmValue::String("7".into()),
    );
    assert_method_watch(&restored, &artifact, "FLAG", 8, VmValue::Integer(1));
}

#[test]
fn dynamic_native_bad_source_arity_is_catchable_without_loosening_static_arity() {
    for expression in [
        r#"ABS("x")"#,
        "ABS(1,2)",
        "LIMIT(1)",
        "POWER(1)",
        "GETBIT(1)",
        "STRLEN(7)",
        "SUBSTRING(,1)",
        "MAX(1,,)",
    ] {
        let template = format!("{{{expression}}}");
        let source = format!(
            "@SYSTEM_TITLE\nRESULT:10 = STRFORMCHECK({})\nFLAG:9 = 1\nRETURN\n",
            serde_json::to_string(&template).unwrap()
        );
        let artifact = compile_source_with_options(&source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(0));
        assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(1));
    }
    let static_report = analyze_project(
        AnalysisInput {
            project_data: project_data(),
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8("@SYSTEM_TITLE\nRESULT = ABS(1,2)\nRETURN\n".into()),
            }],
        },
        &method_options(true),
        &ExtensionRegistry::default(),
    );
    assert!(
        static_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2)
    );
}

#[test]
fn dynamic_core_omission_is_not_a_literal_minimum_integer() {
    let text = r#"{STRFIND("abc","a",,)}|{STRFIND("abc","a",(-9223372036854775807 - 1))}|{ENCODETOUNI("ab",,)}"#;
    let source = format!(
        "@SYSTEM_TITLE\nRESULTS:10 '= STRFORM({})\nRETURN\n",
        serde_json::to_string(text).unwrap()
    );
    let artifact = compile_source_with_options(&source, &method_options(true));
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_method_watch(
        &vm,
        &artifact,
        "RESULTS",
        10,
        VmValue::String("0|-1|97".into()),
    );
}

#[test]
fn original_dynamic_form_cannot_acquire_snake_checker_from_forged_artifact_tables() {
    let mut original = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULTS '= STRFORM(\"{STRFORMCHECK(\\\"literal\\\")}\")\nRETURN\n",
        &method_options(false),
    );
    let (_, denied) = run_entry(&original, VmConfig::default());
    assert_eq!(
        take_fault(denied).category,
        erabasic_vm::FaultCategory::Permission
    );
    let snake = compile_source_with_options("@SYSTEM_TITLE\nRETURN\n", &method_options(true));
    let family = snake
        .runtime_native_authorizations
        .iter()
        .find(|family| family.name == "strformcheck")
        .unwrap()
        .clone();
    let symbol = snake
        .runtime_builtins
        .iter()
        .find(|symbol| symbol.name == "STRFORMCHECK")
        .unwrap()
        .clone();
    original.runtime_builtins.push(symbol);
    original.runtime_native_authorizations.push(family);
    original.refresh_ids().unwrap();
    let context =
        erabasic_compiler::runtime_native_validation_context(&original, &default_host_registry());
    let report = validate_bytecode(original.into_unvalidated(), &context);
    assert!(report.value.is_none());
    assert!(
        report.diagnostics.iter().any(
            |diagnostic| diagnostic.code == erabasic_validator::ValidationCode::HostAbiMismatch
        )
    );
}

#[test]
fn dynamic_strjoin_omissions_preserve_slots_and_literal_minimum_is_not_omitted() {
    for snake in [false, true] {
        let artifact = compile_source_with_options(
            r#"@SYSTEM_TITLE
#DIMS ITEMS, 3
ITEMS:0 '= "a"
ITEMS:1 '= "b"
ITEMS:2 '= "c"
RESULTS:10 '= STRFORM("%STRJOIN(ITEMS,,1,2)%")
RESULTS:11 '= STRFORM("%STRJOIN(ITEMS,,,1)%")
RESULTS:12 '= STRFORM("%STRJOIN(ITEMS,,,)%")
RETURN
"#,
            &method_options(snake),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for (slot, value) in [(10, "b,c"), (11, "a"), (12, "a,b,c")] {
            assert_method_watch(
                &vm,
                &artifact,
                "RESULTS",
                slot,
                VmValue::String(value.into()),
            );
        }
    }
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIMS ITEMS, 3
ITEMS:0 '= "a"
RESULT:10 = STRFORMCHECK("%STRJOIN(ITEMS,,(-9223372036854775807 - 1),1)%")
RESULT:11 = STRFORMCHECK("%STRJOIN(ITEMS,,,(-9223372036854775807 - 1))%")
FLAG:9 = 1
RETURN
"#,
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
    for slot in [10, 11] {
        assert_method_watch(&vm, &artifact, "RESULT", slot, VmValue::Integer(0));
    }
    assert_method_watch(&vm, &artifact, "FLAG", 9, VmValue::Integer(1));
}

#[test]
fn dynamic_strjoin_omitted_delimiter_uses_callers_array_ref() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIMS ITEMS, 3
ITEMS:0 '= "a"
ITEMS:1 '= "b"
ITEMS:2 '= "c"
CALLSTR "TAKE(JOINREF(ITEMS))"
RETURN
@TAKE(ARGS)
RESULTS:10 '= ARGS
RETURN
@JOINREF(VALUES)
#FUNCTIONS
#DIMS REF VALUES
RETURNF STRFORM("%STRJOIN(VALUES,,1,1)%")
"#,
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
    assert_method_watch(&vm, &artifact, "RESULTS", 10, VmValue::String("b".into()));
}
