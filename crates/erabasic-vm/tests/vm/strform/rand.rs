use super::*;

fn warnings(report: &erabasic_vm::VmRunReport) -> Vec<&str> {
    report
        .events
        .iter()
        .filter_map(|event| match event {
            VmEvent::Diagnostic {
                code,
                notification,
                origin,
                ..
            } if code.starts_with("compat.rand.") => {
                assert_eq!(
                    *notification,
                    erabasic_vm::VmDiagnosticNotification::LogOnly
                );
                assert!(origin.source.is_some());
                Some(code.as_str())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn rand_invalid_ranges_share_static_and_dynamic_policy_without_consuming_rng() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
FLAG:0 = -7
FLAG:1 = 0
DUMPRAND
RESULT:19 = RANDDATA:624
RESULT:10 = RAND:(FLAG:0)
RESULT:11 = RAND:(FLAG:1)
RESULT:12 = RAND(5, -2)
RESULT:13 = RAND(0)
RESULT:14 = TOINT(STRFORM("{RAND:(FLAG:0)}"))
RESULT:15 = TOINT(STRFORM("{RAND(5, -2)}"))
RESULT:16 = RAND(0, -1)
RESULT:17 = RAND(__INT_MIN__, __INT_MIN__)
RESULT:18 = EXISTVAR("RAND:(FLAG:0)", 1)
DUMPRAND
RESULT:20 = RANDDATA:624
RETURN
"#,
        &method_options(true),
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|item| item.import.name == "__rand_variable")
    );
    assert!(
        !artifact
            .runtime_builtins
            .iter()
            .any(|item| item.name == "__RAND_VARIABLE")
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for (index, expected) in [
        (10, 0),
        (11, 0),
        (12, 5),
        (13, 0),
        (14, 0),
        (15, 5),
        (16, 0),
        (17, i64::MIN),
        (18, 1),
    ] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
    let result = named_key(&artifact, "RESULT");
    assert_eq!(
        vm.read_variable(result, &[19], None),
        vm.read_variable(result, &[20], None)
    );
    assert_eq!(
        warnings(&report),
        ["compat.rand.variable_range", "compat.rand.function_range"]
    );
}

#[test]
fn rand_dynamic_variable_preserves_signed_inputs_and_uses_the_same_valid_stream() {
    for snake in [false, true] {
        let dynamic = compile_source_with_options(
            r#"@SYSTEM_TITLE
RESULT:10 = TOINT(STRFORM("{RAND:1000000}"))
RESULT:11 = RAND(1000000)
RESULT:12 = RAND(__INT_MIN__, __INT_MIN__ + 1)
RETURN
"#,
            &method_options(snake),
        );
        let direct = compile_source_with_options(
            "@SYSTEM_TITLE\nRESULT:10 = RAND(1000000)\nRESULT:11 = RAND(1000000)\nRETURN\n",
            &method_options(snake),
        );
        let (actual, report) = run_entry(&dynamic, VmConfig::default());
        let (expected, _) = run_entry(&direct, VmConfig::default());
        assert!(
            report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
            "{report:?}"
        );
        for index in [10, 11] {
            assert_eq!(
                actual.read_variable(named_key(&dynamic, "RESULT"), &[index], None),
                expected.read_variable(named_key(&direct, "RESULT"), &[index], None)
            );
        }
        assert_method_watch(&actual, &dynamic, "RESULT", 12, VmValue::Integer(i64::MIN));
        assert!(warnings(&report).is_empty());
    }
}

#[test]
fn rand_original_and_compati_ranges_keep_the_existing_failure_without_warning() {
    for (snake, compati) in [(false, false), (true, true)] {
        for expression in ["RAND:(FLAG:0)", "RAND(0)", "RAND(5, -2)"] {
            let mut options = method_options(snake);
            options.compatible_rand = compati;
            let artifact = compile_source_with_options(
                &format!("@SYSTEM_TITLE\nFLAG:0 = -1\nRESULT = {expression}\nRETURN\n"),
                &options,
            );
            let (_, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{snake}/{compati}/{expression}: {report:?}"
            );
            assert!(warnings(&report).is_empty());
        }
    }
}

#[test]
fn rand_parse_zero_and_omission_stay_distinct_from_runtime_zero() {
    for expression in ["RAND", "RAND:0", "RAND:(+0)", "RAND:FLAG:0"] {
        let analysis = analyze_project(
            AnalysisInput {
                project_data: project_data(),
                sources: vec![ProjectSource {
                    relative_path: "rand.erb".into(),
                    payload: SourcePayload::Utf8(format!("@SYSTEM_TITLE\nRETURN {expression}\n")),
                }],
            },
            &method_options(true),
            &ExtensionRegistry::default(),
        );
        assert!(analysis.diagnostics.iter().any(|item| item.code == erabasic_analyzer::AnalyzerDiagnosticCode::InvalidArgument), "{expression}: {:?}", analysis.diagnostics);
    }
    for expression in ["RAND:(-0)", "RAND:(0 + 0)"] {
        let artifact = compile_source_with_options(
            &format!("@SYSTEM_TITLE\nRESULT:10 = {expression}\nRETURN\n"),
            &method_options(true),
        );
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(0));
        assert_eq!(warnings(&report), ["compat.rand.variable_range"]);
    }
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = STRFORMCHECK("{RAND:0}")
RESULT:11 = STRFORMCHECK("{RAND}")
RESULT:12 = EXISTVAR("RAND:(0 + 0)", 1)
RESULT:13 = STRFORMCHECK("{RAND:FLAG:0}")
RETURN
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_method_watch(&vm, &artifact, "RESULT", 10, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "RESULT", 11, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "RESULT", 12, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "RESULT", 13, VmValue::Integer(0));
    assert!(warnings(&report).is_empty());
}

#[test]
fn rand_warning_session_survives_snapshot_and_fibers_but_resets_in_a_new_vm() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
FLAG:0 = -1
RESULT:10 = RAND:(FLAG:0)
RESULT:11 = RAND(0)
WAIT
RESULT:12 = TOINT(STRFORM("{RAND:(FLAG:0)}"))
RESULT:13 = RAND(3, 2)
RETURN
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|item| item.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert_eq!(warnings(&report).len(), 2);
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["rand_warning_mask"], 3);
    assert_eq!(json["format_version"], 21);
    for mask in [4, 255] {
        let mut invalid = json.clone();
        invalid["rand_warning_mask"] = mask.into();
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let mut rejected_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        let rejected = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            serde_json::from_value(invalid).unwrap(),
            &mut rejected_host,
            &mut rejected_natives,
        );
        assert!(
            matches!(rejected, Err(VmError::Snapshot(message)) if message.contains("RAND diagnostic mask"))
        );
        assert!(rejected_host.rebound.is_empty());
    }
    let mut restored_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut restored_natives,
    )
    .unwrap();
    let Some(FiberStatus::WaitingHost(request)) = restored.fiber_status(fiber) else {
        panic!("expected stable wait")
    };
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(&mut host, &mut restored_natives, RunBudget::default());
    assert!(warnings(&report).is_empty());
    restored.spawn_entry(entry, Vec::new()).unwrap();
    let report = restored.run_slice(&mut host, &mut restored_natives, RunBudget::default());
    assert!(warnings(&report).is_empty());
    let mut fresh = Vm::new(validated(&artifact), VmConfig::default());
    fresh.spawn_entry(entry, Vec::new()).unwrap();
    let report = fresh.run_slice(&mut host, &mut restored_natives, RunBudget::default());
    assert_eq!(warnings(&report).len(), 2);
}

