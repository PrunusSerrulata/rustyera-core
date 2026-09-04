use super::*;
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
fn path_memo_observes_vm_owned_place_mutations() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         FLAG:0 = 4\nFLAG:1 = 9\n\
         RESULT:10 = MUTATE_PLACES(0)\n\
         FLAG:0 = 4\nFLAG:1 = 9\n\
         RESULT:11 = MUTATE_PLACES(0)\nRETURN RESULT\n\
         @MUTATE_PLACES, ARG\n#FUNCTION\n\
         SWAP FLAG:ARG, FLAG:(ARG + 1)\n\
         SETBIT FLAG:ARG, 1\n\
         RETURNF FLAG:ARG * 100 + FLAG:(ARG + 1)\n",
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
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
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
        [1104, 1104].map(VmValue::Integer)
    );
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(11)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(4)));
}

#[test]
fn path_memo_traces_dynamic_getters_and_replays_target_arguments() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         FLAG:0 = 3\n\
         RESULT:10 = DYNAMIC_GET(0)\n\
         RESULT:11 = DYNAMIC_GET(0)\n\
         FLAG:0 = 4\nRESULT:12 = DYNAMIC_GET(0)\n\
         CALLFORMF TARGET_0, 7\n\
         FLAG:0 = 3\nRESULT:13 = DYNAMIC_GET(0)\nRETURN RESULT\n\
         @DYNAMIC_GET, ARG\n#FUNCTION\n\
         CALLFORMF TARGET_{ARG}, ARG\nRETURNF RESULT\n\
         @TARGET_0, ARG\n#FUNCTION\n#DIM DYNAMIC TOTAL\n\
         FOR LOCAL, 0, 100\n\
         TOTAL += MAX(FLAG:ARG, STRCOUNT(\"aaa\", \"a\"))\n\
         NEXT\nRESULT = TOTAL\nRETURNF RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let target = artifact
        .functions
        .iter()
        .find(|function| function.name == "TARGET_0")
        .expect("TARGET_0")
        .key;
    let target_argument = artifact
        .globals
        .iter()
        .find(|global| global.owner == Some(target) && global.name == "ARG")
        .expect("TARGET_0 ARG")
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
        [300, 300, 400, 300].map(VmValue::Integer)
    );
    assert_eq!(
        vm.read_variable(target_argument, &[], None),
        Ok(VmValue::Integer(0)),
        "memo replay must restore a dynamic target's persistent argument"
    );
}

#[test]
fn registered_native_override_remains_a_dynamic_path_memo_boundary() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct CountingNative {
        calls: Arc<AtomicUsize>,
    }

    impl NativeService for CountingNative {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            let value = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(NativeReady::value(VmValue::Integer(
                i64::try_from(value).unwrap(),
            )))
        }

        fn requires_rollback_checkpoint(&self) -> bool {
            false
        }
    }

    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT:10 = DYNAMIC_GET(0)\n\
         RESULT:11 = DYNAMIC_GET(0)\nRETURN RESULT\n\
         @DYNAMIC_GET, ARG\n#FUNCTION\n\
         CALLFORMF TARGET_{ARG}\nRETURNF RESULT\n\
         @TARGET_0\n#FUNCTION\nRESULT = MAX(1, 0)\nRETURNF RESULT\n",
    );
    let max = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name == "max")
        .expect("MAX native")
        .import
        .key;
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
    let calls = Arc::new(AtomicUsize::new(0));
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    assert!(!natives.register(
        max,
        CountingNative {
            calls: Arc::clone(&calls),
        },
    ));
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
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [1, 2].map(VmValue::Integer)
    );
}

#[test]
fn dynamic_path_memo_rechecks_safe_natives_after_a_registry_override() {
    struct ConstantNative;

    impl NativeService for ConstantNative {
        fn call(
            &mut self,
            _request: NativeCallRequest,
        ) -> Result<NativeReady, erabasic_vm::ExecutionFailure> {
            Ok(NativeReady::value(VmValue::Integer(41)))
        }

        fn requires_rollback_checkpoint(&self) -> bool {
            false
        }
    }

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = DYNAMIC_GET(0)\nRETURN RESULT\n\
         @DYNAMIC_GET, ARG\n#FUNCTION\n\
         CALLFORMF TARGET_{ARG}\nRETURNF RESULT\n\
         @TARGET_0\n#FUNCTION\nRESULT = MAX(1, 0)\nRETURNF RESULT\n",
    );
    let max = artifact
        .native_imports
        .iter()
        .find(|native| native.import.name == "max")
        .expect("MAX native")
        .import
        .key;
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
    let first = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !first
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        first.events
    );
    assert_eq!(vm.read_variable(result, &[], None), Ok(VmValue::Integer(1)));

    assert!(!natives.register(max, ConstantNative));
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let second = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );

    assert!(
        !second
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        second.events
    );
    assert_eq!(
        vm.read_variable(result, &[], None),
        Ok(VmValue::Integer(41)),
        "a registry override installed between slices must invalidate the old replay candidate"
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
