use super::*;

#[test]
fn power_statement_writes_the_destination_instead_of_passing_its_place_as_an_operand() {
    let artifact = compile_source("@SYSTEM_TITLE\nPOWER RESULT, 2, 3\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(8));
}

#[test]
fn static_call_target_with_an_inline_comment_executes_in_the_vm() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nCALL TARGET; inline comment\nRETURN RESULT\n@TARGET\nRESULT = 42\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(42));
}

#[test]
fn scalar_ref_parameters_store_aliases_and_mutate_the_callers_arrays() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALL MUTATE_REF(VALUES)\nRETURN RESULT\n@MUTATE_REF(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
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
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn dynamic_calls_bind_variable_arguments_as_refs_or_values_from_the_target_signature() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALLFORM MUTATE_{1}(VALUES)\nCALLFORM READ_{1}(VALUES:1)\nRETURN RESULT\n@MUTATE_1(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN RESULT\n@READ_1(VALUE)\n#DIM VALUE\nRESULT:1 = VALUE\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
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
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn while_false_branch_skips_past_wend_and_finite_loops_terminate() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM ITERATIONS\nWHILE ITERATIONS < 3\nITERATIONS ++\nWEND\nWHILE 0\nITERATIONS = 99\nWEND\nRETURN ITERATIONS\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(3));
}

#[test]
fn bare_return_zeros_result_zero_and_preserves_the_remaining_result_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:1 = 7\nCALL HELPER\nRETURN (RESULT:0 == 0) && (RESULT:1 == 7)\n@HELPER\nRESULT = 99\nRETURN\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn logical_operators_short_circuit_their_right_operand() {
    for (expression, expected) in [
        ("1 || VALUES:1", 1),
        ("0 && VALUES:1", 0),
        ("0 !& VALUES:1", 1),
        ("1 !| VALUES:1", 0),
        ("1 && 7", 1),
        ("0 || 7", 1),
        ("1 !& 0", 1),
        ("0 !| 0", 1),
    ] {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\n#DIM VALUES, 1\nRETURN {expression}\n"
        ));
        assert_eq!(
            run_compiled_result(&artifact),
            VmValue::Integer(expected),
            "{expression}"
        );
    }
}

#[test]
fn rand_pseudo_variable_uses_the_random_native_instead_of_schema_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RAND:1\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));
}

#[test]
fn repeat_updates_count_and_continue_runs_rend_increment() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT = 0\n\
         REPEAT 4\n\
             SIF COUNT == 1\n\
                 CONTINUE\n\
             RESULT += COUNT + 1\n\
         REND\n\
         RESULT = RESULT * 10 + COUNT\n\
         RETURN RESULT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(84));
}

#[test]
fn break_advances_repeat_count_before_leaving_the_loop() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT = 0\n\
         REPEAT 5\n\
             RESULT += 10\n\
             BREAK\n\
         REND\n\
         RESULT += COUNT\n\
         RETURN RESULT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(11));
}

#[test]
fn selectcase_loop_rejects_the_previous_string_tip() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS '= GET_TIP(0)\n\
         RESULT = RESULTS != \"zero\" && RESULTS != \"\"\n\
         RETURN RESULT\n\
         @GET_TIP(ARG)\n\
         #FUNCTIONS\n\
         #DIM INDEX\n\
         #DIMS TEXT\n\
         DO\n\
             SELECTCASE INDEX\n\
                 CASE 0\n\
                     TEXT = \"zero\"\n\
                 CASEELSE\n\
                     TEXT = \"one\"\n\
             ENDSELECT\n\
             INDEX += 1\n\
         LOOP TEXT == \"zero\"\n\
         RETURNF TEXT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn structured_map_native_preserves_order_and_commits_array_outputs() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS KEYS, 4\nRESULT:0 = MAP_CREATE(\"m\")\nRESULT:1 = MAP_SET(\"m\", \"b\", \"1\")\nRESULT:2 = MAP_SET(\"m\", \"a\", \"2\")\nRESULT:3 = MAP_SET(\"m\", \"b\", \"3\")\nRESULTS:0 = %MAP_GET(\"m\", \"b\")%\nRESULTS:1 = %MAP_GETKEYS(\"m\")%\nRESULTS:2 = %MAP_GETKEYS(\"m\", KEYS, 1)%\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let keys = artifact
        .globals
        .iter()
        .find(|global| global.name == "KEYS")
        .expect("KEYS")
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("3".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("b,a".into()))
    );
    assert_eq!(
        vm.read_variable(keys, &[0], None),
        Ok(VmValue::String("b".into()))
    );
    assert_eq!(
        vm.read_variable(keys, &[1], None),
        Ok(VmValue::String("a".into()))
    );
}

#[test]
fn structured_data_table_uses_deterministic_ids_and_updates_rows() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = DT_CREATE(\"t\")\nRESULT:1 = DT_COLUMN_ADD(\"t\", \"score\", \"int32\", 0)\nRESULT:2 = DT_ROW_ADD(\"t\", \"score\", 7)\nRESULT:3 = DT_ROW_SET(\"t\", RESULT:2, \"score\", 9)\nRESULT:4 = DT_CELL_GET(\"t\", RESULT:2, \"score\", 1)\nRETURN RESULT\n",
    );
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
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[4], None),
        Ok(VmValue::Integer(9))
    );
}

