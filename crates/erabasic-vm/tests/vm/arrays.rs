use super::*;

#[test]
fn arraysort_accepts_reference_forward_back_keywords() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 2\nFLAG:1 = 4\nFLAG:2 = 1\nFLAG:3 = 3\nARRAYSORT FLAG, BACK, 0, 4\nARRAYCOPY FLAG, FLAG\nRETURN RESULT\n",
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
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(4),
            VmValue::Integer(3),
            VmValue::Integer(2),
            VmValue::Integer(1),
        ]
    );
}

#[test]
fn arraycopy_resolves_runtime_variable_names_and_array_queries_keep_places() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 3\nARRAYCOPY \"FLAG\", \"FLAG\"\nRESULT:0 = SUMARRAY(FLAG, 0, 3)\nRESULT:1 = MATCH(FLAG, 3, 0, 3)\nRESULT:2 = INRANGEARRAY(FLAG, 2, 3, 0, 3)\nRESULT:3 = GROUPMATCH(3, FLAG:0, FLAG:1, FLAG:2)\nRETURN RESULT\n",
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
        (0..4)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(7),
            VmValue::Integer(2),
            VmValue::Integer(2),
            VmValue::Integer(2),
        ]
    );
}

#[test]
fn arraycopy_copies_the_shared_extent_when_array_lengths_differ() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM TARGET_LIST, 3\nTARGET:0 = 7\nTARGET:1 = 8\nTARGET:2 = 9\nTARGET:3 = 10\nARRAYCOPY \"TARGET\", \"TARGET_LIST\"\nRESULT:0 = TARGET_LIST:0\nRESULT:1 = TARGET_LIST:1\nRESULT:2 = TARGET_LIST:2\nRETURN RESULT\n",
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
        (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(7),
            VmValue::Integer(8),
            VmValue::Integer(9),
        ]
    );
}

#[test]
fn arraycopy_variable_names_prefer_the_active_function_local() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS DYNAMIC DOCS, 3\nDOCS:0 '= \"caller\"\nRESULTS:0 '= \"first\"\nRESULTS:1 '= \"second\"\nCALL COPY_RESULTS\nRESULTS:12 '= DOCS:0\nRETURN RESULT\n@COPY_RESULTS\n#DIMS DYNAMIC DOCS, 3\nARRAYCOPY \"RESULTS\", \"DOCS\"\nRESULTS:10 '= DOCS:0\nRESULTS:11 '= DOCS:1\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS" && global.owner.is_none())
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
    assert_eq!(
        (10..=12)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::String("first".into()),
            VmValue::String("second".into()),
            VmValue::String("caller".into()),
        ]
    );
}

#[test]
fn arraycopy_intersects_each_dimension_and_preserves_other_destination_cells() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM SOURCE_VALUES, 2, 3\n#DIM DESTINATION_VALUES, 3, 2\nSOURCE_VALUES:0:0 = 1\nSOURCE_VALUES:0:1 = 2\nSOURCE_VALUES:0:2 = 3\nSOURCE_VALUES:1:0 = 4\nSOURCE_VALUES:1:1 = 5\nSOURCE_VALUES:1:2 = 6\nDESTINATION_VALUES:2:0 = 9\nDESTINATION_VALUES:2:1 = 9\nARRAYCOPY \"SOURCE_VALUES\", \"DESTINATION_VALUES\"\nRESULT:10 = DESTINATION_VALUES:0:0\nRESULT:11 = DESTINATION_VALUES:0:1\nRESULT:12 = DESTINATION_VALUES:1:0\nRESULT:13 = DESTINATION_VALUES:1:1\nRESULT:14 = DESTINATION_VALUES:2:0\nRESULT:15 = DESTINATION_VALUES:2:1\nRETURN RESULT\n",
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
        (0..6)
            .map(|index| vm.read_variable(result, &[index + 10], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(1),
            VmValue::Integer(2),
            VmValue::Integer(4),
            VmValue::Integer(5),
            VmValue::Integer(9),
            VmValue::Integer(9),
        ]
    );
}

#[test]
fn printsingleforms_expands_a_constant_template_in_the_current_function_scope() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nCALL DRAW_INFORMATIONLINE, \"地图\"\nRETURN RESULT\n@DRAW_INFORMATIONLINE(ARGS)\n#DIMS EQUAL\nEQUAL = =\nPRINTSINGLEFORMS \"== %ARGS% \" + \"%(EQUAL * 3)%\"\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(host.strings, vec!["== 地图 ==="]);
}

#[test]
fn arraymsort_reorders_complete_rows_before_committing() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nRESULT:0 = 30\nRESULT:1 = 10\nRESULT:2 = 20\nRESULT:9 = ARRAYMSORT(FLAG, RESULT)\nRETURN RESULT\n",
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
        (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
    assert_eq!(
        vm.read_variable(result, &[9], None),
        Ok(VmValue::Integer(1))
    );
}

#[test]
fn arraymsortex_resolves_target_names_at_runtime() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nTFLAG:0 = 30\nTFLAG:1 = 10\nTFLAG:2 = 20\nRESULTS:0 '= \"TFLAG\"\nRESULTS:1 '= \"\"\nRESULT:9 = ARRAYMSORTEX(FLAG, RESULTS, 1, -1)\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let tflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
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
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(tflag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
}

