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
