use super::*;
#[test]
fn getcsvno_methods_query_raw_loaded_names_in_snake_profile() {
    for profile in [erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake] {
        let files = ProjectFiles {
            csv: [
                (
                    "CHARA90.CSV",
                    "NO,9\nNAME,Shared\nCALLNAME,Call\nNICKNAME,Nick\nMASTERNAME,Master\n",
                ),
                (
                    "CHARA20.CSV",
                    "NO,2\nNAME,Shared\nCALLNAME,Call\nNICKNAME,Nick\nMASTERNAME,Master\n",
                ),
                ("CHARA1.CSV", "NO,1\nNAME,OnlyName\n"),
                (
                    "CHARA30.CSV",
                    "NO,3\nNAME,\nCALLNAME,\nNICKNAME,\nMASTERNAME,\n",
                ),
            ]
            .into_iter()
            .map(|(path, content)| FrontendFile {
                source_path: None,
                relative_path: path.into(),
                payload: CsvFilePayload::Utf8(content.into()),
            })
            .collect(),
            erb: Vec::new(),
        };
        let identity = erabasic_compat::CompatibilityIdentity::for_profile(profile);
        let data = load_project(
            &files,
            &CsvLoadOptions {
                compatibility: identity.clone(),
                compatible_call_name: true,
                ..CsvLoadOptions::default()
            },
        )
        .data
        .unwrap();
        let artifact = compile_source_with_data_and_options(
            concat!(
                "@SYSTEM_TITLE\n",
                "RESULT:0 = GETCSVNOBYNAME(\"Shared\")\n",
                "RESULT:1 = GETCSVNOBYCALLNAME(\"Call\")\n",
                "RESULT:2 = GETCSVNOBYNICKNAME(\"Nick\")\n",
                "RESULT:3 = GETCSVNOBYMASTERNAME(\"Master\")\n",
                "RESULT:4 = GETCSVNOBYNAME(\"shared\")\n",
                "RESULT:5 = GETCSVNOBYNAME(\"\")\n",
                "RESULT:6 = GETCSVNOBYCALLNAME(\"OnlyName\")\n",
                "RESULT:7 = CSVCALLNAME(1) == \"OnlyName\"\n",
                "RESULT:8 = GETCSVNOBYCALLNAME(\"\")\n",
                "RESULT:9 = GETCSVNOBYNICKNAME(\"\")\n",
                "RESULT:10 = GETCSVNOBYMASTERNAME(\"\")\n",
                "RETURN RESULT\n",
            ),
            data,
            &AnalyzerOptions {
                compatibility: identity,
                ..AnalyzerOptions::default()
            },
        );
        let result = artifact
            .globals
            .iter()
            .find(|global| global.name == "RESULT")
            .unwrap()
            .key;
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm
            .spawn_entry(artifact.functions[0].key, Vec::new())
            .unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        assert!(
            matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))),
            "{profile}: {report:?}"
        );
        for (index, expected) in [2, 2, 2, 2, -1, 3, -1, 1, 3, 3, 3].into_iter().enumerate() {
            assert_eq!(
                vm.read_variable(result, &[index as u64], None),
                Ok(VmValue::Integer(expected)),
                "{profile}: RESULT:{index}"
            );
        }
    }
}

#[test]
fn direct_runtime_fills_validate_the_complete_batch_before_mutation() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RESULT\n");
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(flag, &[0], None, VmValue::Integer(7))
        .unwrap();

    let error = vm.fill_runtime_variables(&[
        VmRuntimeFill {
            variable: flag,
            value: VmValue::Integer(1),
            all_characters: false,
        },
        VmRuntimeFill {
            variable: SymbolKey::derive("test.missing", b"runtime-fill"),
            value: VmValue::Integer(2),
            all_characters: false,
        },
    ]);
    assert!(matches!(error, Err(erabasic_vm::VmError::InvalidState(_))));
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(7)));

    vm.fill_runtime_variables(&[VmRuntimeFill {
        variable: flag,
        value: VmValue::Integer(3),
        all_characters: false,
    }])
    .unwrap();
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(3)));
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(3)));
}

#[test]
fn cvarset_prevalidates_and_fills_the_character_range() {
    let artifact =
        compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nCVARSET CFLAG, 1, 7, 0, 2\nRETURN RESULT\n");
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
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
            .map(|character| vm.read_variable(cflag, &[1], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(7), VmValue::Integer(7)]
    );
}

#[test]
fn script_can_address_character_storage_explicitly_or_through_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nCFLAG:0:1 = 3\nCFLAG:1:1 = 4\nTARGET = 1\nRESULT:0 = CFLAG:1\nRESULT:1 = CFLAG:0:1\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
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
        vm.read_variable(cflag, &[1], Some(0)),
        Ok(VmValue::Integer(3))
    );
    assert_eq!(
        vm.read_variable(cflag, &[1], Some(1)),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(4))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(3))
    );
}

