use super::*;
#[test]
fn getcsvno_form_calls_use_loaded_raw_names_without_native_imports() {
    let identity = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let data = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "CHARA90.CSV".into(),
                payload: CsvFilePayload::Utf8(
                    "NO,7\nNAME,Raw\nCALLNAME,Call\nNICKNAME,Nick\nMASTERNAME,Master\n".into(),
                ),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions {
            compatibility: identity.clone(),
            ..CsvLoadOptions::default()
        },
    )
    .data
    .unwrap();
    let artifact = compile_source_with_data_and_options(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("{GETCSVNOBYNAME(\"Raw\")}:{GETCSVNOBYCALLNAME(\"Call\")}:{GETCSVNOBYNICKNAME(\"Nick\")}:{GETCSVNOBYMASTERNAME(\"Master\")}")
RETURN
"#,
        data,
        &AnalyzerOptions {
            compatibility: identity,
            ..AnalyzerOptions::analysis_mode()
        },
    );
    assert!(
        !artifact
            .native_imports
            .iter()
            .any(|native| native.import.name.starts_with("getcsvno"))
    );
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))),
        "{report:?}"
    );
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::String("7:7:7:7".into()))
    );
}

#[test]
fn raw_character_name_presence_changes_compiled_artifact_identity() {
    let identity = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let compile = |callname: &str| {
        let data = load_project(
            &ProjectFiles {
                csv: vec![FrontendFile {
                    source_path: None,
                    relative_path: "CHARA7.CSV".into(),
                    payload: CsvFilePayload::Utf8(format!("NO,7\nNAME,Alice\n{callname}")),
                }],
                erb: Vec::new(),
            },
            &CsvLoadOptions {
                compatibility: identity.clone(),
                compatible_call_name: true,
                ..CsvLoadOptions::default()
            },
        )
        .data
        .unwrap();
        compile_source_with_data_and_options(
            "@SYSTEM_TITLE\nRESULT = GETCSVNOBYCALLNAME(\"\")\nRETURN\n",
            data,
            &AnalyzerOptions {
                compatibility: identity.clone(),
                ..AnalyzerOptions::analysis_mode()
            },
        )
    };
    let missing = compile("");
    let explicit = compile("CALLNAME,\n");
    assert_eq!(
        missing.project_data.static_data.characters,
        explicit.project_data.static_data.characters
    );
    assert_ne!(missing.manifest.artifact_id, explicit.manifest.artifact_id);
}
