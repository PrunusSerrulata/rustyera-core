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

struct ImportCheckingNative {
    expected: RuntimeImport,
}

impl NativeService for ImportCheckingNative {
    fn call(&mut self, request: NativeCallRequest) -> Result<NativeReady, String> {
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

#[test]
fn path_memo_validates_values_and_replays_persistent_split_state() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         FLAG:0 = 10\nRESULT = 7\n\
         RESULT:10 = LOOKUP_PATH(\"A\", 0)\n\
         RESULT:11 = LOOKUP_PATH(\"A\", 0)\n\
         FLAG:0 = 20\nRESULT:12 = LOOKUP_PATH(\"A\", 0)\n\
         FLAG:0 = 10\nRESULT = 9\nRESULT = 7\n\
         RESULT:13 = LOOKUP_PATH(\"A\", 0)\nRETURN RESULT\n\
         @LOOKUP_PATH, ARGS, ARG\n#FUNCTION\n#LOCALSIZE 4\n#LOCALSSIZE 4\n\
         #DIM SAVED\nSAVED = RESULT\nVARSET LOCALS\n\
         SPLIT \"A/B\", \"/\", LOCALS\n\
         LOCAL = FINDELEMENT(LOCALS, ESCAPE(ARGS), 0, RESULT, 1)\n\
         RESULT = SAVED\nSIF ARG + LOCAL < 0\nTHROW invalid offset\n\
         RETURNF FLAG:ARG + LOCAL\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let lookup = artifact
        .functions
        .iter()
        .find(|function| function.name == "LOOKUP_PATH")
        .expect("LOOKUP_PATH")
        .key;
    let global = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.owner == Some(lookup) && global.name == name)
            .unwrap_or_else(|| panic!("LOOKUP_PATH {name}"))
            .key
    };
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

    assert_eq!(
        (10..=13)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [10, 10, 20, 10].map(VmValue::Integer)
    );
    assert_eq!(vm.read_variable(result, &[], None), Ok(VmValue::Integer(7)));
    assert_eq!(
        vm.read_variable(global("SAVED"), &[], None),
        Ok(VmValue::Integer(7))
    );
    assert_eq!(
        vm.read_variable(global("LOCAL"), &[], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(global("LOCALS"), &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["A", "B", "", ""].map(|value| VmValue::String(value.into()))
    );
}

#[test]
fn reset_new_game_does_not_replay_show_day_with_missing_show_week_statics() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Flag)
        .expect("FLAG name table")
        .lookup
        .insert("回想模式".into(), 0);
    let artifact = compile_source_with_data(
        r#"@SYSTEM_TITLE
RESULTS:0 '= SHOW_DAY()
RETURN

@EVENTFIRST
RESULTS:0 '= SHOW_DAY()
RETURN

@DAYS, ARG = -1
#FUNCTION
ARG = ARG == -1 ? DAY # ARG
RETURNF ARG % 30 + 1

@MONTH, ARG = -1
#FUNCTION
ARG = ARG == -1 ? DAY # ARG
RETURNF (ARG % 360) / 30 + 1

@YEAR, ARG = -1
#FUNCTION
ARG = ARG == -1 ? DAY # ARG
RETURNF ARG / 360 + 1

@SHOW_DAY, ARG = -1
#FUNCTIONS
ARG = ARG == -1 ? DAY # ARG
RETURNF FLAG:回想模式 ? @"第{DAY}日" # @"第{YEAR(ARG)}年 {MONTH(ARG),2}月 {DAYS(ARG),2}日 %SHOW_WEEK(ARG)%"

@WEEK, ARG = -1
#FUNCTION
ARG = ARG == -1 ? DAY # ARG
RETURNF ARG % 7 + 1

@SHOW_WEEK, ARG = -1
#FUNCTIONS
ARG = ARG == -1 ? DAY # ARG
SELECTCASE WEEK(ARG)
    CASE 1
        LOCALS = 周一
    CASE 2
        LOCALS = 周二
    CASE 3
        LOCALS = 周三
    CASE 4
        LOCALS = 周四
    CASE 5
        LOCALS = 周五
    CASE 6
        LOCALS = 周六
    CASE 7
        LOCALS = 周日
ENDSELECT
RETURNF LOCALS
"#,
        data,
    );
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());

    for entry_name in ["SYSTEM_TITLE", "EVENTFIRST"] {
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == entry_name)
            .unwrap_or_else(|| panic!("{entry_name}"))
            .key;
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
            "{entry_name}: {:#?}",
            report.events
        );
        assert_eq!(
            vm.read_variable(results, &[0], None),
            Ok(VmValue::String("第1年  1月  1日 周一".into()))
        );
        if entry_name == "SYSTEM_TITLE" {
            let prepared = vm
                .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
                .unwrap();
            vm.commit_runtime_state(prepared).unwrap();
        }
    }
}