#[test]
fn cvarset_invalid_range_does_not_write_any_character() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nTARGET = 0\nCFLAG:1 = 3\nTARGET = 1\nCFLAG:1 = 4\nCVARSET CFLAG, 1, 7, 0, 3\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let cflag = artifact
        .globals
        .iter()
        .find(|global| global.name == "CFLAG")
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
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("range")
    )));
    assert_eq!(
        (0..2)
            .map(|character| vm.read_variable(cflag, &[1], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(3), VmValue::Integer(4)]
    );
}

#[test]
fn sortchara_reorders_characters_and_remaps_target() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nNO = 30\nTARGET = 1\nNO = 10\nTARGET = 2\nNO = 20\nMASTER = -1\nSORTCHARA NO, FORWARD\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let no = artifact
        .globals
        .iter()
        .find(|global| global.name == "NO")
        .unwrap()
        .key;
    let target = artifact
        .globals
        .iter()
        .find(|global| global.name == "TARGET")
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
            .map(|character| vm.read_variable(no, &[], Some(character)).unwrap())
            .collect::<Vec<_>>(),
        vec![
            VmValue::Integer(10),
            VmValue::Integer(20),
            VmValue::Integer(30),
        ]
    );
    assert_eq!(vm.read_variable(target, &[], None), Ok(VmValue::Integer(1)));
}

#[test]
fn pickupchara_moves_selected_characters_and_remaps_special_indices() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nNO = 10\nTARGET = 1\nNO = 20\nTARGET = 2\nNO = 30\nASSI = 1\nMASTER = 2\nPICKUPCHARA 2, 0, 2\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let variable = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
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
    assert_eq!(
        (0..2)
            .map(|character| vm
                .read_variable(variable("NO"), &[], Some(character))
                .unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(30), VmValue::Integer(10)]
    );
    assert_eq!(
        vm.read_variable(variable("CHARANUM"), &[], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(variable("TARGET"), &[], None),
        Ok(VmValue::Integer(0))
    );
    assert_eq!(
        vm.read_variable(variable("ASSI"), &[], None),
        Ok(VmValue::Integer(-1))
    );
    assert_eq!(
        vm.read_variable(variable("MASTER"), &[], None),
        Ok(VmValue::Integer(0))
    );
}

#[test]
fn pickupchara_out_of_range_is_atomic() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nDELALLCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nNO = 10\nTARGET = 1\nNO = 20\nTARGET = 0\nASSI = 1\nMASTER = 1\nPICKUPCHARA 1, 99\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let variable = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
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
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("out of range")
    )));
    assert_eq!(
        (0..2)
            .map(|character| vm
                .read_variable(variable("NO"), &[], Some(character))
                .unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(10), VmValue::Integer(20)]
    );
    for (name, expected) in [("CHARANUM", 2), ("TARGET", 0), ("ASSI", 1), ("MASTER", 1)] {
        assert_eq!(
            vm.read_variable(variable(name), &[], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn sortchara_invalid_key_is_atomic() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nDELALLCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nNO = 30\nTARGET = 1\nNO = 10\nTARGET = 0\nASSI = 1\nMASTER = 1\nSORTCHARA FLAG, FORWARD\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let variable = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
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
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("character variable")
    )));
    assert_eq!(
        (0..2)
            .map(|character| vm
                .read_variable(variable("NO"), &[], Some(character))
                .unwrap())
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(30), VmValue::Integer(10)]
    );
    for (name, expected) in [("CHARANUM", 2), ("TARGET", 0), ("ASSI", 1), ("MASTER", 1)] {
        assert_eq!(
            vm.read_variable(variable(name), &[], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn cmatch_counts_an_indexed_character_field_across_the_requested_range() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nTARGET = 0\nCFLAG:5 = 9\nTARGET = 1\nCFLAG:5 = 4\nTARGET = 2\nCFLAG:5 = 9\nRETURN CMATCH(CFLAG:5, 9, 0, 3)\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));
}

#[test]
fn failed_character_mutation_rolls_back_the_complete_candidate() {
    let artifact = compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nDELCHARA 0, 99\nRETURN RESULT\n");
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
        VmEvent::FiberFaulted { fault, .. } if fault.message.contains("out of range")
    )));
    // New-game memory starts with one character. ADDVOIDCHARA committed first,
    // while the later multi-delete validates every index before mutating memory.
    assert_eq!(vm.export_era_state().characters.len(), 2);
}

