use super::*;

#[test]
fn indexed_data_targets_dynamic_labels_and_try_lists_execute_lazily() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nPRINTDATA RESULT:2\nDATA chosen\nENDDATA\nSTRDATA RESULTS:3\nDATA stored\nENDDATA\nTRYCALLLIST\nFUNC MISSING, 1 / LOCAL\nFUNC LIST_TARGET, 7\nENDFUNC\nRESULTS:11 = %\"MISSING_LABEL\"%\nTRYCGOTOFORM %RESULTS:11%\nCATCH\nRESULT:3 = 3\nENDCATCH\nTRYGOTOLIST\nFUNC MISSING_LABEL\nFUNC FOUND_LABEL\nENDFUNC\nRESULT:4 = 99\n$FOUND_LABEL\nRESULT:4 = 4\nRETURN RESULT\n@LIST_TARGET(ARG)\nFLAG:0 = ARG\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
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
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[3], None),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(result, &[4], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(results, &[3], None),
        Ok(VmValue::String("stored".into()))
    );
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(7)));
}

#[test]
fn callevent_runs_the_reference_event_group_inside_the_calling_fiber() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 0\nCALLEVENT EVENTFIRST\nRESULT:3 = 9\nRETURN RESULT\n@EVENTFIRST\n#PRI\nRESULT:0 += 1\nRETURN RESULT\n@EVENTFIRST\nRESULT:1 += 2\nRETURN RESULT\n@EVENTFIRST\n#LATER\nRESULT:2 += 4\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
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
    for (index, expected) in [(0, 1), (1, 2), (2, 4), (3, 9)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn dynamic_calls_apply_omission_conversion_and_event_compatibility_options() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatible_function_argument_optional = true;
    options.compatible_function_argument_auto_convert = true;
    options.compatible_call_event = true;
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\nCALLFORM STRING_TARGET()\nCALLFORM STRING_TARGET(7)\nCALLFORM EVENTFIRST\nRETURN RESULT\n@STRING_TARGET(ARGS)\nRESULTS:0 = %ARGS%\nRETURN RESULT\n@EVENTFIRST\nRESULT:0 = 8\nRETURN RESULT\n",
        &options,
    );
    assert!(artifact.call_compatibility.allow_omitted_arguments);
    assert!(artifact.call_compatibility.auto_convert_integer_to_string);
    assert!(artifact.call_compatibility.allow_event_as_normal);
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(8))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("7".into()))
    );
}

#[test]
fn compatibility_rest_matches_the_reference_oracle_fixture() {
    let artifact = compile_source(include_str!(
        "../../../../tools/runtime-tester/fixture-reference/erb/oracle.erb"
    ));
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_COMPAT_REST")
        .expect("oracle entry")
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
    for (index, expected) in [(1, 0), (2, 3), (3, 4), (4, 1), (5, 2)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        vm.read_variable(results, &[10], None),
        Ok(VmValue::String("STORED".into()))
    );
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(7)));
    assert_eq!(vm.read_variable(flag, &[2], None), Ok(VmValue::Integer(8)));
}

#[test]
fn dynamic_statement_calls_enforce_method_and_normal_function_kinds() {
    let valid = compile_source(
        "@SYSTEM_TITLE\nCALLFORMF METHOD_TARGET(3)\nRETURN RESULT\n@METHOD_TARGET(ARG)\n#FUNCTION\nRETURNF ARG + 1\n",
    );
    let entry = valid
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&valid);
    let mut vm = Vm::new(validated(&valid), VmConfig::default());
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

    let invalid = compile_source(
        "@SYSTEM_TITLE\nCALLFORM METHOD_TARGET(3)\nRETURN RESULT\n@METHOD_TARGET(ARG)\n#FUNCTION\nRETURNF ARG + 1\n",
    );
    let entry = invalid
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("entry")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&invalid);
    let mut vm = Vm::new(validated(&invalid), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::TypeMismatch
    )));
}

