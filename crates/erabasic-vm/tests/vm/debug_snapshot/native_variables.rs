use super::*;
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
fn compiled_split_uses_the_whole_indexed_array_and_preserves_its_tail() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIMS TEMP, 2048\nTEMP:3 '= \"tail\"\nSPLIT \"a/b\", \"/\", TEMP:2, RESULT\nRETURN (TEMP:0 == \"a\") * 1000 + (TEMP:1 == \"b\") * 100 + (TEMP:3 == \"tail\") * 10 + RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1112));
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
        "@SYSTEM_TITLE\nRESULTS '= \"dynamic-key\"\nRESULT = GETNUM(CFLAG, RESULTS)\nRETURN RESULT\n",
        data,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(17));
}

#[test]
fn folded_getnum_lookups_preserve_execution_results() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("known".into(), 17);
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cdflag2)
        .unwrap()
        .lookup
        .insert("second".into(), 23);
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\n\
         RESULT = GETNUM(CFLAG, \"known\") * 10000\n\
         RESULT += (GETNUM(CFLAG, \"missing\") + 1) * 1000\n\
         RESULT += GETNUM(CDFLAG, \"second\", 2) * 10\n\
         RESULT += GETNUM(CFLAG, \"known\", -1) + 1\n\
         RETURN RESULT\n",
        data,
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(170_230));
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
            canonical_names: [(0, "zero".into()), (1, "second".into())]
                .into_iter()
                .collect(),
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

fn alias_project_data() -> erabasic_data::ProjectData {
    let mut data = project_data();
    data.static_data.deferred_indices.resolved.insert(
        "BUFF".into(),
        erabasic_data::ResolvedUserIndex {
            variable_name: "BUFF".into(),
            entries: [
                ("main".into(), 10),
                ("alias".into(), 10),
                ("negative".into(), -1),
                ("outside".into(), 300),
            ]
            .into_iter()
            .collect(),
            canonical_names: [
                (10, "main".into()),
                (-1, "negative".into()),
                (300, "outside".into()),
            ]
            .into_iter()
            .collect(),
        },
    );
    data
}

#[test]
fn snake_aliases_resolve_dynamic_indices_and_keep_primary_reverse_names() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let artifact = compile_source_with_data_and_options(
        "@SYSTEM_TITLE\n#DIM BUFF,32\nRESULTS:0 = alias\nBUFF:alias = 42\nRESULT = BUFF:(RESULTS:0) * 10 + (ERDNAME(BUFF,10) == \"main\")\nRETURN RESULT\n",
        alias_project_data(),
        &options,
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(421));

    let original = compile_source_with_data(
        "@SYSTEM_TITLE\n#DIM BUFF,32\nRESULT = ERDNAME(BUFF,10) == \"alias\"\nRETURN RESULT\n",
        alias_project_data(),
    );
    assert_eq!(run_compiled_result(&original), VmValue::Integer(1));
}

#[test]
fn snake_signed_aliases_still_fault_on_out_of_bounds_array_access() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    for alias in ["negative", "outside"] {
        let artifact = compile_source_with_data_and_options(
            &format!(
                "@SYSTEM_TITLE\n#DIM BUFF,32\nRESULTS:0 = {alias}\nRESULT = BUFF:(RESULTS:0)\nRETURN RESULT\n"
            ),
            alias_project_data(),
            &options,
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
            "{alias}: {:?}",
            report.events
        );
    }
}

#[test]
fn erafl_compatibility_fixture_compiles_and_matches_the_reference_result() {
    const SOURCE: &str = "@ERAFL_COMPAT\n#DIM\u{3000}OUT\n#DIMS CONST PAD = \" \" * 3\nVARI COUNT = 2\nVARS WORD = \"xy\"\nVARI ITEMS, 3\n{\t\nCOUNT += 1\n}\t\nFOR LOCAL, , 2\nITEMS:LOCAL = COUNT\nNEXT\nIF 0\nOUT = ENUMFILES(\"missing-directory\", \"*.none\")\nCALLSHARP MISSING_PLUGIN()\nENDIF\nRESULT = COUNT * 10000 + (WORD == \"xy\") * 1000 + (ITEMS:1 == 3) * 100 + (PAD == \"   \") * 10 + (ERDNAME(CUSTOM_NAMES, 2) == \"later\")\nRETURN RESULT\n";
    let mut data = project_data();
    data.static_data.deferred_indices.resolved.insert(
        "CUSTOM_NAMES".into(),
        erabasic_data::ResolvedUserIndex {
            variable_name: "CUSTOM_NAMES".into(),
            canonical_names: [(0, "zero".into()), (2, "later".into())]
                .into_iter()
                .collect(),
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
