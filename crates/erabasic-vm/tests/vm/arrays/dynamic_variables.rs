use super::*;
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
         #DIM INDEX\n\
         #DIMS WORDS = \"zero\", \"one\"\n\
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
         INDEX = 1\n\
         RESULTS:15 = %GETVARS(\"WORDS:INDEX\")%\n\
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
    let fixture =
        include_str!("../../../../../tools/runtime-tester/fixture-reference/erb/oracle.erb")
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
