use super::*;
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "One fixture compares normal/debug execution and restored diagnostic identity."
)]
fn snake_arithmetic_is_profile_bound_and_warning_state_survives_snapshot() {
    let source = r#"
@SYSTEM_TITLE
#DIM COUNTER
FLAG:0 = 9223372036854775807
FLAG:1 = -9223372036854775807 - 1
RESULT:0 = FLAG:0 + 1
RESULT:1 = 9223372036854775807 + 1
RESULT:2 = FLAG:0++
RESULT:3 = FLAG:1--
RESULT:4 = 0 - FLAG:1
RESULT:5 = FLAG:0 / 0
RESULTS:0 '= STRFORM("{FLAG:0 + 1}")
FOR COUNTER, 9223372036854775806, 9223372036854775807, 2
RESULT:6 = COUNTER
NEXT
RESULT:7 = COUNTER
CALL OVERFLOW
CALL OVERFLOW
RETURN RESULT:0
@OVERFLOW
RESULT:8 = FLAG:0 + 1
RETURN RESULT:0
@UNUSED_BREAKPOINT
RETURN
"#;
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let artifact = compile_source_with_options(source, &options);
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let unused = artifact
        .functions
        .iter()
        .find(|function| function.name == "UNUSED_BREAKPOINT")
        .unwrap()
        .key;
    let mut runs = Vec::new();
    for debugging in [false, true] {
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
        if debugging {
            vm.update_breakpoints(
                &[VmBreakpoint {
                    id: 1,
                    enabled: true,
                    hit_count: 0,
                    location: VmBreakpointLocation::Function(unused),
                }],
                &[],
            )
            .unwrap();
        }
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        let observed: Vec<_> = (0..9)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect();
        let expected = [
            i64::MAX,
            i64::MAX,
            i64::MAX - 1,
            i64::MIN + 1,
            i64::MIN,
            0,
            i64::MAX - 1,
            i64::MAX,
            i64::MAX,
        ];
        assert_eq!(observed, expected.map(VmValue::Integer));
        assert_eq!(
            vm.read_variable(results, &[0], None).unwrap(),
            VmValue::String(i64::MAX.to_string())
        );
        let warnings: Vec<_> = report
            .events
            .iter()
            .filter_map(|event| match event {
                VmEvent::Diagnostic { code, origin, .. }
                    if code.starts_with("compat.arithmetic.") =>
                {
                    Some((
                        code.clone(),
                        origin.function_name.clone(),
                        origin.instruction,
                    ))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            warnings
                .iter()
                .filter(|(_, function, _)| function == "OVERFLOW")
                .count(),
            1
        );
        assert!(
            warnings
                .iter()
                .any(|(code, _, _)| code == "compat.arithmetic.divide_by_zero")
        );
        assert!(warnings.len() >= 8, "{warnings:?}");
        runs.push((observed, warnings));
        let snapshot = vm.snapshot(&natives).unwrap();
        let encoded = snapshot.encode().unwrap();
        let decoded = VmSnapshot::decode(&encoded, 16 * 1024 * 1024).unwrap();
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            decoded,
            &mut ReadyHost::default(),
            &mut natives,
        )
        .unwrap();
        restored.spawn_entry(entry, Vec::new()).unwrap();
        let repeated = restored.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !repeated.events.iter().any(|event| matches!(
                event,
                VmEvent::Diagnostic { .. } | VmEvent::FiberFaulted { .. }
            )),
            "{repeated:?}"
        );
        assert_eq!(
            restored.read_variable(result, &[8], None).unwrap(),
            VmValue::Integer(i64::MAX)
        );
    }
    assert_eq!(runs[0], runs[1]);
}

#[test]
fn runtime_minimum_divide_and_remainder_fault_without_panicking_in_both_profiles() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let mut options = AnalyzerOptions::analysis_mode();
        options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(profile);
        for operation in ["/", "%"] {
            let source = format!(
                "@SYSTEM_TITLE\nFLAG:0 = -9223372036854775807 - 1\nRESULT = FLAG:0 {operation} -1\nRETURN\n"
            );
            let artifact = compile_source_with_options(&source, &options);
            let entry = artifact
                .functions
                .iter()
                .find(|function| function.name == "SYSTEM_TITLE")
                .unwrap()
                .key;
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            vm.spawn_entry(entry, Vec::new()).unwrap();
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            assert!(
                report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{profile:?} {operation}: {report:?}"
            );
        }
    }
}

fn snake_arithmetic_test_artifact(source: &str) -> BytecodeArtifact {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    compile_source_with_options(source, &options)
}

fn disable_arithmetic_test_optimizations(vm: &mut Vm, artifact: &BytecodeArtifact) {
    let unused = artifact
        .functions
        .iter()
        .find(|function| function.name == "UNUSED_BREAKPOINT")
        .unwrap()
        .key;
    vm.update_breakpoints(
        &[VmBreakpoint {
            id: 1,
            enabled: true,
            hit_count: 0,
            location: VmBreakpointLocation::Function(unused),
        }],
        &[],
    )
    .unwrap();
}