#[test]
fn nested_method_character_selector_preserves_the_returned_character() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         ADDVOIDCHARA\n\
         ADDVOIDCHARA\n\
         MASTER = 0\n\
         CFLAG:MASTER:300 = 260\n\
         CFLAG:1:300 = 260\n\
         TCVAR:1:30 = 2\n\
         RESULT = TCVAR:(IN_ROOM_MEMBER(260)):30\n\
         RETURN RESULT\n\
         @IN_ROOM_MEMBER(ARG)\n\
         #FUNCTION\n\
         VARSET LOCAL, 0\n\
         FOR LOCAL, 1, CHARANUM\n\
             SIF CFLAG:LOCAL:300 != ARG\n\
                 CONTINUE\n\
             IF LOCAL:3++ == 0\n\
                 LOCAL:1 = TCVAR:LOCAL:30\n\
                 LOCAL:2 = LOCAL\n\
             ELSEIF LOCAL:1 > TCVAR:LOCAL:30\n\
                 LOCAL:1 = TCVAR:LOCAL:30\n\
                 LOCAL:2 = LOCAL\n\
             ENDIF\n\
         NEXT\n\
         RETURNF LOCAL:2\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));
}

#[test]
fn increment_expressions_mutate_their_place_and_return_reference_values() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         LOCAL = 4\n\
         LOCAL:1 = LOCAL++\n\
         LOCAL:2 = ++LOCAL\n\
         LOCAL:3 = LOCAL--\n\
         LOCAL:4 = --LOCAL\n\
         RETURN LOCAL:1 * 1000 + LOCAL:2 * 100 + LOCAL:3 * 10 + LOCAL:4\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(4664));
}

#[test]
fn compiled_bit_mutations_prevalidate_and_update_the_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT = 0\nSETBIT RESULT, 1, 3\nINVERTBIT RESULT, 1\nCLEARBIT RESULT, 3\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));
}

#[test]
fn compiled_split_preserves_empty_fields_and_reports_the_full_count() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS TEMP, 4\nSPLIT \"a//b/\", \"/\", TEMP\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(4));
}

#[test]
fn getnum_resolves_the_referenced_builtin_name_table_at_runtime() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("dynamic-key".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"dynamic-key\")\nRETURN RESULT\n",
        data,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(17));
}

#[test]
fn getnum_runtime_source_dimension_matches_constant_evaluation() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Palam)
        .unwrap()
        .lookup
        .insert("快Ｃ".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n#DIM CONST COMPILED = GETNUM(CUP, \"快Ｃ\", 1)\nRESULT = COMPILED * 100 + GETNUM(CUP, \"快Ｃ\", 1)\nRETURN RESULT\n",
        data,
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1_717));
}

#[test]
fn erdname_resolves_a_user_defined_index_name_at_runtime() {
    let mut data = project_data();
    data.static_data.deferred_indices.resolved.insert(
        "CUSTOM_NAMES".into(),
        erabasic_data::ResolvedUserIndex {
            variable_name: "CUSTOM_NAMES".into(),
            entries: [("zero".into(), 0), ("second".into(), 1)]
                .into_iter()
                .collect(),
        },
    );
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n#DIMS CUSTOM_NAMES, 2\nRESULT = ERDNAME(CUSTOM_NAMES, 1) == \"second\"\nRETURN RESULT\n",
        data,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn erafl_compatibility_fixture_compiles_and_matches_the_reference_result() {
    const SOURCE: &str = "@ERAFL_COMPAT\n#DIM\u{3000}OUT\n#DIMS CONST PAD = \" \" * 3\nVARI COUNT = 2\nVARS WORD = \"xy\"\nVARI ITEMS, 3\n{\t\nCOUNT += 1\n}\t\nFOR LOCAL, , 2\nITEMS:LOCAL = COUNT\nNEXT\nIF 0\nOUT = ENUMFILES(\"missing-directory\", \"*.none\")\nCALLSHARP MISSING_PLUGIN()\nENDIF\nRESULT = COUNT * 10000 + (WORD == \"xy\") * 1000 + (ITEMS:1 == 3) * 100 + (PAD == \"   \") * 10 + (ERDNAME(CUSTOM_NAMES, 2) == \"later\")\nRETURN RESULT\n";
    let mut data = project_data();
    data.static_data.deferred_indices.resolved.insert(
        "CUSTOM_NAMES".into(),
        erabasic_data::ResolvedUserIndex {
            variable_name: "CUSTOM_NAMES".into(),
            entries: [("zero".into(), 0), ("later".into(), 2)]
                .into_iter()
                .collect(),
        },
    );
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources: vec![
                ProjectSource {
                    relative_path: "custom-names.erh".into(),
                    payload: SourcePayload::Utf8("#DIMS CUSTOM_NAMES, 3\n".into()),
                },
                ProjectSource {
                    relative_path: "erafl-compat.erb".into(),
                    payload: SourcePayload::Utf8(SOURCE.into()),
                },
            ],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(analysis.project.is_some(), "{:#?}", analysis.diagnostics);
    let compilation = compile_project(
        &analysis.project.unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    );
    let artifact = compilation
        .artifact
        .unwrap_or_else(|| panic!("{:#?}", compilation.diagnostics));
    assert!(artifact.host_imports.iter().any(|import| {
        import.import.namespace == "rustyera.extension" && import.import.name == "callsharp"
    }));

    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ERAFL_COMPAT")
        .unwrap()
        .key;
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(31_111))
    );
}

