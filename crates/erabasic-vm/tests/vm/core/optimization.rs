use super::*;

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

#[test]
fn pure_function_memoization_skips_repeated_work_and_tracks_dependency_writes() {
    fn run_instructions(calls: &str) -> u64 {
        let repeated_lookup = "INDEX = FINDELEMENT(RESULTS, NEEDLE, 0, 2, 1)\n".repeat(100);
        let source = format!(
            "@SYSTEM_TITLE\nRESULTS:0 '= \"target\"\n{calls}\nRETURN RESULT\n\
             @LOOKUP(NEEDLE)\n#FUNCTION\n#DIMS NEEDLE\n#DIM INDEX\n\
             {repeated_lookup}RETURNF INDEX\n"
        );
        let artifact = compile_source(&source);
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
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
        report.instructions
    }

    let once = run_instructions("RESULT:0 = LOOKUP(\"target\")");
    let twice = run_instructions("RESULT:0 = LOOKUP(\"target\")\nRESULT:1 = LOOKUP(\"target\")");
    assert!(
        twice.saturating_sub(once) < 20,
        "a repeated pure call executed its body again: first={once}, repeated={twice}"
    );

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"target\"\nRESULT:0 = LOOKUP(\"target\")\n\
         RESULTS:0 '= \"\"\nRESULTS:1 '= \"target\"\nRESULT:1 = LOOKUP(\"target\")\n\
         RETURN RESULT\n@LOOKUP(NEEDLE)\n#FUNCTION\n#DIMS NEEDLE\n#DIM INDEX\n\
         INDEX = FINDELEMENT(RESULTS, NEEDLE, 0, 2, 1)\nRETURNF INDEX\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
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
        Ok(VmValue::Integer(1))
    );
}

#[test]
fn function_memoization_accepts_get_no_style_private_scratch() {
    fn run_instructions(calls: &str) -> u64 {
        let source = format!(
            "@SYSTEM_TITLE\nRESULTS:0 '= \"target\"\nRESULTS:10 '= \"ITEM\"\n\
             RESULT:10 = 0\nRESULT:11 = 2\n{calls}\nRETURN RESULT\n\
             @LOOKUP, ARGS, ARGS:1\n#FUNCTION\n#LOCALSIZE 1\n#DIM START\n#DIM END\n\
             LOCAL = FINDELEMENT(RESULTS, ARGS, 10, 11, 1)\n\
             START = RESULT:LOCAL\nEND = RESULT:(LOCAL + 1)\n\
             LOCAL = FINDELEMENT(RESULTS, ESCAPE(ARGS:1), START, END, 1) - START\n\
             SIF LOCAL < 0\nTHROW missing %ARGS% %ARGS:1%\nRETURNF LOCAL\n"
        );
        let artifact = compile_source(&source);
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
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
        report.instructions
    }

    let once = run_instructions("FLAG:0 = LOOKUP(\"ITEM\", \"target\")");
    let twice = run_instructions(
        "FLAG:0 = LOOKUP(\"ITEM\", \"target\")\n\
         FLAG:1 = LOOKUP(\"ITEM\", \"target\")",
    );
    assert!(
        twice.saturating_sub(once) < 10,
        "GET_NO-style function was not memoized: first={once}, repeated={twice}"
    );
}

#[test]
fn memoized_selector_accelerates_indexed_getters_without_caching_the_target_value() {
    fn run(calls: &str) -> (u64, Vec<VmValue>) {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\nDA:1:0 = 7\n{calls}\nRETURN RESULT\n\
             @READ_DATA, ARG, ARGS\n#FUNCTION\n#LOCALSIZE 1\n\
             LOCAL = ISNUMERIC(ARGS) ? TOINT(ARGS) # LOOKUP_INDEX(\"DATA\", ARGS)\n\
             RETURNF DA:ARG:LOCAL\n\
             @LOOKUP_INDEX, ARGS, ARGS:1\n#FUNCTION\n\
             SIF ARGS:1 == \"FIELD\"\nRETURNF 0\nRETURNF 1\n"
        ));
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
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
        let values = (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect();
        (report.instructions, values)
    }

    let once = run("RESULT:0 = READ_DATA(1, \"FIELD\")");
    let repeated = run("RESULT:0 = READ_DATA(1, \"FIELD\")\n\
         RESULT:1 = READ_DATA(1, \"FIELD\")\n\
         DA:1:0 = 9\nRESULT:2 = READ_DATA(1, \"FIELD\")");
    assert!(
        repeated.0.saturating_sub(once.0) < 20,
        "the indexed getter executed its bytecode body again: once={}, repeated={}",
        once.0,
        repeated.0
    );
    assert_eq!(
        repeated.1,
        [7, 7, 9].map(VmValue::Integer),
        "the shortcut must read the current target value"
    );
}

#[test]
fn simple_array_fill_loop_preserves_results_and_instruction_accounting() {
    fn run(fill: &str) -> (u64, Vec<VmValue>, VmValue) {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\nDA:1:0 = 7\nDA:2:0 = 9\nDA:2:3 = 9\n\
             CALL CLEAR_ROW(2, 0)\nRETURN RESULT\n\
             @CLEAR_ROW(ARG, VALUE)\n#DIM VALUE\n#LOCALSIZE 1\n\
             FOR LOCAL, 0, 4\nDA:ARG:LOCAL = {fill}\nNEXT\n\
             RESULT:0 = LOCAL\nRETURN RESULT\n"
        ));
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .expect("SYSTEM_TITLE")
            .key;
        let da = artifact
            .globals
            .iter()
            .find(|global| global.name == "DA")
            .expect("DA")
            .key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
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
        let values = [
            vm.read_variable(da, &[1, 0], None).unwrap(),
            vm.read_variable(da, &[2, 0], None).unwrap(),
            vm.read_variable(da, &[2, 3], None).unwrap(),
        ]
        .into();
        let result = vm.read_variable(result, &[0], None).unwrap();
        (report.instructions, values, result)
    }

    let optimized = run("0");
    let ordinary = run("VALUE");
    assert_eq!(optimized, ordinary);
    assert_eq!(optimized.1, [7, 0, 0].map(VmValue::Integer));
    assert_eq!(optimized.2, VmValue::Integer(4));
}

#[test]
fn literal_groupmatch_preserves_results_and_instruction_accounting() {
    fn run(candidates: &str) -> (u64, VmValue) {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\n#DIMS VALUE\n#DIMS CANDIDATE_A\n#DIMS CANDIDATE_B\n\
             VALUE '= \"keep\"\nCANDIDATE_A '= \"keep\"\nCANDIDATE_B '= \"other\"\n\
             RESULT:0 = GROUPMATCH(VALUE, {candidates})\n\
             RETURN RESULT\n"
        ));
        let entry = artifact.functions[0].key;
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .expect("RESULT")
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
        (
            report.instructions,
            vm.read_variable(result, &[0], None).unwrap(),
        )
    }

    let optimized = run("\"keep\", \"other\", \"keep\"");
    let ordinary = run("CANDIDATE_A, CANDIDATE_B, CANDIDATE_A");
    assert_eq!(optimized, ordinary);
    assert_eq!(optimized.1, VmValue::Integer(2));
}