#[test]
fn snake_warm_selector_cannot_skip_first_getter_warning_or_fault() {
    for (expression, faults) in [("FLAG:0 + 1", false), ("FLAG:1 / -1", true)] {
        let artifact = snake_arithmetic_test_artifact(&format!(
            r#"
@SYSTEM_TITLE
FLAG:0 = 9223372036854775807
FLAG:1 = -9223372036854775807 - 1
DA:1:0 = 7
RESULT:11 = 73
RESULT:10 = LOOKUP_INDEX("DATA", "FIELD")
RESULT:11 = READ_DATA(1, "FIELD")
RETURN RESULT
@READ_DATA, ARG, ARGS
#FUNCTION
#LOCALSIZE 2
LOCAL:1 = {expression}
LOCAL = ISNUMERIC(ARGS) ? TOINT(ARGS) # LOOKUP_INDEX("DATA", ARGS)
RETURNF DA:ARG:LOCAL
@LOOKUP_INDEX, ARGS, ARGS:1
#FUNCTION
SIF ARGS:1 == "FIELD"
RETURNF 0
RETURNF 1
@UNUSED_BREAKPOINT
RETURN
"#
        ));
        let entry = artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|g| g.name == "RESULT")
            .unwrap()
            .key;
        let mut observations = Vec::new();
        for debugging in [false, true] {
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            if debugging {
                disable_arithmetic_test_optimizations(&mut vm, &artifact);
            }
            vm.spawn_entry(entry, Vec::new()).unwrap();
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            let faulted = report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. }));
            let warnings: Vec<_> = report
                .events
                .iter()
                .filter_map(|event| match event {
                    VmEvent::Diagnostic { code, origin, .. }
                        if origin.function_name == "READ_DATA" =>
                    {
                        Some(code.clone())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(faulted, faults, "{report:?}");
            assert_eq!(warnings.len(), usize::from(!faults), "{report:?}");
            assert_eq!(
                vm.read_variable(result, &[10], None).unwrap(),
                VmValue::Integer(0)
            );
            let value = vm.read_variable(result, &[11], None).unwrap();
            assert_eq!(value, VmValue::Integer(if faults { 73 } else { 7 }));
            observations.push((value, warnings, faulted));
        }
        assert_eq!(observations[0], observations[1]);
    }
}

#[test]
fn snake_function_memo_keeps_safe_hits_without_suppressing_overflow_diagnostics() {
    let artifact = snake_arithmetic_test_artifact(
        r"
@SYSTEM_TITLE, ARG
RESULT = ADD_ONE(ARG)
RETURN RESULT
@ADD_ONE, ARG
#FUNCTION
RETURNF ARG + 1
@UNUSED_BREAKPOINT
RETURN
",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|g| g.name == "RESULT")
        .unwrap()
        .key;
    let mut observations = Vec::new();
    for debugging in [false, true] {
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        if debugging {
            disable_arithmetic_test_optimizations(&mut vm, &artifact);
        }
        let mut calls = Vec::new();
        let mut instructions = Vec::new();
        for argument in [4, 4, i64::MAX, i64::MAX, 5, 5] {
            vm.spawn_entry(entry, vec![VmValue::Integer(argument)])
                .unwrap();
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            let warnings = report
                .events
                .iter()
                .filter(|event| {
                    matches!(event,
                        VmEvent::Diagnostic { code, origin, .. }
                        if code == "compat.arithmetic.overflow" && origin.function_name == "ADD_ONE"
                    )
                })
                .count();
            calls.push((vm.read_variable(result, &[0], None).unwrap(), warnings));
            instructions.push(report.instructions);
        }
        assert_eq!(
            calls,
            [(5, 0), (5, 0), (i64::MAX, 1), (i64::MAX, 0), (6, 0), (6, 0)]
                .map(|(value, warnings)| (VmValue::Integer(value), warnings))
        );
        if !debugging {
            assert!(
                instructions[1] < instructions[0],
                "safe call did not hit memo: {instructions:?}"
            );
            assert!(
                instructions[5] < instructions[4],
                "new safe argument did not hit memo: {instructions:?}"
            );
        }
        observations.push(calls);
    }
    assert_eq!(observations[0], observations[1]);
}

#[test]
fn snake_arithmetic_diagnostics_are_reissued_for_a_new_generation() {
    let source = "@SYSTEM_TITLE\nRESULT = FLAG:0 + 1\nRETURN RESULT\n";
    let base = snake_arithmetic_test_artifact(source);
    let target = snake_arithmetic_test_artifact(
        &source.replace("RETURN RESULT", "RESULT:1 = 2\nRETURN RESULT"),
    );
    let patch = create_patch(&base, &target);
    let entry = base
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let flag = base.globals.iter().find(|g| g.name == "FLAG").unwrap().key;
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&base);
    vm.write_variable(flag, &[0], None, VmValue::Integer(i64::MAX))
        .unwrap();
    let mut generations = Vec::new();
    for pass in 0..3 {
        if pass == 2 {
            vm.prepare_hot_reload(
                &patch,
                &erabasic_compiler::runtime_native_validation_context(
                    &target,
                    &default_host_registry(),
                ),
            )
            .unwrap();
            vm.commit_hot_reload().unwrap();
        }
        vm.spawn_entry(entry, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        let warnings: Vec<_> = report
            .events
            .iter()
            .filter_map(|event| match event {
                VmEvent::Diagnostic { code, origin, .. }
                    if code == "compat.arithmetic.overflow" =>
                {
                    Some(origin.generation)
                }
                _ => None,
            })
            .collect();
        assert_eq!(warnings.len(), usize::from(pass != 1), "{report:?}");
        generations.extend(warnings);
    }
    assert_ne!(generations[0], generations[1]);
}
