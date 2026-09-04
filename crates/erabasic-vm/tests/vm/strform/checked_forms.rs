use super::*;
#[test]
fn strformcheck_catches_parse_and_expansion_failures_without_rolling_back_effects() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{SIDE()}")
RESULT:1 = STRFORMCHECK("{SIDE()} {FLAG:9999999}")
RESULT:2 = STRFORMCHECK("{")
RESULT:3 = STRFORMCHECK("{UNKNOWN_FORM_VARIABLE}")
RESULTS:0 '= "{SIDE()} {FLAG:9999999}"
RESULTS:1 '= "{STRFORMCHECK(RESULTS:0)}"
RESULT:4 = STRFORMCHECK(RESULTS:1)
RESULT:5 = FLAG:0
RETURN RESULT:0
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:0
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
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, 1), (1, 0), (2, 0), (3, 0), (4, 1), (5, 3)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(expected));
    }
}

#[test]
fn strformcheck_outer_argument_failure_is_not_caught_by_its_own_checkpoint() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK(BAD_SOURCE())
FLAG:1 = 1
RETURN
@BAD_SOURCE
#FUNCTIONS
FLAG:0 += 1
RESULT:9 = FLAG:9999999
RETURNF "unused"
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(matches!(
        fault.category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Bounds)
    ));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn runtime_form_user_calls_discard_extra_actuals_before_evaluation() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("{TAKE(7, SIDE())}")
RESULTS:1 '= STRFORM("{GETMETH(\"TAKE\", , 8, SIDE())}")
RESULT:1 = FLAG:0
RETURN
@TAKE(ARG)
#FUNCTION
RETURNF ARG
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("7".into()));
    assert_method_watch(&vm, &artifact, "RESULTS", 1, VmValue::String("8".into()));
    assert_method_watch(&vm, &artifact, "RESULT", 1, VmValue::Integer(0));
}

#[test]
fn strformcheck_checkpoint_survives_input_wait_and_rejects_forged_snapshot_markers() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("{WAIT_FAILURE()}")
FLAG:1 = 1
RETURN
@WAIT_FAILURE
#FUNCTION
INPUT
FLAG:0 += 1
RETURNF FLAG:9999999
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .expect("checked expansion must reach the real Host wait");
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    for field in ["id", "work_depth", "value_depth", "owner_stack_depth"] {
        let mut corrupted = saved.clone();
        corrupted["fibers"][fiber.0.to_string()]["frames"][0]["runtime_form"]["checkpoints"][0]
            [field] = serde_json::json!(999_999);
        let mut rejected = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        assert!(
            Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                serde_json::from_value(corrupted).unwrap(),
                &mut rejected,
                &mut natives
            )
            .is_err(),
            "{field}"
        );
        assert!(
            rejected.rebound.is_empty(),
            "{field}: invalid checkpoint rebound Host"
        );
    }
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        serde_json::from_value(saved).unwrap(),
        &mut host,
        &mut natives,
    )
    .unwrap();
    restored.resume_host(request, HostReady::empty()).unwrap();
    let resumed = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    completed_without_fault(&resumed, fiber);
    assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&restored, &artifact, "FLAG", 1, VmValue::Integer(1));
}

#[test]
fn strformcheck_does_not_catch_call_depth_resource_failure() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = STRFORMCHECK("%RECURSE()%")
FLAG:1 = 1
RETURN
@RECURSE
#FUNCTIONS
RETURNF STRFORM("%RECURSE()%")
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_call_depth: 4,
            ..VmConfig::default()
        },
    );
    let fault = take_fault(report);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::ResourceLimit);
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
}

#[test]
fn call_text_try_catches_binding_and_name_failure_but_not_callee_failure() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
TRYCCALLSTR "TAKE(\"bad type\")"
FLAG:0 = 99
CATCH
FLAG:0 = 1
ENDCATCH
TRYCALLSTR "MISSING, 7"
TRYCALLSTR "TAKE(UNKNOWN_VARIABLE)"
CALLSTR "TAKE(8)"
RETURN
@TAKE(ARG)
FLAG:1 = ARG
RETURN
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(1));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(8));
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nTRYCALLSTR \"TAKE(7)\"\nRETURN\n@TAKE(ARG)\nFLAG:0 = FLAG:9999999\nRETURN\n",
        &method_options(true),
    );
    let (_, report) = run_entry(&artifact, VmConfig::default());
    assert!(matches!(
        take_fault(report).category,
        erabasic_vm::FaultCategory::Script(_)
    ));
}

