use super::*;
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the inline method fixture and same-VM warmup/debug comparison in one scenario."
)]
fn dynamic_methods_remain_observable_after_memo_warmup_and_match_debug_execution() {
    let warm_body = "INDEX = FINDELEMENT(RESULTS, NEEDLE, 0, 2, 1)\n".repeat(100);
    let source = format!(
        "@WARM_ENTRY\nRETURN LOOKUP(\"target\")\n@LOOKUP(NEEDLE)\n#FUNCTION\n#DIMS NEEDLE\n#DIM INDEX\n{warm_body}RETURNF INDEX\n"
    ) + r#"
@DYNAMIC_ENTRY
RESULT:10 = WRAP_INT()
RESULTS:10 '= WRAP_STR()
RESULT:11 = WRAP_EXISTS()
RESULTS:11 '= WRAP_FORM()
RETURN
@WRAP_INT
#FUNCTION
RETURNF GETMETH(STR:0, , FLAG)
@WRAP_STR
#FUNCTIONS
RETURNF GETMETHS(STR:1)
@WRAP_EXISTS
#FUNCTION
RETURNF EXISTMETH(STR:3)
@WRAP_FORM
#FUNCTIONS
RETURNF STRFORM("{GETMETH(\"READ_VALUE\")}")
@FIRST(NUMBERS)
#FUNCTION
#DIM REF NUMBERS
NUMBERS:0 += 1
FLAG:1 += 1
RETURNF NUMBERS:0
@SECOND(NUMBERS)
#FUNCTION
#DIM REF NUMBERS
NUMBERS:0 += 10
FLAG:1 += 1
RETURNF NUMBERS:0
@TEXT_A
#FUNCTIONS
FLAG:2 += 1
RETURNF STR:2 + "A"
@TEXT_B
#FUNCTIONS
FLAG:2 += 1
RETURNF STR:2 + "B"
@READ_VALUE
#FUNCTION
RETURNF FLAG:0
@UNUSED_BREAKPOINT
#FUNCTION
RETURNF 0
"#;
    for snake in [false, true] {
        let mut options = AnalyzerOptions::analysis_mode();
        if snake {
            options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            );
        }
        let artifact = compile_source_with_options(&source, &options);
        let function = |name: &str| {
            artifact
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
                .key
        };
        let variable = |name: &str| {
            artifact
                .globals
                .iter()
                .find(|global| global.name == name)
                .unwrap()
                .key
        };
        let mut results = Vec::new();
        for debugging in [false, true] {
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 7);
            if debugging {
                // An enabled breakpoint outside the exercised path uses the existing
                // debug scheduler, which disables both VM memo paths without stopping.
                vm.update_breakpoints(
                    &[VmBreakpoint {
                        id: 1,
                        enabled: true,
                        hit_count: 0,
                        location: VmBreakpointLocation::Function(function("UNUSED_BREAKPOINT")),
                    }],
                    &[],
                )
                .unwrap();
            }
            vm.write_variable(
                variable("RESULTS"),
                &[0],
                None,
                VmValue::String("target".into()),
            )
            .unwrap();
            let mut warm_instructions = Vec::new();
            for _ in 0..2 {
                vm.spawn_entry(function("WARM_ENTRY"), Vec::new()).unwrap();
                let report = vm.run_slice(
                    &mut ReadyHost::default(),
                    &mut natives,
                    RunBudget::default(),
                );
                assert!(
                    !report.events.iter().any(|event| matches!(
                        event,
                        VmEvent::FiberFaulted { .. } | VmEvent::DebugStopped(_)
                    )),
                    "{report:?}"
                );
                warm_instructions.push(report.instructions);
            }
            if !debugging {
                assert!(
                    warm_instructions[1] < warm_instructions[0],
                    "the same VM must have an actually warmed memo before dynamic calls: {warm_instructions:?}"
                );
            }
            let mut observations = Vec::new();
            for (round, (number, text, existence, expected_value, expected_exists)) in [
                ("FIRST", "TEXT_A", "READ_VALUE", 1, 1),
                ("FIRST", "TEXT_A", "TEXT_A", 2, 2),
                ("SECOND", "TEXT_B", "MISSING", 12, 0),
                ("SECOND", "TEXT_B", "READ_VALUE", 22, 1),
                ("FIRST", "TEXT_A", "TEXT_B", 23, 2),
            ]
            .into_iter()
            .enumerate()
            {
                for (slot, value) in [
                    (0, number.to_owned()),
                    (1, text.to_owned()),
                    (2, format!("round{round}")),
                    (3, existence.to_owned()),
                ] {
                    vm.write_variable(variable("STR"), &[slot], None, VmValue::String(value))
                        .unwrap();
                }
                vm.spawn_entry(function("DYNAMIC_ENTRY"), Vec::new())
                    .unwrap();
                let report = vm.run_slice(
                    &mut ReadyHost::default(),
                    &mut natives,
                    RunBudget::default(),
                );
                assert!(
                    !report.events.iter().any(|event| matches!(
                        event,
                        VmEvent::FiberFaulted { .. } | VmEvent::DebugStopped(_)
                    )),
                    "{report:?}"
                );
                let observed = [
                    ("RESULT", 10),
                    ("RESULT", 11),
                    ("RESULTS", 10),
                    ("RESULTS", 11),
                    ("FLAG", 0),
                    ("FLAG", 1),
                    ("FLAG", 2),
                ]
                .map(|(name, index)| vm.read_variable(variable(name), &[index], None).unwrap());
                assert_eq!(
                    observed,
                    [
                        VmValue::Integer(expected_value),
                        VmValue::Integer(expected_exists),
                        VmValue::String(format!(
                            "round{round}{}",
                            if text == "TEXT_A" { "A" } else { "B" }
                        )),
                        VmValue::String(expected_value.to_string()),
                        VmValue::Integer(expected_value),
                        VmValue::Integer(i64::try_from(round + 1).unwrap()),
                        VmValue::Integer(i64::try_from(round + 1).unwrap()),
                    ]
                );
                observations.push(observed);
            }
            results.push(observations);
        }
        assert_eq!(
            results[0], results[1],
            "memo-enabled and debugger execution differ"
        );
    }
}