#[test]
fn arraymsortex_rolls_back_when_a_later_dynamic_target_is_invalid() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 3\nFLAG:1 = 1\nFLAG:2 = 2\nFLAG:3 = 0\nTFLAG:0 = 30\nTFLAG:1 = 10\nTFLAG:2 = 20\nRESULTS:0 '= \"TFLAG\"\nRESULTS:1 '= \"MISSING\"\nRESULT:9 = ARRAYMSORTEX(FLAG, RESULTS, 1, -1)\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let tflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
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
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("MISSING")
    )));
    assert_eq!(
        (0..3)
            .map(|index| vm.read_variable(tflag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(30),
            VmValue::Integer(10),
            VmValue::Integer(20),
        ]
    );
}

#[test]
fn character_mutations_commit_as_one_memory_transaction() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDCOPYCHARA 0\nSWAPCHARA 0, 1\nDELCHARA 1\nRESULT = CHARANUM\nRETURN RESULT\n",
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|import| import.import.name.eq_ignore_ascii_case("ADDVOIDCHARA")),
        "{:#?}",
        artifact.native_imports
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
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
}

#[test]
fn varset_fills_only_the_validated_half_open_range() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nFLAG:0 = 1\nFLAG:1 = 2\nFLAG:2 = 3\nFLAG:3 = 4\nVARSET FLAG, 9, 1, 3\nRESULTS:0 '= \"a\"\nRESULTS:1 '= \"b\"\nRESULTS:2 '= \"c\"\nRESULTS:3 '= \"d\"\nVARSET RESULTS, \"x\", 3, 1\nRESULT = FLAG:1\nRETURN RESULT\n",
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
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(1),
            VmValue::Integer(9),
            VmValue::Integer(9),
            VmValue::Integer(4),
        ]
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(9))
    );
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::String("a".into()),
            VmValue::String("x".into()),
            VmValue::String("x".into()),
            VmValue::String("d".into()),
        ]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn omitted_varset_values_use_the_destination_type_default() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIMS WORDS, 2, 2\n\
         FLAG:0 = 7\n\
         FLAG:1 = 8\n\
         VARSET FLAG,\n\
         FLAG:5 = 9\n\
         FLAG:6 = 10\n\
         VARSET FLAG:5, 0\n\
         RESULTS:0 '= \"a\"\n\
         RESULTS:1 '= \"b\"\n\
         RESULTS:2 '= \"c\"\n\
         RESULTS:3 '= \"d\"\n\
         VARSET RESULTS,, 2, 4\n\
         VARSET RESULTS, 0, 0, 2\n\
         ADDVOIDCHARA\n\
         TARGET = 0\n\
         CSTR:0 '= \"first\"\n\
         CSTR:1 '= \"first-extra\"\n\
         TARGET = 1\n\
         CSTR:0 '= \"second\"\n\
         CSTR:1 '= \"second-extra\"\n\
         CVARSET CSTR, 0,\n\
         CVARSET CSTR, 1, 0\n\
         WORDS:0:0 '= \"zero\"\n\
         WORDS:0:1 '= \"one\"\n\
         WORDS:1:0 '= \"two\"\n\
         WORDS:1:1 '= \"three\"\n\
         VARSET WORDS, 0\n\
         RETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let cstr = artifact
        .globals
        .iter()
        .find(|global| global.name == "CSTR")
        .unwrap()
        .key;
    let words = artifact
        .globals
        .iter()
        .find(|global| global.name == "WORDS")
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
    assert_eq!(
        (0..2)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(0); 2]
    );
    assert_eq!(
        (5..7)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(0); 2]
    );
    assert_eq!(
        (0..4)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::String(String::new()); 4]
    );
    assert_eq!(
        [(0, 0), (0, 1), (1, 0), (1, 1)]
            .into_iter()
            .map(|(character, index)| {
                vm.read_variable(cstr, &[index], Some(character)).unwrap()
            })
            .collect::<Vec<_>>(),
        vec![VmValue::String(String::new()); 4]
    );
    assert_eq!(
        [(0, 0), (0, 1), (1, 0), (1, 1)]
            .into_iter()
            .map(|(first, second)| { vm.read_variable(words, &[first, second], None).unwrap() })
            .collect::<Vec<_>>(),
        vec![VmValue::String(String::new()); 4]
    );
}

#[test]
fn dynamic_variable_methods_resolve_local_global_and_named_places() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Savestr)
        .unwrap()
        .lookup
        .insert("portrait".into(), 3);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         #DIMS HTMLS\n\
         SETVAR \"HTMLS\", \"local\"\n\
         RESULTS:10 = %HTMLS%\n\
         SETVAR \"SAVESTR:portrait\", \"saved\"\n\
         RESULTS:11 = %SAVESTR:portrait%\n\
         RESULT:10 = SETVAR(\"FLAG:4\", 9)\n\
         RESULT:11 = GETVAR(\"FLAG:4\")\n\
         RESULT:12 = COLOR_FROMNAME(\"LightSalmon\")\n\
         RESULT:13 = COLOR_FROMNAME(\"not-a-color\")\n\
         RESULTS:13 = %GETVARS(\"HTMLS\")%\n\
         RESULTS:14 = %GETVARS(\"SAVESTR:portrait\")%\n\
         RETURN RESULT\n",
        data,
    );
    let entry = artifact.functions[0].key;
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
    let savestr = artifact
        .globals
        .iter()
        .find(|global| global.name == "SAVESTR")
        .unwrap()
        .key;
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
    assert_eq!(
        (10..=13)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(1),
            VmValue::Integer(9),
            VmValue::Integer(0x00ff_a07a),
            VmValue::Integer(-1),
        ]
    );
    assert_eq!(vm.read_variable(flag, &[4], None), Ok(VmValue::Integer(9)));
    assert_eq!(
        vm.read_variable(savestr, &[3], None),
        Ok(VmValue::String("saved".into()))
    );
    assert_eq!(
        [10, 11, 13, 14]
            .into_iter()
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::String("local".into()),
            VmValue::String("saved".into()),
            VmValue::String("local".into()),
            VmValue::String("saved".into()),
        ]
    );
}