#[test]
fn runtime_string_indices_use_strict_name_resolution() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Flag)
        .unwrap()
        .lookup
        .insert("dynamic-key".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nFLAG:17 = 9\nRESULTS:0 '= \"dynamic-key\"\nRESULT = FLAG:(RESULTS:0)\nRETURN RESULT\n",
        data,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(9));

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= \"missing-key\"\nRESULT = FLAG:(RESULTS:0)\nRETURN RESULT\n",
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
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::FiberFaulted { fault, .. }
                if fault.message.contains("FLAG has no named index")
                    && fault.message.contains("missing-key")
        )),
        "{:#?}",
        report.events
    );
}

#[test]
fn runtime_string_indices_use_shared_builtin_name_tables() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Palam)
        .unwrap()
        .lookup
        .insert("快Ｃ".into(), 17);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nCUP:0:17 = 9\nRESULTS:0 '= \"快Ｃ\"\nRESULT = CUP:0:(RESULTS:0)\nRETURN RESULT\n",
        data,
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(9));
}

#[test]
fn break_at_the_end_of_a_selected_branch_does_not_enter_else() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT:0 = BREAK_BRANCH()\n\
         RETURN RESULT\n\
         @BREAK_BRANCH()\n\
         #FUNCTION\n\
         #DIM LCOUNT\n\
         IF 1\n\
             FOR LCOUNT, 0, 1\n\
                 BREAK\n\
             NEXT\n\
         ELSE\n\
             RETURNF 99\n\
         ENDIF\n\
         RETURNF 7\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(7));
}