#[test]
fn path_memo_find_element_queries_follow_array_revisions() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS:0 '= \"target\"\nRESULTS:1 '= \"\"\nRESULTS:2 '= \"target\"\n\
         RESULT:10 = LOOKUP_RANGE(\"target\")\n\
         RESULT:11 = LOOKUP_RANGE(\"target\")\n\
         RESULTS:0 '= \"\"\nRESULTS:1 '= \"target\"\nRESULTS:2 '= \"\"\n\
         RESULT:12 = LOOKUP_RANGE(\"target\")\n\
         RESULT:13 = LOOKUP_RANGE(\"target\")\nRETURN RESULT\n\
         @LOOKUP_RANGE, ARGS\n#FUNCTION\n#LOCALSSIZE 2\n\
         VARSET LOCALS\nSPLIT \"left/right\", \"/\", LOCALS\n\
         RETURNF FINDELEMENT(RESULTS, ESCAPE(ARGS), 0, 3, 1) * 10 + FINDLASTELEMENT(RESULTS, ESCAPE(ARGS), 0, 3, 1)\n",
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
        (10..=13)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [2, 2, 11, 11].map(VmValue::Integer)
    );
}

#[test]
fn path_memo_character_reads_follow_implicit_and_explicit_identities() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         ADDVOIDCHARA\n\
         CFLAG:0:1 = 10\nCFLAG:1:1 = 20\n\
         CSTR:0:0 '= \"needle\"\nCSTR:0:1 '= \"other\"\n\
         CSTR:1:0 '= \"other\"\nCSTR:1:1 '= \"needle\"\n\
         TARGET = 0\nRESULT:10 = READ_TARGET()\nRESULT:11 = FIND_TARGET(\"needle\")\n\
         RESULT:12 = READ_TARGET()\nRESULT:13 = FIND_TARGET(\"needle\")\n\
         TARGET = 1\nRESULT:14 = READ_TARGET()\nRESULT:15 = FIND_TARGET(\"needle\")\n\
         RESULT:16 = READ_TARGET()\nRESULT:17 = FIND_TARGET(\"needle\")\n\
         MASTER = 0\nRESULT:18 = READ_MASTER()\nRESULT:19 = READ_MASTER()\n\
         MASTER = 1\nRESULT:20 = READ_MASTER()\nRESULT:21 = READ_MASTER()\n\
         MASTER = 0\nASSI = 1\nCFLAG:1:5 = 30\n\
         RESULT:22 = WRITE_MASTER_READ_ASSI()\nCFLAG:1:5 = 40\n\
         RESULT:23 = WRITE_MASTER_READ_ASSI()\nRETURN RESULT\n\
         @READ_TARGET\n#FUNCTION\nRETURNF CFLAG:1\n\
         @FIND_TARGET, ARGS\n#FUNCTION\nRETURNF FINDELEMENT(CSTR, ESCAPE(ARGS), 0, 2, 1)\n\
         @READ_MASTER\n#FUNCTION\nRETURNF CFLAG:MASTER:1\n\
         @WRITE_MASTER_READ_ASSI\n#FUNCTION\nCFLAG:MASTER:5 = 9\nRETURNF CFLAG:ASSI:5\n",
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
        (10..=23)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [10, 0, 10, 0, 20, 1, 20, 1, 10, 10, 20, 20, 30, 40].map(VmValue::Integer)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn path_memo_replays_character_mutations_only_for_the_resolved_character() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         ADDVOIDCHARA\n\
         TARGET = 0\nRESULT:10 = WRITE_TARGET()\nCFLAG:0:1 = 0\nRESULT:11 = WRITE_TARGET()\n\
         TARGET = 1\nRESULT:12 = WRITE_TARGET()\nCFLAG:1:1 = 0\nRESULT:13 = WRITE_TARGET()\n\
         TARGET = 0\nRESULT:14 = FILL_TARGET()\nCFLAG:0:2 = 0\nCFLAG:0:3 = 0\nRESULT:15 = FILL_TARGET()\n\
         TARGET = 1\nRESULT:16 = FILL_TARGET()\nCFLAG:1:2 = 0\nCFLAG:1:3 = 0\nRESULT:17 = FILL_TARGET()\n\
         MASTER = 0\nRESULT:18 = WRITE_MASTER()\nCFLAG:0:4 = 0\nRESULT:19 = WRITE_MASTER()\n\
         MASTER = 1\nRESULT:20 = WRITE_MASTER()\nCFLAG:1:4 = 0\nRESULT:21 = WRITE_MASTER()\nRETURN RESULT\n\
         @WRITE_TARGET\n#FUNCTION\nCFLAG:1 = 7\nRETURNF 1\n\
         @FILL_TARGET\n#FUNCTION\nVARSET CFLAG:TARGET, 8, 2, 4\nRETURNF 1\n\
         @WRITE_MASTER\n#FUNCTION\nCFLAG:MASTER:4 = 9\nRETURNF 1\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let global = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .key
    };
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

    for character in 0..=1 {
        assert_eq!(
            vm.read_variable(global("CFLAG"), &[1], Some(character)),
            Ok(VmValue::Integer(7))
        );
        assert_eq!(
            (2..=3)
                .map(|index| {
                    vm.read_variable(global("CFLAG"), &[index], Some(character))
                        .unwrap()
                })
                .collect::<Vec<_>>(),
            [VmValue::Integer(8), VmValue::Integer(8)]
        );
        assert_eq!(
            vm.read_variable(global("CFLAG"), &[4], Some(character)),
            Ok(VmValue::Integer(9))
        );
    }
}