#[test]
fn varsetex_fills_the_selected_or_all_final_dimension_ranges() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM GRID, 2, 3\n\
         #DIMS WORDS, 2, 3\n\
         RESULT:0 = VARSETEX(\"GRID:1:1\", 7, 0)\n\
         RESULT:1 = VARSETEX(\"GRID:0:1\", 9)\n\
         RESULT:2 = VARSETEX(\"WORDS:1:0\", \"leaf\", 0)\n\
         RETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let key = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
            .key
    };
    let result = key("RESULT");
    let grid = key("GRID");
    let words = key("WORDS");
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
        (0..3)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(1); 3]
    );
    assert_eq!(
        [[0, 0], [0, 1], [0, 2], [1, 0], [1, 1], [1, 2]]
            .map(|indices| vm.read_variable(grid, &indices, None).unwrap()),
        [
            VmValue::Integer(0),
            VmValue::Integer(9),
            VmValue::Integer(9),
            VmValue::Integer(0),
            VmValue::Integer(9),
            VmValue::Integer(9),
        ]
    );
    assert_eq!(
        [[0, 0], [0, 1], [0, 2], [1, 0], [1, 1], [1, 2]]
            .map(|indices| vm.read_variable(words, &indices, None).unwrap()),
        [
            VmValue::String(String::new()),
            VmValue::String(String::new()),
            VmValue::String(String::new()),
            VmValue::String("leaf".into()),
            VmValue::String("leaf".into()),
            VmValue::String("leaf".into()),
        ]
    );
}

#[test]
fn dynamic_variable_methods_resolve_character_named_indices() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Palam)
        .unwrap()
        .lookup
        .insert("快Ｃ".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nSETVAR \"CUP:0:快Ｃ\", 8\nRESULT = GETVAR(\"CUP:0:快Ｃ\")\nRETURN RESULT\n",
        data,
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(8));
}

#[test]
fn omitted_substring_and_statement_encodetouni_match_reference_results() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS:12 = %SUBSTRING(\"abcd\", , 2)%\n\
         RESULTS:13 = %SUBSTRING(\"abcd\", 2, -1)%\n\
         RESULTS:14 = %SUBSTRINGU(\"aβcd\", 2, -1)%\n\
         RESULTS:20 '= \"A界\"\n\
         ENCODETOUNI %RESULTS:20%\n\
         RETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
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
        (0..=2)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(2),
            VmValue::Integer(65),
            VmValue::Integer(30_028),
        ]
    );
    assert_eq!(
        vm.read_variable(results, &[12], None),
        Ok(VmValue::String("ab".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[13], None),
        Ok(VmValue::String("cd".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[14], None),
        Ok(VmValue::String("cd".into()))
    );
}

#[test]
fn dynamic_variable_string_oracle_matches_reference_fixture() {
    let source = "@ORACLE_DYNAMIC_VARIABLES\n\
         #DIMS HTMLS\n\
         SETVAR \"HTMLS\", \"local\"\n\
         RESULTS:50 = %GETVARS(\"HTMLS\")%\n\
         SETVAR \"SAVESTR:0\", \"saved\"\n\
         RESULTS:51 = %GETVARS(\"SAVESTR:0\")%\n\
         RESULT:50 = SETVAR(\"FLAG:4\", 9)\n\
         RESULT:51 = GETVAR(\"FLAG:4\")\n\
         RESULT:52 = COLOR_FROMNAME(\"LightSalmon\")\n\
         RESULT:53 = COLOR_FROMNAME(\"not-a-color\")\n\
         RESULTS:52 = %SUBSTRING(\"abcd\", , 2)%\n\
         ENCODETOUNI A界\n\
         RESULT:54 = RESULT:0\n\
         RETURN\n";
    let fixture = include_str!("../../../../tools/runtime-tester/fixture-reference/erb/oracle.erb")
        .replace("\r\n", "\n");
    assert!(
        fixture.contains(source),
        "Rust and reference oracle bodies differ"
    );
    let artifact = compile_source(source);
    let key = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
            .key
    };
    let result = key("RESULT");
    let results = key("RESULTS");
    let flag = key("FLAG");
    let savestr = key("SAVESTR");
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(artifact.functions[0].key, Vec::new())
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
        "{:#?}",
        report.events
    );
    assert_eq!(
        [1, 2, 50, 51, 52, 53, 54]
            .into_iter()
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [65, 30_028, 1, 9, 16_752_762, -1, 2]
            .into_iter()
            .map(VmValue::Integer)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        (50..=52)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["local", "saved", "ab"]
            .into_iter()
            .map(|value| VmValue::String(value.into()))
            .collect::<Vec<_>>()
    );
    assert_eq!(vm.read_variable(flag, &[4], None), Ok(VmValue::Integer(9)));
    assert_eq!(
        vm.read_variable(savestr, &[0], None),
        Ok(VmValue::String("saved".into()))
    );
}

fn bit_options() -> AnalyzerOptions {
    AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::default()
    }
}

fn bit_result(artifact: &BytecodeArtifact, vm: &Vm, index: u64) -> VmValue {
    let key = artifact
        .globals
        .iter()
        .find(|v| v.name == "RESULT" && v.owner.is_none())
        .unwrap()
        .key;
    vm.read_variable(key, &[index], None).unwrap()
}