#[test]
fn rand_short_circuit_and_probe_do_not_notify_and_private_entry_is_not_source_callable() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = 0 && RAND(0)
RESULT:11 = TOINT(STRFORM("{1 || RAND:(FLAG:0)}"))
RESULT:12 = EXISTVAR("RAND:(FLAG:0)", 1)
RESULT:13 = STRFORMCHECK("{__RAND_VARIABLE(0)}")
RETURN
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    for (index, expected) in [(10, 0), (11, 1), (12, 1), (13, 0)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
    assert!(warnings(&report).is_empty());
}

#[test]
fn rand_hot_reload_retains_the_session_warning_mask() {
    let base = compile_source_with_options(
        "@SYSTEM_TITLE\nFLAG:0 = -1\nRESULT:10 = RAND:(FLAG:0)\nRESULT:11 = RAND(0)\nRETURN\n",
        &method_options(true),
    );
    let target = compile_source_with_options(
        "@SYSTEM_TITLE\nFLAG:0 = -2\nRESULT:10 = RAND:(FLAG:0)\nRESULT:11 = RAND(5, 2)\nRETURN\n",
        &method_options(true),
    );
    let entry = base
        .functions
        .iter()
        .find(|item| item.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&base, 1234);
    let mut host = ReadyHost::default();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let first = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert_eq!(warnings(&first).len(), 2);
    vm.prepare_hot_reload_artifact(validated(&target)).unwrap();
    vm.commit_hot_reload().unwrap();
    // The target adds a two-operand physical RAND import. Registration changes
    // do not create a new VM session or reset its diagnostic mask.
    natives = NativeServiceRegistry::for_artifact_with_seed(&target, 1234);
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let next = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        next.events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{next:?}"
    );
    assert!(warnings(&next).is_empty());
    assert_method_watch(&vm, &target, "RESULT", 11, VmValue::Integer(5));
}