#[test]
fn runtime_form_prefix_postfix_share_fixed_profile_boundary_results_and_warnings() {
    for snake in [false, true] {
        for (expression, initial, original_return, original_store, snake_return, snake_store) in [
            ("++FLAG:0", i64::MAX, i64::MIN, i64::MIN, i64::MAX, i64::MAX),
            ("--FLAG:0", i64::MIN, i64::MAX, i64::MAX, i64::MIN, i64::MIN),
            (
                "FLAG:0++",
                i64::MAX,
                i64::MAX,
                i64::MIN,
                i64::MAX - 1,
                i64::MAX,
            ),
            (
                "FLAG:0--",
                i64::MIN,
                i64::MIN,
                i64::MAX,
                i64::MIN + 1,
                i64::MIN,
            ),
        ] {
            let artifact = compile_source_with_options(
                &format!("@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"{{{expression}}}\")\nRETURN\n"),
                &method_options(snake),
            );
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            vm.write_variable(
                named_key(&artifact, "FLAG"),
                &[0],
                None,
                VmValue::Integer(initial),
            )
            .unwrap();
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let entry = artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .unwrap()
                .key;
            let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            completed_without_fault(&report, fiber);
            let (returned, stored) = if snake {
                (snake_return, snake_store)
            } else {
                (original_return, original_store)
            };
            assert_method_watch(
                &vm,
                &artifact,
                "RESULTS",
                0,
                VmValue::String(returned.to_string()),
            );
            assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(stored));
            assert_eq!(
                report
                    .events
                    .iter()
                    .filter(|event| matches!(event,
                VmEvent::Diagnostic { code, .. } if code == "compat.arithmetic.overflow"))
                    .count(),
                usize::from(snake)
            );
        }
    }
}

#[test]
fn runtime_form_mutation_preserves_index_value_order_and_character_and_ref_places() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC NUMBERS, 3
FLAG:0 = 10
RESULTS:0 '= STRFORM("{++FLAG:(INDEX())}|{FLAG:0++}|{OBSERVE()}")
ADDVOIDCHARA
CFLAG:0:0 = 20
RESULTS:1 '= STRFORM("{CFLAG:0:0++}|{++CFLAG:0:0}")
NUMBERS:0 = 30
RESULTS:2 '= STRFORM("%CHANGE_REF(NUMBERS)%")
RESULT:2 = NUMBERS:0
RETURN
@INDEX
#FUNCTION
FLAG:1 = FLAG:1 * 10 + 1
RETURNF 0
@OBSERVE
#FUNCTION
FLAG:1 = FLAG:1 * 10 + 2
RETURNF FLAG:0
@CHANGE_REF(VALUES)
#FUNCTIONS
#DIM REF VALUES
RETURNF STRFORM("{VALUES:0++}|{++VALUES:0}")
"#,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    for (index, expected) in [(0, "11|11|12"), (1, "20|22"), (2, "30|32")] {
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            index,
            VmValue::String(expected.into()),
        );
    }
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(12));
    assert_method_watch(&vm, &artifact, "RESULT", 2, VmValue::Integer(32));
    assert_eq!(
        vm.read_variable(named_key(&artifact, "CFLAG"), &[0], Some(0)),
        Ok(VmValue::Integer(22))
    );
}

#[test]
fn existvar_evaluates_source_then_mode_then_source_only_for_nonzero_mode() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), MODE_VALUE())
RESULT:1 = FLAG:0
FLAG:0 = 0
RESULTS:0 '= STRFORM("{EXISTVAR(NAME_SOURCE(), MODE_VALUE())}")
RESULT:2 = FLAG:0
FLAG:0 = 0
FLAG:1 = 1
RESULT:3 = EXISTVAR(NAME_SOURCE(), MODE_VALUE())
RESULT:4 = FLAG:0
RETURN RESULT:0
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RETURNF "FLAG"
@MODE_VALUE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF FLAG:1
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
    for (index, value) in [(0, 1), (1, 12), (2, 12), (3, 1), (4, 121)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("1".into()));
}

#[test]
fn existvar_expression_probe_resolves_without_reading_cells_or_executing_terms() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM LOCAL_ONLY, 2
RESULT:0 = EXISTVAR("LOCAL_ONLY")
RESULT:1 = EXISTVAR("LOCAL_ONLY:999999", 1)
RESULT:2 = EXISTVAR("1 / 0", 1)
RESULT:3 = EXISTVAR("SIDE()", 1)
RESULT:4 = EXISTVAR("GETTIME()", 1)
RESULT:5 = EXISTVAR("FLAG:\"not a real key\"", 1)
RESULT:6 = EXISTVAR("", 1)
RESULT:7 = EXISTVAR("NO_SUCH_VARIABLE", 1)
RESULT:8 = EXISTVAR("1 +", 1)
CALL CHECK_REF, LOCAL_ONLY
RETURN RESULT:0
@SIDE
#FUNCTION
FLAG:0 += 1
RETURNF FLAG:99999999
@CHECK_REF(VALUES)
#DIM REF VALUES
RESULT:9 = EXISTVAR("VALUES:999999", 1)
RETURN RESULT:0
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
    for (index, value) in [
        (0, 1),
        (1, 1),
        (2, 1),
        (3, 1),
        (4, 1),
        (5, 1),
        (6, 1),
        (7, 0),
        (8, 0),
        (9, 1),
    ] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(0));
}

