use super::*;
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
    fn run(calls: &str, profile: erabasic_compat::CompatibilityProfileId) -> (u64, Vec<VmValue>) {
        let selector = match profile {
            erabasic_compat::CompatibilityProfileId::EmueraEm => {
                "ISNUMERIC(ARGS) ? TOINT(ARGS) # LOOKUP_INDEX(\"DATA\", ARGS)"
            }
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake => {
                "LOOKUP_INDEX(\"DATA\", ARGS)"
            }
        };
        let source = format!(
            "@SYSTEM_TITLE\nDA:1:0 = 7\n{calls}\nRETURN RESULT\n\
             @READ_DATA, ARG, ARGS\n#FUNCTION\n#LOCALSIZE 1\n\
             LOCAL = {selector}\n\
             RETURNF DA:ARG:LOCAL\n\
             @LOOKUP_INDEX, ARGS, ARGS:1\n#FUNCTION\n\
             SIF ARGS:1 == \"FIELD\"\nRETURNF 0\nRETURNF 1\n"
        );
        let artifact = compile_source_with_options(
            &source,
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::default()
            },
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
        let values = (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect();
        (report.instructions, values)
    }

    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let once = run("RESULT:0 = READ_DATA(1, \"FIELD\")", profile);
        let repeated = run(
            "RESULT:0 = READ_DATA(1, \"FIELD\")\n\
         RESULT:1 = READ_DATA(1, \"FIELD\")\n\
         DA:1:0 = 9\nRESULT:2 = READ_DATA(1, \"FIELD\")",
            profile,
        );
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