#[test]
fn title_startup_reports_memory_before_program_indexing_with_monotonic_progress() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN\n");
    let mut events = Vec::new();
    let _runtime = RuntimeVm::new_for_title_with_seed_and_progress(
        validated(&artifact),
        VmConfig::default(),
        1,
        &mut |event| events.push(event),
    );

    let first_indexing = events
        .iter()
        .position(|event| event.stage == VmPreparationStage::IndexingProgram)
        .expect("program indexing progress");
    assert!(
        events[..first_indexing]
            .iter()
            .all(|event| event.stage == VmPreparationStage::InitializingMemory)
    );
    for stage in [
        VmPreparationStage::InitializingMemory,
        VmPreparationStage::IndexingProgram,
    ] {
        let stage_events = events
            .iter()
            .filter(|event| event.stage == stage)
            .collect::<Vec<_>>();
        assert_eq!(stage_events.first().map(|event| event.completed), Some(0));
        assert_eq!(
            stage_events.last().map(|event| event.completed),
            stage_events.last().map(|event| event.total)
        );
        assert!(
            stage_events
                .windows(2)
                .all(|events| events[0].completed <= events[1].completed)
        );
    }
}

struct ImportCheckingNative {
    expected: RuntimeImport,
}

impl NativeService for ImportCheckingNative {
    fn call(
        &mut self,
        request: NativeCallRequest,
    ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
        assert_eq!(request.import, self.expected);
        assert_eq!(request.arguments, [VmValue::Integer(-4)]);
        assert!(request.places.is_empty());
        assert!(request.implicit_places.is_empty());
        Ok(NativeReady::value(VmValue::Integer(17)))
    }

    fn requires_rollback_checkpoint(&self) -> bool {
        false
    }
}

#[test]
fn registered_native_receives_the_complete_runtime_import() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = ABS(-4)\nRETURN RESULT\n");
    let native = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name.eq_ignore_ascii_case("abs"))
        .expect("ABS native import");
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    assert!(!natives.register(
        native.import.key,
        ImportCheckingNative {
            expected: native.import.clone(),
        },
    ));
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        "{:#?}",
        report.events
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(17))
    );
}