#[test]
fn bit_operations_cover_word_boundaries_omission_and_method_results() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
#DIM WORDS, 2
RESULT:10 = BITSET(WORDS, 63, , 2)
RESULT:11 = WORDS:0
RESULT:12 = WORDS:1
RESULT:13 = BITGET(WORDS, 63)
BITTOGGLE WORDS, 64
RESULT:14 = RESULT
RESULT:15 = BITGET(WORDS, 64)
RESULT:16 = BITGET(WORDS, 128)
RESULT:17 = BITTOGGLE(WORDS, -1)
RESULT:18 = BITINDEXOFFIRST(WORDS, 1)
RESULT:19 = BITINDEXOFFIRST(WORDS)
RESULT:20 = BITSET(WORDS, -2, 1, 0)
RESULT:21 = BITSET(WORDS, -1, 1, 2)
RESULT:22 = BITGET(WORDS, 0)
RETURN
",
        &bit_options(),
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(artifact.functions[0].key, Vec::new())
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
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    let expected = [1, i64::MIN, 1, 1, 1, 0, -1, 0, 63, 0, 1, 1, 1];
    for (index, expected) in expected.into_iter().enumerate() {
        assert_eq!(
            bit_result(&artifact, &vm, index as u64 + 10),
            VmValue::Integer(expected),
            "slot {index}"
        );
    }
}

#[test]
fn bit_first_token_index_is_not_evaluated_and_ancestor_local_ref_is_retained() {
    for form in [false, true] {
        let expression = "BITSET(ITEMS:SIDE(), 64)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let source = format!(
            "@SYSTEM_TITLE\n#DIM WORDS, 2\nRESULT:10 = MUTATE(WORDS)\nRESULT:11 = WORDS:1\nRETURN\n@MUTATE(ITEMS)\n#FUNCTION\n#DIM REF ITEMS\nRETURNF {expression}\n@SIDE\n#FUNCTION\nFLAG:8 += 1\nRETURNF 999999\n"
        );
        let artifact = compile_source_with_options(&source, &bit_options());
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        vm.spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
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
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(bit_result(&artifact, &vm, 11), VmValue::Integer(1));
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            vm.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(0)
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)] // Corruptions share one suspended-call fixture and restore proof.
fn bit_tail_wait_snapshot_rejects_missing_forged_lease_before_host_rebind() {
    for form in [false, true] {
        let expression = "BITSET(FLAG, INDEX(), 1)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nRESULT:10 = {expression}\nRETURN\n@INDEX\n#FUNCTION\nFLAG:8 += 1\nINPUT\nRETURNF 64\n"
            ),
            &bit_options(),
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let fiber = vm
            .spawn_entry(
                artifact
                    .functions
                    .iter()
                    .find(|f| f.name == "SYSTEM_TITLE")
                    .unwrap()
                    .key,
                Vec::new(),
            )
            .unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|e| match e {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .expect("tail waits");
        let snapshot = vm.snapshot(&natives).unwrap();
        let original = serde_json::to_value(&snapshot).unwrap();
        for corruption in ["lease", "owner", "length"] {
            let mut corrupt = original.clone();
            let entries = corrupt["memory"]["array_leases"]["entries"]
                .as_object_mut()
                .unwrap();
            match corruption {
                "lease" => entries.clear(),
                "owner" => {
                    entries.values_mut().next().unwrap()["owner"]["frame"] =
                        serde_json::json!(999_999);
                }
                "length" => entries.values_mut().next().unwrap()["length"] = serde_json::json!(0),
                _ => unreachable!(),
            }
            let mut rejected_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let corrupt: VmSnapshot =
                serde_json::from_value(corrupt).expect("corruption must deserialize");
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    corrupt,
                    &mut rejected_host,
                    &mut NativeServiceRegistry::for_artifact(&artifact)
                )
                .is_err(),
                "{corruption}"
            );
            assert!(rejected_host.rebound.is_empty());
        }
        if !form {
            let mut corrupt = original.clone();
            let frame = &mut corrupt["fibers"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()["frames"][0];
            frame["bit_calls"] = serde_json::json!([]);
            assert!(
                Vm::restore_snapshot(
                    validated(&artifact),
                    VmConfig::default(),
                    serde_json::from_value(corrupt).unwrap(),
                    &mut host,
                    &mut NativeServiceRegistry::for_artifact(&artifact)
                )
                .is_err()
            );
        }
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
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
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            restored.read_variable(flag, &[1], None).unwrap(),
            VmValue::Integer(1)
        );
        assert_eq!(
            restored.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(1)
        );
        vm.cancel_fiber(fiber).unwrap();
        let cancelled = serde_json::to_value(vm.snapshot(&natives).unwrap()).unwrap();
        assert!(
            cancelled["memory"]["array_leases"]["entries"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn bit_opaque_wire_and_original_identity_are_rejected() {
    let base = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = BITSET(FLAG, 0)\nRETURN\n",
        &bit_options(),
    );
    for corruption in ["pop", "identity", "origin"] {
        let mut artifact = base.clone();
        if corruption == "identity" {
            artifact.manifest.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraEm,
            );
        } else {
            let function = &mut artifact.functions[0];
            let at = function
                .code
                .iter()
                .position(|op| Opcode::try_from(op.opcode) == Ok(Opcode::FinishBitCall))
                .unwrap();
            function.code[at] = if corruption == "pop" {
                erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new())
            } else {
                erabasic_bytecode::EncodedInstruction::new(
                    Opcode::FinishBitCall,
                    u32::try_from(at).unwrap().to_le_bytes().to_vec(),
                )
            };
        }
        artifact.refresh_ids().unwrap();
        assert!(
            validate_bytecode(
                artifact.clone().into_unvalidated(),
                &erabasic_compiler::runtime_native_validation_context(
                    &artifact,
                    &default_host_registry()
                )
            )
            .value
            .is_none(),
            "{corruption}"
        );
    }
}