#[test]
fn native_tail_matches_the_reference_oracle_fixture() {
    let artifact = compile_source(
        "@ORACLE_NATIVE\n#DIMS PARTS, 4\n#DIMS JOINED, 4\n#DIMS REPEATED, 1\nRESULT:0 = 0\nSETBIT RESULT:0, 1, 3\nINVERTBIT RESULT:0, 1\nCLEARBIT RESULT:0, 3\nSPLIT \"a//b/\", \"/\", PARTS, RESULT:1\nRESULT:2 = STRCOUNT(\"ababa\", \"aba\")\nRESULT:3 = GETPALAMLV(499, 5)\nRESULTS:0 = %ESCAPE(\"a+b\")%\nJOINED:0 = %\"a\"%\nJOINED:1 = %\"b\"%\nJOINED:2 = %\"c\"%\nRESULT:4 = STRLENS(\"Ab\")\nRESULT:5 = STRLENSU(\"Aé\")\nRESULT:12 = STRLENSU(\"😀\")\nRESULT:6 = STRFINDU(\"aβc\", \"β\")\nRESULT:7 = ENCODETOUNI(\"β\")\nRESULT:8 = UNICODEBYTE(\"β\")\nRESULTS:1 = %CHARATU(\"aβ\", 1)%\nRESULTS:2 = %TOUPPER(\"Abc\")%\nRESULTS:3 = %TOLOWER(\"AbC\")%\nRESULTS:4 = %STRJOIN(JOINED, \"/\", 1, 2)%\nRESULTS:5 = %STRJOIN(JOINED)%\nRESULTS:6 = %UNICODE(946)%\nRESULT:9 = TOINT(\"12.9\")\nRESULT:10 = ISNUMERIC(\"0x10\")\nRESULT:11 = COLOR_FROMRGB(1, 2, 3)\nRESULTS:7 = %CONVERT(255, 16)%\nREPEATED:0 '= \"<p>----------------</p>\"\nRESULT:13 = FINDELEMENT(REPEATED, \"([^ ])\\\\1{15}\", 0, 1, 0)\nRETURN RESULT\n",
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
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(4))
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
        Ok(VmValue::String("a\\+b".into()))
    );
    for (index, expected) in [(4, 2), (5, 2), (6, 1), (7, 946), (8, 946)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    for (index, expected) in [(9, 12), (10, 1), (11, 0x0001_0203)] {
        assert_eq!(
            vm.read_variable(result, &[index], None),
            Ok(VmValue::Integer(expected))
        );
    }
    assert_eq!(
        vm.read_variable(result, &[12], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[13], None),
        Ok(VmValue::Integer(0))
    );
    for (index, expected) in [
        (1, "β"),
        (2, "ABC"),
        (3, "abc"),
        (4, "b/c"),
        (5, "a,b,c,"),
        (6, "β"),
        (7, "ff"),
    ] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn unicode_positions_and_lengths_follow_reference_encodings() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = STRLENSU(\"A😀\")\nRESULT:1 = STRFINDU(\"A😀B\", \"B\")\nRESULT:2 = STRLENS(\"Aé\")\nRESULTS:0 = %CHARATU(\"A😀B\", 1)%\nRETURN RESULT\n",
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
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("😀".into()))
    );
}

#[test]
fn legacy_string_width_keeps_narrow_formcell_text_out_of_the_truncation_path() {
    let mut data = project_data();
    data.static_data.legacy_encoding = erabasic_data::LegacyEncoding::ChineseHans;
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         RESULT:0 = STRLENS(\"们\")\n\
         RESULTS:0 = %FORMCELL(\"们\", 2)%\n\
         RESULTS:1 = %SUBSTRING(\"甲乙\", 0, 3)%\n\
         RETURN RESULT\n\
         @FORMCELL(ARGS, ARG = -1)\n\
         #FUNCTIONS\n\
         #DIMS DYNAMIC LOCALSTR\n\
         #DIM DYNAMIC WIDTH\n\
         WIDTH = STRLENS(ARGS)\n\
         IF ARG > 0 && WIDTH > ARG\n\
             LOCALSTR = %SUBSTRING(ARGS, 0, ARG - 3)%\n\
             WIDTH = STRLENS(LOCALSTR)\n\
             LOCALSTR += \".\" * (ARG - WIDTH)\n\
         ELSE\n\
             LOCALSTR = %ARGS%\n\
         ENDIF\n\
         RETURNF LOCALSTR\n",
        data,
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
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("们".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("甲乙".into()))
    );
}

