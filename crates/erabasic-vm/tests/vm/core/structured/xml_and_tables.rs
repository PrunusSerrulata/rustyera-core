use super::*;
#[test]
fn xml_addnode_clones_nested_content_for_every_matching_parent() {
    let source = r#"@SYSTEM_TITLE
#DIMS DOCUMENT
DOCUMENT '= "<root><group id='a'/><group id='b'/></root>"
XML_ADDNODE DOCUMENT, "//group", "<child><leaf>initial</leaf></child>", 0, 1
XML_SET DOCUMENT, "//group[@id='a']/child/leaf", "changed", 0, 1
RESULTS '= DOCUMENT
RETURN
"#;
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let artifact = compile_source_with_options(
            source,
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::default()
            },
        );
        assert_eq!(
            run_compiled_string_result(&artifact),
            VmValue::String("<root><group id=\"a\"><child><leaf>changed</leaf></child></group><group id=\"b\"><child><leaf>initial</leaf></child></group></root>".into()),
            "{profile}",
        );
    }
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
        "@SYSTEM_TITLE\nRESULT:0 = XML_DOCUMENT(1, \"<root><item id='a'>one</item><item id='b'>two</item></root>\")\nRESULTS:0 = %XML_TOSTR(1)%\nRESULT:1 = XML_SET(RESULTS:0, \"//item[@id='b']\", \"changed\", 0, 1)\nRESULT:2 = XML_ADDATTRIBUTE(RESULTS:0, \"//item[@id='a']\", \"kind\", \"first\")\nRESULT:3 = XML_ADDATTRIBUTE(RESULTS:0, \"//item[@id='a']\", \"kind\", \"last\")\nRETURN RESULT\n",
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
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String(
            "<root><item id=\"a\" kind=\"last\">one</item><item id=\"b\">changed</item></root>"
                .into()
        ))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn erafl_ui_attribute_pipeline_preserves_the_complete_division_rectangle() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         CALL UIC_ADD(\"[border:1px][rect:10,20,300,400]\")\n\
         CALL UIC_SET_DIVATTR(\"pos\", \"50,0\")\n\
         #DIMS DIVS, 2\n\
         XML_GET RESULTS:20, \"//containers[@id = '1']/container\", DIVS, 2\n\
         RESULTS:30 '= DIVS:0\n\
         RETURN\n\
         @UIC_ADD(DIC_ATTRIBUTES)\n\
         #DIMS DIC_ATTRIBUTES\n\
         #DIMS XML\n\
         #DIM KEY_NUM\n\
         #DIM I\n\
         #DIMS KEYS, 10\n\
         XML '= \"<layout><containers id='1'></containers></layout>\"\n\
         XML_ADDNODE XML, \"//containers[@id = '1']\", \"<container name='main'><div></div></container>\"\n\
         DIC_ATTRIBUTES = %TOLOWER(DIC_ATTRIBUTES)%\n\
         IF !DIC_CONTAINSKEY(DIC_ATTRIBUTES, \"xpos\")\n\
             DIC_ATTRIBUTES += \"[xpos:0]\"\n\
         ENDIF\n\
         IF !DIC_CONTAINSKEY(DIC_ATTRIBUTES, \"ypos\")\n\
             DIC_ATTRIBUTES += \"[ypos:0]\"\n\
         ENDIF\n\
         CALL DIC_KEYS(DIC_ATTRIBUTES)\n\
         KEY_NUM = RESULT\n\
         ARRAYCOPY \"RESULTS\", \"KEYS\"\n\
         RESULTS:20 '= XML\n\
         FOR I, 0, KEY_NUM\n\
             CALL UIC_SET_DIVATTR(TOLOWER(KEYS:I), TOLOWER(DIC_GET(DIC_ATTRIBUTES, KEYS:I)))\n\
         NEXT\n\
         RETURN\n\
         @UIC_SET_DIVATTR(ATTRIBUTE_NAME, ATTRIBUTE_VALUE)\n\
         #DIMS ATTRIBUTE_NAME\n\
         #DIMS ATTRIBUTE_VALUE\n\
         #DIMS POS_VALUES, 4\n\
         SELECTCASE ATTRIBUTE_NAME\n\
             CASE \"width\", \"height\", \"xpos\", \"ypos\", \"size\", \"rect\", \"pos\"\n\
             CASE \"depth\", \"color\", \"display\"\n\
             CASE \"margin\", \"padding\", \"border\", \"bcolor\", \"radius\"\n\
             CASEELSE\n\
                 RETURN 0\n\
         ENDSELECT\n\
         IF ATTRIBUTE_NAME == \"rect\"\n\
             SPLIT ATTRIBUTE_VALUE, \",\", POS_VALUES\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"xpos\", POS_VALUES:0\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"ypos\", POS_VALUES:1\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"width\", POS_VALUES:2\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"height\", POS_VALUES:3\n\
         ELSEIF ATTRIBUTE_NAME == \"pos\"\n\
             SPLIT ATTRIBUTE_VALUE, \",\", POS_VALUES\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"xpos\", POS_VALUES:0\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", \"ypos\", POS_VALUES:1\n\
         ELSE\n\
             XML_ADDATTRIBUTE RESULTS:20, \"//container[@name = 'main']/div\", ATTRIBUTE_NAME, ATTRIBUTE_VALUE\n\
         ENDIF\n\
         RETURN 1\n\
         @DIC_CONTAINSKEY(DICTIONARY, KEY)\n\
         #FUNCTION\n\
         #DIMS DICTIONARY\n\
         #DIMS KEY\n\
         RETURNF STRFINDU(DICTIONARY, \"[\" + KEY + \":\") >= 0\n\
         @DIC_GET(DICTIONARY, KEY)\n\
         #FUNCTIONS\n\
         #DIMS DICTIONARY\n\
         #DIMS KEY\n\
         #DIM OPEN\n\
         #DIM CLOSE\n\
         OPEN = STRFINDU(DICTIONARY, \"[\" + KEY + \":\")\n\
         SIF OPEN < 0\n\
             RETURNF \"\"\n\
         OPEN += STRLENSU(KEY) + 2\n\
         CLOSE = STRFINDU(DICTIONARY, \"]\", OPEN)\n\
         RETURNF SUBSTRINGU(DICTIONARY, OPEN, CLOSE - OPEN)\n\
         @DIC_KEYS(DICTIONARY)\n\
         #DIMS DICTIONARY\n\
         #DIMS PAIRS, 10\n\
         #DIM KEY_COUNT\n\
         #DIM INDEX\n\
         #DIM COLON\n\
         VARSET RESULTS\n\
         SPLIT DICTIONARY, \"[\", PAIRS\n\
         KEY_COUNT = RESULT\n\
         FOR INDEX, 0, KEY_COUNT\n\
             COLON = STRFINDU(PAIRS:INDEX, \":\")\n\
             SIF COLON < 0\n\
                 CONTINUE\n\
             RESULTS:(INDEX - 1) = %SUBSTRINGU(PAIRS:INDEX, 0, COLON)% \n\
         NEXT\n\
         RETURN KEY_COUNT - 1\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
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
        vm.read_variable(results, &[20], None),
        Ok(VmValue::String(
            "<layout><containers id=\"1\"><container name=\"main\"><div border=\"1px\" width=\"300\" height=\"400\" xpos=\"50\" ypos=\"0\" /></container></containers></layout>"
                .into()
        ))
    );
    let VmValue::String(division) = vm.read_variable(results, &[30], None).unwrap() else {
        panic!("UIC_SHOW fragment is not a string")
    };
    erabasic_html::parse_document(&division).expect("complete division rectangle should parse");
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
#[allow(clippy::too_many_lines)] // Keep every Rust/C# oracle watch explicit and reviewable.
fn erafl_xml_xpath_reference_fixture_runs_through_the_vm_host_abi() {
    let artifact = compile_source(include_str!(
        "../../../../../../tools/runtime-tester/fixture-reference/erb/xml-xpath.erb"
    ));
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_XML_XPATH")
        .expect("ORACLE_XML_XPATH")
        .key;
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
        vm.read_variable(result, &[60], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(results, &[60], None),
        Ok(VmValue::String("201,208,210".into()))
    );
    assert_eq!(
        vm.read_variable(result, &[61], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(results, &[61], None),
        Ok(VmValue::String("201,210,211".into()))
    );
    for (index, count, value) in [
        (62, 2, "208,211"),
        (63, 1, "201"),
        (64, 2, "210,211"),
        (65, 2, "201,210"),
        (66, 3, "201,210,211"),
        (67, 1, "201"),
        (68, 1, "1"),
        (69, 1, "5"),
        (70, 1, "Alice"),
    ] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(count))
        );
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(value.into()))
        );
    }
    assert_eq!(
        vm.read_variable(result, &[71], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[72], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[71], None),
        Ok(VmValue::String("1".into()))
    );
    assert_eq!(
        vm.read_variable(result, &[73], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(results, &[72], None),
        Ok(VmValue::String("<root>  <item id='1' />  </root>".into()))
    );
    for (index, count, result_index, value) in [
        (74, 1, 73, "201"),
        (75, 2, 74, "a,b"),
        (
            76,
            1,
            75,
            "<layout><containers id=\"1\"><container name=\"main\" /></containers></layout>",
        ),
    ] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(count))
        );
        assert_eq!(
            vm.read_variable(results, &[result_index], None),
            Ok(VmValue::String(value.into()))
        );
    }
}