#[test]
fn character_csv_queries_use_loaded_templates_and_character_lookup() {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "CHARA0.CSV".into(),
                payload: CsvFilePayload::Utf8("NO,10\nNAME,Alice\nBASE,0,100\nCFLAG,1,7\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("character CSV should load");
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nRESULT:0 = GETCHARA(10)\nRESULT:1 = CSVBASE(10, 0)\nRESULT:2 = CSVCFLAG(10, 1)\nRESULT:3 = CSVNAME(10) == \"Alice\"\nRETURN RESULT\n",
        loaded,
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
            VmValue::Integer(0),
            VmValue::Integer(100),
            VmValue::Integer(7),
            VmValue::Integer(1),
        ]
    );
}

#[test]
fn title_memory_initializes_gamebase_and_replace_calculated_variables() {
    let mut data = project_data();
    data.static_data.game_base.unique_code = 42;
    data.static_data.game_base.version = 1_234;
    data.static_data.game_base.compatible_min_version = 1_200;
    data.static_data.game_base.default_character = 7;
    data.static_data.game_base.no_item = 8;
    data.static_data.game_base.title = "Demo".into();
    data.static_data.game_base.author = "Author".into();
    data.static_data.game_base.year = "2026".into();
    data.static_data.game_base.info = "Info".into();
    data.static_data.game_base.window_title = Some("Window".into());
    data.static_data.game_base.update_url = "https://example.invalid/update".into();
    data.static_data.game_base.version_name = "Release".into();
    data.static_data.replace.money_label = "円".into();
    data.static_data.replace.draw_line_string = "=".into();

    let artifact = compile_source_with_data("@SYSTEM_TITLE\nRETURN RESULT\n", data);
    let vm = Vm::new(validated(&artifact), VmConfig::default());
    let read = |name: &str| {
        let key = artifact
            .globals
            .iter()
            .find(|global| global.name == name)
            .unwrap()
            .key;
        vm.read_variable(key, &[], None).unwrap()
    };

    assert_eq!(read("GAMEBASE_GAMECODE"), VmValue::Integer(42));
    assert_eq!(read("GAMEBASE_VERSION"), VmValue::Integer(1_234));
    assert_eq!(read("GAMEBASE_ALLOWVERSION"), VmValue::Integer(1_200));
    assert_eq!(read("GAMEBASE_DEFAULTCHARA"), VmValue::Integer(7));
    assert_eq!(read("GAMEBASE_NOITEM"), VmValue::Integer(8));
    assert_eq!(read("GAMEBASE_AUTHER"), VmValue::String("Author".into()));
    assert_eq!(read("GAMEBASE_AUTHOR"), VmValue::String("Author".into()));
    assert_eq!(read("GAMEBASE_INFO"), VmValue::String("Info".into()));
    assert_eq!(read("GAMEBASE_YEAR"), VmValue::String("2026".into()));
    assert_eq!(read("GAMEBASE_TITLE"), VmValue::String("Demo".into()));
    assert_eq!(
        read("GAMEBASE_URL"),
        VmValue::String("https://example.invalid/update".into())
    );
    assert_eq!(
        read("GAMEBASE_VERSIONNAME"),
        VmValue::String("Release".into())
    );
    assert_eq!(read("WINDOW_TITLE"), VmValue::String("Window".into()));
    assert_eq!(read("MONEYLABEL"), VmValue::String("円".into()));
    assert_eq!(read("DRAWLINESTR"), VmValue::String("=".into()));
    assert_eq!(read("EMUERA_VERSION"), VmValue::String("1.824.0.0".into()));
    assert_eq!(read("__INT_MAX__"), VmValue::Integer(i64::MAX));
    assert_eq!(read("__INT_MIN__"), VmValue::Integer(i64::MIN));
}

#[test]
fn normal_addchara_accepts_nonzero_cflag_zero_when_sp_compatibility_is_disabled() {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "CHARA0.CSV".into(),
                payload: CsvFilePayload::Utf8("NO,0\nNAME,Player\nCFLAG,0,1900\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .expect("character CSV should load");
    let artifact = compile_source_with_data(
        "@SYSTEM_TITLE\nADDCHARA 0\nRESULT = CHARANUM\nRETURN RESULT\n",
        loaded,
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(2));
}

#[test]
fn resetdata_clears_initial_characters_before_script_insertion() {
    let loaded = load_project(
        &ProjectFiles {
            csv: vec![FrontendFile {
                source_path: None,
                relative_path: "Chara/CHARA0.CSV".into(),
                payload: CsvFilePayload::Utf8("NO,0\nNAME,Master\n".into()),
            }],
            erb: Vec::new(),
        },
        &CsvLoadOptions {
            search_subdirectories: true,
            ..CsvLoadOptions::default()
        },
    )
    .data
    .expect("character CSV should load");
    let artifact = compile_source_with_data("@SYSTEM_TITLE\nRETURN RESULT\n", loaded);
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    assert_eq!(vm.export_era_state().characters.len(), 1);

    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::ResetGameData)
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();

    assert!(vm.export_era_state().characters.is_empty());
}