#[test]
fn bit_candidate_reset_rejection_preserves_parent_backing_and_frames() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = BITSET(FLAG, INDEX())\nRETURN\n@INDEX\n#FUNCTION\nINPUT\nRETURNF 64\n",
        &bit_options(),
    );
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|e| matches!(e, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let before = live.encode_unrestricted_snapshot().unwrap();
    let candidate = live.fork_isolated().unwrap();
    assert!(matches!(
        candidate.prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    // A failed prepare leaves the candidate committable with exactly its inherited roots.
    live.commit_candidate_state(candidate.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    let ordinary = RuntimeVm::new(validated(&artifact), VmConfig::default())
        .fork_isolated()
        .unwrap();
    ordinary
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
        .unwrap();
}

#[test]
fn bit_unbound_ref_fails_before_index_side_effect_and_checker_continues() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM REF ITEMS
RESULT:10 = STRFORMCHECK("{BITGET(ITEMS, SIDE())}")
RESULT:11 = 7
RETURN
@SIDE
#FUNCTION
FLAG:8 += 1
RETURNF 0
"#,
        &bit_options(),
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
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
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(bit_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(bit_result(&artifact, &vm, 11), VmValue::Integer(7));
    let flag = artifact
        .globals
        .iter()
        .find(|v| v.name == "FLAG" && v.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(flag, &[8], None).unwrap(),
        VmValue::Integer(0)
    );
}

fn match_options() -> AnalyzerOptions {
    AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    }
}
fn run_match_source(source: &str) -> (BytecodeArtifact, Vm, erabasic_vm::VmRunReport) {
    let artifact = compile_source_with_options(source, &match_options());
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
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
    (artifact, vm, report)
}
fn match_result(artifact: &BytecodeArtifact, vm: &Vm, index: u64) -> VmValue {
    let result = artifact
        .globals
        .iter()
        .find(|v| v.name == "RESULT" && v.owner.is_none())
        .unwrap()
        .key;
    vm.read_variable(result, &[index], None).unwrap()
}

#[test]
fn matchall_orders_ranges_before_needle_and_never_evaluates_token_indices() {
    for form in [false, true] {
        let expression = "MATCHALL(FLAG:IGNORED(), NEEDLE(), BEG(), ENDING(), OUT:IGNORED())";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let source = format!(
            r"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 1
FLAG:0 = 7
FLAG:1 = 8
FLAG:2 = 7
RESULT:10 = {expression}
RESULT:11 = OUT:0
RESULT:12 = FLAG:4
RESULT:13 = FLAG:5
RETURN
@IGNORED
#FUNCTION
FLAG:5 += 1
RETURNF 0
@BEG
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 1
RETURNF 0
@ENDING
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 2
RETURNF 3
@NEEDLE
#FUNCTION
FLAG:4 = FLAG:4 * 10 + 3
RETURNF 7
"
        );
        let (artifact, vm, report) = run_match_source(&source);
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(
            (10..14)
                .map(|i| match_result(&artifact, &vm, i))
                .collect::<Vec<_>>(),
            vec![
                VmValue::Integer(2),
                VmValue::Integer(0),
                VmValue::Integer(123),
                VmValue::Integer(0)
            ]
        );
    }
}

#[test]
fn matchall_indexed_const_input_preserves_reference_restructure_failure() {
    let source = r"@SYSTEM_TITLE
#DIM CONST WORDS, 2 = 1, 2
RESULT:10 = MATCHALL(WORDS:0, 1)
RESULT:11 = 9
RETURN
";
    let artifact = compile_source_with_options(source, &match_options());
    let spec = artifact
        .functions
        .iter()
        .flat_map(|function| &function.code)
        .find(|instruction| {
            erabasic_bytecode::Opcode::try_from(instruction.opcode)
                == Ok(erabasic_bytecode::Opcode::BeginMatchCall)
        })
        .map(|instruction| erabasic_bytecode::MatchCallSpec::decode(&instruction.payload).unwrap())
        .expect("MATCHALL opener");
    assert!(spec.input_restructured_to_scalar);

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
        "{report:?}"
    );

    let (artifact, vm, report) = run_match_source(
        "@SYSTEM_TITLE\n#DIM CONST WORDS, 2 = 1, 2\nRESULT:10 = STRFORMCHECK(\"{MATCHALL(WORDS:0, 1)}\")\nRESULT:11 = 9\nRETURN\n",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(9));
}

#[test]
fn matchall_empty_range_still_evaluates_needle_but_reversed_range_does_not() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
RESULT:10 = MATCHALL(FLAG, NEEDLE(), 9, 9)
RESULT:11 = STRFORMCHECK("{MATCHALL(FLAG, NEEDLE(), -1, ENDING())}")
RESULT:12 = FLAG:8
RESULT:13 = FLAG:9
RETURN
@NEEDLE
#FUNCTION
FLAG:8 += 1
RETURNF 7
@ENDING
#FUNCTION
FLAG:9 += 1
RETURNF 2
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        (10..14)
            .map(|i| match_result(&artifact, &vm, i))
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(1),
            VmValue::Integer(1)
        ]
    );
}