#[test]
fn structured_data_table_treats_omitted_values_as_null_cells() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nDT_CREATE \"t\"\nDT_COLUMN_ADD \"t\", \"name\", \"string\"\nDT_COLUMN_ADD \"t\", \"score\", \"int32\"\nDT_ROW_ADD \"t\", \"name\",, \"score\",\nRESULT:0 = RESULT\nRESULT:1 = DT_CELL_ISNULL(\"t\", RESULT:0, \"name\", 1)\nRESULT:2 = DT_CELL_ISNULL(\"t\", RESULT:0, \"score\", 1)\nRESULT:3 = DT_CELL_SET(\"t\", RESULT:0, \"name\", \"filled\", 1)\nRESULT:4 = DT_CELL_SET(\"t\", RESULT:0, \"name\",, 1)\nRESULT:5 = DT_CELL_ISNULL(\"t\", RESULT:0, \"name\", 1)\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT" && global.owner.is_none())
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
        (0..=5)
            .map(|index| vm.read_variable(result, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(1); 6]
    );
}

#[test]
fn structured_xml_mutations_match_the_reference_fixture_subset() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = XML_DOCUMENT(1, \"<root><item id='a'>one</item><item id='b'>two</item></root>\")\nRESULTS:0 = %XML_TOSTR(1)%\nRESULT:1 = XML_SET(RESULTS:0, \"//item[@id='b']\", \"changed\", 0, 1)\nRESULT:2 = XML_ADDATTRIBUTE(RESULTS:0, \"//item[@id='a']\", \"kind\", \"first\")\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
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
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String(
            "<root><item id=\"a\" kind=\"first\">one</item><item id=\"b\">changed</item></root>"
                .into()
        ))
    );
}

#[test]
fn xml_get_instruction_writes_selected_nodes_to_a_local_string_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nCALL LOAD_ITEMS\nRETURN RESULT\n@LOAD_ITEMS\n#DIMS DYNAMIC ITEMS, 4\nXML_DOCUMENT 1, \"<root><item id='a'>one</item><item id='b'>two</item></root>\"\nXML_GET 1, \"//item\", ITEMS, 3\nRESULTS:10 '= ITEMS:0\nRESULTS:11 '= ITEMS:1\nRETURN RESULT\n",
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
        (10..=11)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::String("<item id=\"a\">one</item>".into()),
            VmValue::String("<item id=\"b\">two</item>".into()),
        ]
    );
}

#[test]
fn structured_xml_descendant_axes_include_root_elements_and_attributes() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS VALUES, 4\nRESULT:0 = XML_DOCUMENT(1, \"<unicodeIcon unicode='A'><layer unicode='B'/></unicodeIcon>\")\nRESULT:1 = XML_GET(1, \"//@unicode\", VALUES, 1)\nRESULTS:10 '= VALUES:0\nRESULTS:11 '= VALUES:1\nRESULT:2 = XML_GET(\"<enemy_data name='rabbit'/>\", \"//enemy_data/@name\", VALUES, 1)\nRESULTS:12 '= VALUES:0\nRESULT:3 = XML_GET(\"<rooted code='ok'/>\", \"rooted/@code\", VALUES, 1)\nRESULTS:13 '= VALUES:0\nRETURN RESULT\n",
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
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        (10..=13)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["A", "B", "rabbit", "ok"]
            .map(|value| VmValue::String(value.into()))
            .to_vec()
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
fn regexpmatch_supports_positive_boundaries_without_consuming_adjacent_tokens() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM GROUP_COUNT\n#DIMS MATCHES, 4\nRESULT:0 = REGEXPMATCH(\"[$TOKEN:A][$TOKEN:B]\", \"(?<=\\\\[\\\\$TOKEN:).*?(?=\\\\])\", GROUP_COUNT, MATCHES)\nRESULT:1 = GROUP_COUNT\nRESULTS:10 '= MATCHES:0\nRESULTS:11 '= MATCHES:1\nRETURN RESULT\n",
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["A", "B"]
            .map(|value| VmValue::String(value.into()))
            .to_vec()
    );
}

#[test]
fn one_dimensional_array_operations_accept_an_indexed_reference() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RELATION:0:0 = 4\n\
         RELATION:0:1 = 8\n\
         RELATION:0:2 = 12\n\
         RELATION:0:3 = 16\n\
         RESULT:0 = FINDELEMENT(RELATION:0:0, 12, 0, 4)\n\
         ARRAYREMOVE RELATION:0:0, 1, 1\n\
         RESULT:1 = RELATION:0:1\n\
         RETURN RESULT\n",
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
        vm.read_variable(result, &[0], None).unwrap(),
        VmValue::Integer(2)
    );
    assert_eq!(
        vm.read_variable(result, &[1], None).unwrap(),
        VmValue::Integer(12)
    );
}

#[test]
fn conditional_form_trims_branch_edge_whitespace() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS:0 = \\@ 0 ? unused # %\"魔力\"% \\@\n\
         RESULTS:1 = \\@ 1 ? \tkept\t # unused \\@\n\
         RETURN RESULT\n",
    );
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
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
    assert_eq!(
        vm.read_variable(results, &[0], None).unwrap(),
        VmValue::String("魔力".into())
    );
    assert_eq!(
        vm.read_variable(results, &[1], None).unwrap(),
        VmValue::String("kept".into())
    );
}