#[test]
fn era_function_local_persists_across_calls() {
    let artifact = compile_source("@COUNTER\nLOCAL:0 += 1\nRESULT = LOCAL:0\nRETURN RESULT\n");
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let local = artifact
        .globals
        .iter()
        .find(|global| {
            global.name == "LOCAL" && global.storage == BytecodeStorage::FunctionPersistent
        })
        .expect("persistent LOCAL")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut host = ReadyHost::default();
    for expected in [1, 2] {
        vm.spawn_entry(entry, Vec::new()).unwrap();
        vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_eq!(
            vm.read_variable(result, &[0], None),
            Ok(VmValue::Integer(expected))
        );
        assert_eq!(
            vm.read_variable(local, &[0], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn function_local_size_directives_override_project_defaults() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRETURN RESIZED_LOCALS()\n\
         @RESIZED_LOCALS\n#FUNCTION\n#LOCALSIZE 2500\n#LOCALSSIZE 200\n\
         LOCAL:1000 = 7\nLOCALS:150 '= \"ok\"\n\
         RETURNF (LOCAL:1000 == 7) && (LOCALS:150 == \"ok\")\n",
    );
    let resized_locals = artifact
        .functions
        .iter()
        .find(|function| function.name == "RESIZED_LOCALS")
        .expect("RESIZED_LOCALS")
        .key;
    let local_dimensions = artifact
        .globals
        .iter()
        .filter(|global| {
            global.owner == Some(resized_locals)
                && global.storage == BytecodeStorage::FunctionPersistent
        })
        .map(|global| (global.name.as_str(), global.dimensions.as_slice()))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(local_dimensions.get("LOCAL"), Some(&[2500_u64].as_slice()));
    assert_eq!(local_dimensions.get("LOCALS"), Some(&[200_u64].as_slice()));
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn era_function_fallthrough_resets_result_zero() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 9\nCALL FALLTHROUGH\nFLAG:0 = RESULT\nRETURN RESULT\n@FALLTHROUGH\nLOCAL = 1\n",
    );
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );

    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(0)));
}

#[test]
fn swap_native_commits_both_places() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 10\nFLAG:1 = 20\nSWAP FLAG:0, FLAG:1\nRESULT = FLAG:0 * 100 + FLAG:1\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        "{:#?}",
        report.events
    );
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(20)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(10)));
}

#[test]
fn array_shift_and_remove_commit_after_validating_the_whole_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 1\nFLAG:1 = 2\nFLAG:2 = 3\nFLAG:3 = 4\nARRAYSHIFT FLAG, 1, 9, 0, 4\nARRAYREMOVE FLAG, 1, 2\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        "{:#?}",
        report.events
    );
    let values = (0..4)
        .map(|index| vm.read_variable(flag, &[index], None).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            VmValue::Integer(9),
            VmValue::Integer(3),
            VmValue::Integer(0),
            VmValue::Integer(0),
        ]
    );
}

#[test]
fn findelement_uses_the_verified_regex_subset() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"zz\"\nRESULTS:1 '= \"abc\"\nRESULTS:2 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"^ab$\", 0, 3, 1)\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"<p>----------------</p>\"\nRESULT = FINDELEMENT(RESULTS, \"([^ ])\\\\1{15}\", 0, 1, 0)\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));

    // An empty range never inspects an element, so even an invalid pattern is
    // intentionally not compiled and the query returns the not-found sentinel.
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"a(?=b)\", 0, 0, 0)\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(-1));

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"ab\"\nRESULT = FINDELEMENT(RESULTS, \"a(?=b)\", 0, 1, 0)\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. }
            if fault.message.contains("lookaround")
    )));
}

#[test]
fn findelement_returns_absolute_indices_for_bounded_forward_and_reverse_ranges() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:2 '= \"target\"\nRESULTS:4 '= \"target\"\nRESULT:0 = FINDELEMENT(RESULTS, \"target\", 2, 5, 1)\nRESULT:1 = FINDLASTELEMENT(RESULTS, \"target\", 2, 5, 1)\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );

    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(4))
    );
}

#[test]
fn findelement_cache_is_invalidated_by_target_variable_writes() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"target\"\nRESULT:0 = FINDELEMENT(RESULTS, \"target\", 0, 2, 1)\nRESULT:1 = FINDELEMENT(RESULTS, \"target\", 0, 2, 1)\nRESULTS:0 '= \"\"\nRESULTS:1 '= \"target\"\nRESULT:2 = FINDELEMENT(RESULTS, \"target\", 0, 2, 1)\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );

    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
}