#[test]
fn path_memo_does_not_replay_a_trace_that_mutates_its_queried_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"target\"\nRESULTS:1 '= \"\"\n\
         RESULT:10 = FIND_THEN_MOVE(\"target\")\n\
         RESULT:11 = FIND_THEN_MOVE(\"target\")\nRETURN RESULT\n\
         @FIND_THEN_MOVE, ARGS\n#FUNCTION\n#DIM FOUND\n\
         FOUND = FINDELEMENT(RESULTS, ESCAPE(ARGS), 0, 2, 1)\n\
         RESULTS:0 '= \"\"\nRESULTS:1 '= \"target\"\nRETURNF FOUND\n",
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
        (10..=11)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [0, 1].map(VmValue::Integer)
    );
}

#[test]
fn faulting_path_is_never_replayed_from_a_successful_path_memo() {
    struct ThrowFaultHost;

    impl VmHost for ThrowFaultHost {
        fn call(&mut self, request: HostCallRequest) -> HostCallResult {
            if request.import.name.eq_ignore_ascii_case("THROW") {
                HostCallResult::Error("thrown from test".into())
            } else {
                HostCallResult::Ready(HostReady::empty())
            }
        }
    }

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 7\nFLAG:0 = 0\n\
         RESULT:10 = LOOKUP_FAULT(\"A\", 0)\n\
         RESULT:11 = LOOKUP_FAULT(\"A\", 0)\n\
         FLAG:0 = -1\nRESULT:12 = LOOKUP_FAULT(\"A\", 0)\nRETURN RESULT\n\
         @LOOKUP_FAULT, ARGS, ARG\n#FUNCTION\n#LOCALSIZE 4\n#LOCALSSIZE 4\n\
         #DIM SAVED\nSAVED = RESULT\nVARSET LOCALS\n\
         SPLIT \"A/B\", \"/\", LOCALS\n\
         LOCAL = FINDELEMENT(LOCALS, ESCAPE(ARGS), 0, RESULT, 1)\n\
         RESULT = SAVED\nSIF FLAG:0 + ARG + LOCAL < 0\nTHROW invalid offset\n\
         RETURNF LOCAL + ARG\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let lookup = artifact
        .functions
        .iter()
        .find(|function| function.name == "LOOKUP_FAULT")
        .expect("LOOKUP_FAULT")
        .key;
    let saved = artifact
        .globals
        .iter()
        .find(|global| global.owner == Some(lookup) && global.name == "SAVED")
        .expect("LOOKUP_FAULT SAVED")
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
    let report = vm.run_slice(&mut ThrowFaultHost, &mut natives, RunBudget::default());

    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.read_variable(result, &[], None), Ok(VmValue::Integer(7)));
    assert_eq!(vm.read_variable(saved, &[], None), Ok(VmValue::Integer(7)));
}