#[test]
fn matchall_checks_output_only_on_matching_write_and_preserves_tail() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIMS DYNAMIC WRONG, 1
#DIM DYNAMIC OUT, 3
FLAG:0 = 7
FLAG:1 = 9
OUT:2 = 81
RESULT:10 = MATCHALL(FLAG, 4, 0, 2, WRONG)
RESULT:11 = STRFORMCHECK("{MATCHALL(FLAG, 7, 0, 2, WRONG)}")
MATCHALL FLAG, 7, 0, 2, OUT
RESULT:12 = RESULT
RESULT:13 = OUT:2
RETURN
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        (10..14)
            .map(|i| match_result(&artifact, &vm, i))
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(0),
            VmValue::Integer(0),
            VmValue::Integer(1),
            VmValue::Integer(81)
        ]
    );
}

#[test]
fn matchall_string_input_and_character_input_use_live_field_zero() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIMS DYNAMIC WORDS, 3
WORDS:0 = x
WORDS:1 = z
WORDS:2 = x
RESULT:10 = MATCHALL(WORDS, "x")
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
BASE:0:1 = 9
BASE:1:1 = 8
RESULT:11 = MATCHALL(BASE:0:1, 7)
RETURN
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(2));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(2));
}

#[test]
fn matchall_character_input_observes_prior_output_through_character_array_ref() {
    let (artifact, vm, report) = run_match_source(
        r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
RESULT:10 = SCAN(BASE:1)
RETURN
@SCAN(OUT)
#FUNCTION
#DIM REF OUT, 0
RETURNF MATCHALL(BASE, 7, 0, 2, OUT)
",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(1));
}

#[test]
fn matchallex_uses_exact_name_lookup_before_begin_side_effect() {
    for ignore_case in [false, true] {
        let source = "@SYSTEM_TITLE\nFLAG:0 = 7\nRESULT:10 = STRFORMCHECK(\"{MATCHALLEX(\\\"flag\\\", 7, BEG(), 1)}\")\nRESULT:11 = FLAG:8\nRETURN\n@BEG\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n";
        let mut options = match_options();
        options.ignore_case = ignore_case;
        let artifact = compile_source_with_options(source, &options);
        assert_eq!(artifact.call_compatibility.ignore_case, ignore_case);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        vm.spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(
            match_result(&artifact, &vm, 10),
            VmValue::Integer(i64::from(ignore_case))
        );
        assert_eq!(
            match_result(&artifact, &vm, 11),
            VmValue::Integer(i64::from(ignore_case))
        );
    }
}

#[test]
fn matchall_bounded_chunks_block_stable_snapshot_and_keep_one_needle() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = MATCHALL(FLAG, NEEDLE(), 0, 600)\nRETURN\n@NEEDLE\n#FUNCTION\nFLAG:900 += 1\nRETURNF 0\n",
        &match_options(),
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let mut reached_chunk = false;
    for _ in 0..200 {
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget {
                maximum_instructions: 1,
                fiber_quantum: 1,
                ..RunBudget::default()
            },
        );
        assert!(
            report.instructions <= 1,
            "MATCH exceeded its slice budget: {report:?}"
        );
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        let saved = inspect_snapshot(
            &vm.encode_unrestricted_snapshot(&natives).unwrap(),
            VmConfig::default().maximum_snapshot_bytes,
        )
        .unwrap()
        .state;
        let active = saved["fibers"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|fiber| fiber["frames"].as_array().unwrap())
            .flat_map(|frame| frame["match_calls"].as_array().unwrap())
            .any(|call| call["state"]["needle"].is_object());
        if active {
            assert!(
                vm.snapshot(&natives).is_err(),
                "a runnable scan is not a stable snapshot point"
            );
            reached_chunk = true;
            break;
        }
    }
    assert!(
        reached_chunk,
        "large MATCH must yield a bounded scanner chunk"
    );
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(600));
    let flag = artifact
        .globals
        .iter()
        .find(|v| v.name == "FLAG" && v.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(flag, &[900], None).unwrap(),
        VmValue::Integer(1)
    );
}