#[test]
fn runtime_draw_line_width_prevents_formcell_from_truncating_to_a_negative_count() {
    let mut data = project_data();
    data.static_data.legacy_encoding = erabasic_data::LegacyEncoding::ChineseHans;
    data.static_data.replace.draw_line_string = "-".into();
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         RESULT:0 = STRLENS(DRAWLINESTR)\n\
         RESULTS:0 = %FORMCELL(\"温馨提示：选择物品时，按住空格可以临时进入多选模式。\", MAXWIDTH(), \"LEFT\")%\n\
         RETURN RESULT\n\
         @MAXWIDTH\n\
         #FUNCTION\n\
         RETURNF STRLENS(DRAWLINESTR)\n\
         @FORMCELL(ARGS, ARG = -1, ARGS:1 = \"LEFT\", ARG:1 = 0)\n\
         #FUNCTIONS\n\
         #DIMS DYNAMIC LOCALSTR,1\n\
         #DIM DYNAMIC FORMCELLTEMP,3\n\
         FORMCELLTEMP = STRLENS(ARGS)\n\
         IF ARG > 0 && FORMCELLTEMP > ARG\n\
             IF ARG:1\n\
                 LOCALSTR = %SUBSTRING(ARGS, 0, ARG)%\n\
             ELSE\n\
                 LOCALSTR = %SUBSTRING(ARGS, 0, ARG - 3)%\n\
                 FORMCELLTEMP = STRLENS(LOCALSTR)\n\
                 LOCALSTR += \".\" * (ARG - FORMCELLTEMP)\n\
             ENDIF\n\
             FORMCELLTEMP = STRLENS(LOCALSTR)\n\
         ELSE\n\
             LOCALSTR = %ARGS%\n\
             SIF ARG < 0\n\
                 ARG = 0\n\
         ENDIF\n\
         RETURNF @\"%LOCALSTR,ARG,LEFT%\"\n",
        data,
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
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    vm.set_line_columns(198);
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetGameData)
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.drive(RunBudget::default(), VmDriveMode::Normal);

    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, erabasic_vm::VmPortEvent::FiberFaulted(..))),
        "{:#?}",
        report.events
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
    assert_eq!(
        vm.vm().read_variable(result, &[0], None),
        Ok(VmValue::Integer(198))
    );
    let VmValue::String(cell) = vm.vm().read_variable(results, &[0], None).unwrap() else {
        panic!("FORMCELL must return a string");
    };
    assert!(cell.starts_with("温馨提示：选择物品时，按住空格可以临时进入多选模式。"));
    assert_eq!(UnicodeWidthStr::width(cell.as_str()), 198);
}

#[test]
fn runtime_fault_resolves_to_utf8_source_location() {
    let entry = SymbolKey::derive("test.function", b"fault");
    let first = erabasic_bytecode::EncodedInstruction::new(Opcode::Nop, Vec::new());
    let first_length = first.encoded_len();
    let trap = erabasic_bytecode::EncodedInstruction::new(Opcode::Trap, b"intentional".to_vec());
    let length = first_length + trap.encoded_len();
    let mut artifact = artifact(
        vec![function(entry, "FAULT", vec![first, trap])],
        Vec::new(),
    );
    let text = "@FAULT\nTRAP 中文\n";
    artifact.source_map = SourceMap {
        sources: vec![SourceRecord {
            relative_path: "fault.erb".into(),
            content_hash: Digest::hash("test.source", &[text.as_bytes()]),
            byte_len: text.len() as u64,
            line_starts: vec![0, "@FAULT\n".len() as u64],
        }],
        statement_fingerprints: vec![
            Digest::hash("test.statement", &[b"fault"]),
            Digest::hash("test.statement", &[b"overlap"]),
        ],
        entries: vec![
            SourceMapEntry {
                function: entry,
                code_start: 0,
                code_end: length,
                source_index: 0,
                byte_start: "@FAULT\n".len() as u64,
                byte_end: text.len() as u64,
                statement_fingerprint: 0,
                origin_chain: None,
            },
            // A later, narrower overlapping entry must not override the serialized map's first
            // match. The generation index is an execution cache, not a semantic reordering.
            SourceMapEntry {
                function: entry,
                code_start: first_length,
                code_end: length,
                source_index: 0,
                byte_start: 0,
                byte_end: "@FAULT".len() as u64,
                statement_fingerprint: 1,
                origin_chain: None,
            },
        ],
    };
    artifact.refresh_ids().unwrap();
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut NativeServiceRegistry::default(),
        RunBudget::default(),
    );
    let Some(FiberStatus::Faulted(fault)) = vm.fiber_status(fiber) else {
        panic!("fiber should fault");
    };
    let source = fault.source.expect("fault should have a source location");
    assert_eq!(source.relative_path, "fault.erb");
    assert_eq!(source.line, 2);
    assert_eq!(source.byte_column, 0);
}