#[test]
fn rand_dynamic_variable_can_snapshot_while_its_signed_argument_waits() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = TOINT(STRFORM("{RAND:WAIT_ZERO()}"))
RETURN
@WAIT_ZERO
#FUNCTION
INPUT
RETURNF 0
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|item| item.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(warnings(&report).is_empty());
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("{report:?}")
    };
    let snapshot = vm.snapshot(&natives).unwrap();
    let saved = serde_json::to_value(&snapshot).unwrap();
    for attack in ["source", "site", "bound"] {
        let mut invalid = saved.clone();
        let work = invalid["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["work"]
            .as_array_mut()
            .unwrap();
        let native = work
            .iter_mut()
            .find_map(|task| task.get_mut("FinishNative"))
            .expect("RAND must remain staged while its user argument waits");
        match attack {
            "source" => native["source"][0] = serde_json::Value::Null,
            "site" => native["site"]["plan"] = u64::MAX.into(),
            "bound" => {
                native["bound"]["import"]["parameters"][0] =
                    serde_json::to_value(BytecodeType::String).unwrap();
            }
            _ => unreachable!(),
        }
        reject_rand_snapshot_without_side_effects(&artifact, invalid, attack);
    }
    let mut old_format = saved;
    old_format["format_version"] = 20.into();
    reject_rand_snapshot_without_side_effects(&artifact, old_format, "format 20");
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let report = restored.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert_eq!(warnings(&report), ["compat.rand.variable_range"]);
    assert_method_watch(&restored, &artifact, "RESULT", 10, VmValue::Integer(0));
}

fn reject_rand_snapshot_without_side_effects(
    artifact: &BytecodeArtifact,
    invalid: serde_json::Value,
    attack: &str,
) {
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(artifact, 9876);
    // A separate idle VM observes the complete native checkpoints, including SFMT,
    // before and after the failed restore without advancing the native services.
    let observer = Vm::new(validated(artifact), VmConfig::default());
    let before = serde_json::to_value(observer.snapshot(&natives).unwrap()).unwrap();
    assert!(
        before["native_states"]
            .as_array()
            .is_some_and(|states| !states.is_empty())
    );
    assert_ne!(
        before["native_states"], invalid["native_states"],
        "distinct restore seed: {attack}"
    );
    let result = Vm::restore_snapshot(
        validated(artifact),
        VmConfig::default(),
        serde_json::from_value(invalid).unwrap(),
        &mut host,
        &mut natives,
    );
    assert!(matches!(result, Err(VmError::Snapshot(_))), "{attack}");
    assert!(
        host.rebound.is_empty(),
        "{attack}: rejected restore rebound Host"
    );
    let after = serde_json::to_value(observer.snapshot(&natives).unwrap()).unwrap();
    assert_eq!(
        before["native_states"], after["native_states"],
        "{attack}: rejected restore changed native SFMT checkpoints"
    );
}

#[test]
fn rand_warning_mask_rejects_original_and_compati_snapshots_before_native_restore() {
    for (snake, compati) in [(false, false), (true, true)] {
        let mut options = method_options(snake);
        options.compatible_rand = compati;
        let artifact = compile_source_with_options(
            "@SYSTEM_TITLE\nRESULT:10 = RAND(10)\nWAIT\nRETURN\n",
            &options,
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(warnings(&report).is_empty());
        assert!(matches!(
            vm.fiber_status(fiber),
            Some(FiberStatus::WaitingHost(_))
        ));
        let snapshot = vm.snapshot(&natives).unwrap();
        // Prove the source snapshot is valid under its own unchanged configuration.
        let mut restored_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot.clone(),
            &mut host,
            &mut restored_natives,
        )
        .unwrap();
        let saved = serde_json::to_value(snapshot).unwrap();
        assert_eq!(saved["rand_warning_mask"], 0);
        for mask in [1, 2, 3] {
            let mut invalid = saved.clone();
            invalid["rand_warning_mask"] = mask.into();
            reject_rand_snapshot_without_side_effects(
                &artifact,
                invalid,
                &format!("snake={snake}/compati={compati}/mask={mask}"),
            );
        }
    }
}

#[test]
fn rand_first_source_omission_is_rejected_without_confusing_explicit_minimum() {
    for expression in ["RAND(, 1)", "RAND(, -1)"] {
        let analysis = analyze_project(
            AnalysisInput {
                project_data: project_data(),
                sources: vec![ProjectSource {
                    relative_path: "rand-omitted.erb".into(),
                    payload: SourcePayload::Utf8(format!("@SYSTEM_TITLE\nRETURN {expression}\n")),
                }],
            },
            &method_options(true),
            &ExtensionRegistry::default(),
        );
        assert!(analysis.diagnostics.iter().any(|item|
            item.code == erabasic_analyzer::AnalyzerDiagnosticCode::InvalidArgument),
            "{expression}: {:?}", analysis.diagnostics);
    }
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:10 = STRFORMCHECK("{RAND(, 1)}")
RESULT:11 = STRFORMCHECK("{RAND(, -1)}")
RESULT:12 = RAND(__INT_MIN__, __INT_MIN__ + 1)
RESULT:13 = TOINT(STRFORM("{RAND(__INT_MIN__, __INT_MIN__ + 1)}"))
RESULT:14 = STRFORMCHECK("{RAND(__INT_MIN__, __INT_MIN__ + 1)}")
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
    for (index, expected) in [(10, 0), (11, 0), (12, i64::MIN), (13, i64::MIN), (14, 1)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
    assert!(warnings(&report).is_empty());
}