#[test]
fn matchall_caught_late_bounds_error_keeps_previous_output_writes() {
    let (artifact, vm, report) = run_match_source(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 3
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
OUT:0 = 99
RESULT:10 = STRFORMCHECK("{MATCHALL(BASE, REMOVE_LAST(), 0, , OUT)}")
RESULT:11 = OUT:0
RETURN
@REMOVE_LAST
#FUNCTION
DELCHARA 1
RETURNF 7
"#,
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
    assert_eq!(match_result(&artifact, &vm, 11), VmValue::Integer(0));
}

#[test]
#[allow(clippy::too_many_lines)] // Each corruption is checked against the same suspended needle.
fn matchall_needle_can_wait_and_restore_without_repeating_side_effects() {
    for form in [false, true] {
        let expression = "MATCHALL(FLAG, NEEDLE(), 0, 2)";
        let expression = if form {
            format!("TOINT(STRFORM(\"{{{expression}}}\"))")
        } else {
            expression.into()
        };
        let artifact = compile_source_with_options(
            &format!(
                "@SYSTEM_TITLE\nFLAG:0 = 7\nFLAG:1 = 7\nRESULT:10 = {expression}\nRETURN\n@NEEDLE\n#FUNCTION\nFLAG:8 += 1\nINPUT\nRETURNF 7\n"
            ),
            &match_options(),
        );
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        vm.spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let request = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::HostPending { request, .. } => Some(*request),
                _ => None,
            })
            .expect("needle waits");
        let snapshot = vm.snapshot(&natives).unwrap();
        if !form {
            let original = serde_json::to_value(&snapshot).unwrap();
            for corruption in ["needle", "cursor", "list", "owner"] {
                let mut corrupt = original.clone();
                let frames = corrupt["fibers"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                    .next()
                    .unwrap()["frames"]
                    .as_array_mut()
                    .unwrap();
                let frame = &mut frames[0];
                match corruption {
                    "needle" => {
                        frame["match_calls"][0]["state"]["needle"] =
                            serde_json::to_value(VmValue::Integer(7)).unwrap();
                    }
                    "cursor" => frame["match_calls"][0]["state"]["cursor"] = serde_json::json!(1),
                    "list" => frame["match_calls"] = serde_json::json!([]),
                    "owner" => {
                        frame["match_calls"][0]["state"]["input"]["owner"] =
                            serde_json::json!(9999);
                    }
                    _ => unreachable!(),
                }
                assert!(
                    Vm::restore_snapshot(
                        validated(&artifact),
                        VmConfig::default(),
                        serde_json::from_value(corrupt).unwrap(),
                        &mut PendingHost {
                            stability: HostWaitStability::StableInput,
                            rebound: Vec::new()
                        },
                        &mut NativeServiceRegistry::for_artifact(&artifact)
                    )
                    .is_err(),
                    "{corruption}"
                );
            }
        }
        let mut restored = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            snapshot,
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
        assert!(
            !report
                .events
                .iter()
                .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
            "{report:?}"
        );
        assert_eq!(match_result(&artifact, &restored, 10), VmValue::Integer(2));
        let flag = artifact
            .globals
            .iter()
            .find(|v| v.name == "FLAG" && v.owner.is_none())
            .unwrap()
            .key;
        assert_eq!(
            restored.read_variable(flag, &[8], None).unwrap(),
            VmValue::Integer(1)
        );
    }
}

#[test]
fn match_wire_validation_rejects_phase_forgery_pop_and_original_identity() {
    let base = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = MATCHALL(FLAG, 0, 0, 3)\nRETURN\n",
        &match_options(),
    );
    for corruption in ["phase", "pop", "identity"] {
        let mut artifact = base.clone();
        if corruption == "identity" {
            artifact.manifest.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraEm,
            );
        } else {
            let function = artifact
                .functions
                .iter_mut()
                .find(|f| f.name == "SYSTEM_TITLE")
                .unwrap();
            let at = function
                .code
                .iter()
                .position(|op| Opcode::try_from(op.opcode) == Ok(Opcode::MatchCallRange))
                .unwrap();
            if corruption == "phase" {
                let mut payload = function.code[at].payload.to_vec();
                payload[4] = 1;
                function.code[at] =
                    erabasic_bytecode::EncodedInstruction::new(Opcode::MatchCallRange, payload);
            } else {
                // Neither ordinary values nor POP may consume the opaque token.
                function.code[at] =
                    erabasic_bytecode::EncodedInstruction::new(Opcode::Pop, Vec::new());
            }
        }
        artifact.refresh_ids().unwrap();
        let report = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &erabasic_compiler::runtime_native_validation_context(
                &artifact,
                &default_host_registry(),
            ),
        );
        assert!(report.value.is_none(), "{corruption}");
    }
}

#[test]
fn nested_bit_candidate_commit_preserves_inherited_roots_and_parent_atomicity() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nRESULT:10 = BITSET(FLAG, INDEX())\nRETURN\n@INDEX\n#FUNCTION\nINPUT\nRETURNF 64\n",
        &bit_options(),
    );
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let before = live.encode_unrestricted_snapshot().unwrap();
    let mut outer = live.fork_isolated().unwrap();
    let mut inner = outer.fork_isolated().unwrap();
    assert!(matches!(
        inner.prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame),
        Err(VmError::InvalidState(_))
    ));
    let save = inner.export_era_state();
    assert!(matches!(
        inner.restore_era_state(&save),
        Err(VmError::InvalidState(_))
    ));
    outer
        .commit_candidate_state(inner.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    live.commit_candidate_state(outer.into_candidate_state().unwrap())
        .unwrap();
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
}

#[test]
fn copychara_invalidates_prepared_candidate_even_when_cell_revisions_match() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
CFLAG:0:0 = 7
CFLAG:1:0 = 19
IF 0
CALL COPY_ROW
ENDIF
RESULT:10 = KEEP(CFLAG:1:0, WAIT_INDEX())
RETURN
@KEEP(VALUES, DUMMY)
#FUNCTION
#DIM REF VALUES
#DIM DUMMY
RETURNF VALUES:0
@WAIT_INDEX
#FUNCTION
INPUT
RETURNF 0
@COPY_ROW
COPYCHARA 0, 1
RETURN
",
        &bit_options(),
    );
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.spawn_entry(
        artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key,
        Vec::new(),
    )
    .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_))),
        "{report:?}"
    );
    let candidate = live
        .fork_isolated()
        .unwrap()
        .into_candidate_state()
        .unwrap();
    let copy = live
        .spawn_entry(
            artifact
                .functions
                .iter()
                .find(|f| f.name == "COPY_ROW")
                .unwrap()
                .key,
            Vec::new(),
        )
        .unwrap();
    let report = live.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::FiberCompleted(id, _) if *id == copy)),
        "{report:?}"
    );
    let current = live.encode_unrestricted_snapshot().unwrap();
    let flag = artifact
        .globals
        .iter()
        .find(|variable| variable.name == "CFLAG")
        .unwrap()
        .key;
    assert_eq!(
        live.read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: flag,
            indices: vec![0],
            character: Some(1)
        }])
        .unwrap(),
        vec![VmValue::Integer(7)]
    );
    assert!(matches!(
        live.commit_candidate_state(candidate),
        Err(VmError::InvalidState(_))
    ));
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), current);
}