#[test]
fn existvar_catches_only_second_source_script_failure_and_preserves_effects() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), 1)
FLAG:1 = 1
RETURN
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:9 = FLAG:99999999
ENDIF
RETURNF "FLAG"
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
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(0));
    assert_method_watch(&vm, &artifact, "FLAG", 0, VmValue::Integer(2));
    assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(1));
    for source in [
        "@SYSTEM_TITLE\nRESULT = EXISTVAR(BAD_NAME(), 1)\nFLAG:1 = 1\nRETURN\n@BAD_NAME\n#FUNCTIONS\nRESULT:9 = FLAG:99999999\nRETURNF \"FLAG\"\n",
        "@SYSTEM_TITLE\nRESULT = EXISTVAR(\"FLAG\", BAD_MODE())\nFLAG:1 = 1\nRETURN\n@BAD_MODE\n#FUNCTION\nRETURNF FLAG:99999999\n",
    ] {
        let artifact = compile_source_with_options(source, &method_options(true));
        let (vm, report) = run_entry(&artifact, VmConfig::default());
        assert!(matches!(
            take_fault(report).category,
            erabasic_vm::FaultCategory::Script(_)
        ));
        assert_method_watch(&vm, &artifact, "FLAG", 1, VmValue::Integer(0));
    }
}

#[test]
fn existvar_second_source_wait_preserves_checkpoint_and_rejects_deleted_snapshot_state() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR(NAME_SOURCE(), 1)
RETURN RESULT:0
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
INPUT
ENDIF
RETURNF "FLAG:999999"
"#,
        &method_options(true),
    );
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    let saved = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
    let mut bad = saved.clone();
    bad["fibers"][fiber.0.to_string()]["frames"][0]["existvar_checks"] = serde_json::json!([]);
    let mut rejected = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    assert!(
        Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            serde_json::from_value(bad).unwrap(),
            &mut rejected,
            &mut natives
        )
        .is_err()
    );
    assert!(rejected.rebound.is_empty());
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
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report.events.iter().any(|event| matches!(event,
            VmEvent::FiberCompleted { fiber: completed, value: Some(VmValue::Integer(1)) }
            if *completed == fiber
        )),
        "{report:?}"
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&restored, &artifact, "RESULT", 0, VmValue::Integer(1));
    assert_method_watch(&restored, &artifact, "FLAG", 0, VmValue::Integer(2));
}

#[test]
fn existvar_probe_preserves_rand_and_character_parse_policy_without_cell_reads() {
    for compatible_rand in [false, true] {
        for system_no_target in [false, true] {
            let mut options = method_options(true);
            options.compatible_rand = compatible_rand;
            options.system_no_target = system_no_target;
            let artifact = compile_source_with_options(
                r#"@SYSTEM_TITLE
RESULT:0 = EXISTVAR("RAND", 1)
RESULT:1 = EXISTVAR("RAND:0", 1)
RESULT:2 = EXISTVAR("RAND:(+0)", 1)
RESULT:3 = EXISTVAR("RAND:(-0)", 1)
RESULT:4 = EXISTVAR("RAND:(0 + 0)", 1)
RESULT:5 = EXISTVAR("CFLAG:0", 1)
RESULT:6 = EXISTVAR("CFLAG:0:0", 1)
RESULT:7 = EXISTVAR("CFLAG", 1)
RETURN RESULT:0
"#,
                &options,
            );
            assert_eq!(artifact.call_compatibility.compatible_rand, compatible_rand);
            assert_eq!(
                artifact.call_compatibility.system_no_target,
                system_no_target
            );
            let (vm, report) = run_entry(&artifact, VmConfig::default());
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            for (index, value) in [
                (0, i64::from(compatible_rand)),
                (1, i64::from(compatible_rand)),
                (2, i64::from(compatible_rand)),
                (3, 1),
                (4, 1),
                (5, i64::from(!system_no_target)),
                (6, 1),
                (7, 1),
            ] {
                assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
            }
        }
    }
}