#[test]
fn match_scan_zero_first_failure_and_partial_failure_charge_exact_work() {
    // This CHECK enters a user method so MATCH uses the bytecode scanner, not
    // RuntimeForm's already-single-row task. Range length is captured before
    // SHRINK executes. Row0 writes remain visible when a later row faults.
    for (end, remove, expected_check, expected_out) in [
        ("0", "DELALLCHARA", 1, 99),
        ("", "DELALLCHARA", 0, 99),
        ("", "DELCHARA 1", 0, 0),
    ] {
        let source = format!(
            r#"@SYSTEM_TITLE
#DIM DYNAMIC OUT, 3
DELALLCHARA
ADDVOIDCHARA
ADDVOIDCHARA
BASE:0:0 = 7
BASE:1:0 = 7
OUT:0 = 99
RESULT:10 = STRFORMCHECK("{{SCAN(OUT)}}")
RESULT:11 = OUT:0
RESULT:12 = 17
RETURN
@SCAN(OUTPUT)
#FUNCTION
#DIM REF OUTPUT
RETURNF MATCHALL(BASE, SHRINK(), 0, {end}, OUTPUT)
@SHRINK
#FUNCTION
{remove}
RETURNF 7
"#
        );
        let artifact = compile_source_with_options(&source, &match_options());
        let entry = artifact
            .functions
            .iter()
            .find(|f| f.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let mut totals = Vec::new();
        for maximum in [1, 128] {
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
            let mut total = 0;
            let mut completed = false;
            for _ in 0..2000 {
                let report = vm.run_slice(
                    &mut ReadyHost::default(),
                    &mut natives,
                    RunBudget {
                        maximum_instructions: maximum,
                        fiber_quantum: u32::try_from(maximum).unwrap(),
                        ..RunBudget::default()
                    },
                );
                assert!(report.instructions <= maximum, "{source}\n{report:?}");
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
                    "{source}\n{report:?}"
                );
                total += report.instructions;
                if matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))) {
                    completed = true;
                    break;
                }
            }
            assert!(
                completed,
                "CHECK must continue after a catchable scan error"
            );
            assert_eq!(
                match_result(&artifact, &vm, 10),
                VmValue::Integer(expected_check)
            );
            assert_eq!(
                match_result(&artifact, &vm, 11),
                VmValue::Integer(expected_out)
            );
            assert_eq!(match_result(&artifact, &vm, 12), VmValue::Integer(17));
            totals.push(total);
        }
        assert_eq!(
            totals[0], totals[1],
            "coalescing must count the same completed/failed rows: {source}"
        );
    }
}

// Fixed .NET 8 ICU72 name-casing regression candidate; not yet run.
// Expectations are source-derived for the .NET 8 ICU path, not captured oracle goldens.
#[test]
fn matchallex_unicode_name_casing_does_not_casefold_array_values() {
    for (declared, lookup, equal) in [
        ("ÉTAT", "état", true),
        ("ΣNAME", "ςname", true),
        ("ΜNAME", "µname", true),
        ("I_NAME", "ı_name", false),
        ("S_NAME", "ſ_name", false),
        ("K_NAME", "\u{212a}_name", false),
        ("ẞNAME", "ßname", false),
        ("ᾈNAME", "ᾀname", true),
        ("𐐀NAME", "𐐨name", true),
        ("ÉNAME", "E\u{301}name", false),
    ] {
        for ignore_case in [false, true] {
            let source = format!(
                "@SYSTEM_TITLE\n#DIMS DYNAMIC {declared}, 1\n{declared}:0 '= \"É\"\nRESULT:10 = STRFORMCHECK(\"{{MATCHALLEX(\\\"{lookup}\\\", \\\"é\\\", BEG(), 1)}}\")\nRESULT:11 = FLAG:8\nRETURN\n@BEG\n#FUNCTION\nFLAG:8 += 1\nRETURNF 0\n"
            );
            let mut options = match_options();
            options.ignore_case = ignore_case;
            let artifact = compile_source_with_options(&source, &options);
            let mut vm = Vm::new(validated(&artifact), VmConfig::default());
            vm.spawn_entry(
                artifact
                    .functions
                    .iter()
                    .find(|f| f.name == "SYSTEM_TITLE")
                    .unwrap()
                    .key,
                Vec::new(),
            )
            .unwrap();
            let mut natives = NativeServiceRegistry::for_artifact(&artifact);
            let report = vm.run_slice(
                &mut ReadyHost::default(),
                &mut natives,
                RunBudget::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            let name_exists = ignore_case && equal;
            assert_eq!(
                match_result(&artifact, &vm, 10),
                VmValue::Integer(i64::from(name_exists))
            );
            assert_eq!(
                match_result(&artifact, &vm, 11),
                VmValue::Integer(i64::from(name_exists))
            );
        }
    }
    // Successful lookup must still compare array string elements ordinally.
    let (artifact, vm, report) = run_match_source(
        "@SYSTEM_TITLE\n#DIMS DYNAMIC ÉTAT, 1\nÉTAT:0 '= \"É\"\nRESULT:10 = MATCHALLEX(\"état\", \"é\", 0, 1)\nRETURN\n",
    );
    assert!(
        !report
            .events
            .iter()
            .any(|e| matches!(e, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(match_result(&artifact, &vm, 10), VmValue::Integer(0));
}
