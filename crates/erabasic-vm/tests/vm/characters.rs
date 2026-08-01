use super::*;

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

#[test]
fn duplicate_event_handlers_share_persistent_era_locals() {
    let artifact = compile_source(
        "@EVENTTRAIN\nVARSET LOCAL\nRETURN RESULT\n@EVENTTRAIN\nLOCAL:0 = 1\nRETURN RESULT\n",
    );
    let entries = artifact
        .functions
        .iter()
        .filter(|function| function.name.eq_ignore_ascii_case("EVENTTRAIN"))
        .map(|function| function.key)
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);

    for entry in entries {
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
    }
}

#[test]
fn regexpmatch_writes_reference_capture_outputs_atomically() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:0 = REGEXPMATCH(\"ab ac\", \"a(.)\", 1)\nRESULT:2 = REGEXPMATCH(\"az\", \"a(.)\", RESULT:5, RESULTS)\nRETURN RESULT\n",
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
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[2], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(result, &[5], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("az".into()))
    );
    assert_eq!(
        vm.read_variable(results, &[1], None),
        Ok(VmValue::String("z".into()))
    );
}

#[test]
fn initrand_and_dumprand_exchange_all_randdata_state_atomically() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nDUMPRAND\nRESULT:0 = RAND:1000000\nINITRAND\nRESULT:1 = RAND:1000000\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        vm.read_variable(result, &[1], None)
    );
}

#[test]
fn cooperative_fibers_are_round_robin_and_complete_independently() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let first = vm.spawn_entry(entry, Vec::new()).unwrap();
    let second = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 32,
            maximum_host_calls: 8,
            fiber_quantum: 1,
        },
    );
    assert_eq!(host.calls, vec![7, 7]);
    assert!(matches!(
        vm.fiber_status(first),
        Some(FiberStatus::Completed(None))
    ));
    assert!(matches!(
        vm.fiber_status(second),
        Some(FiberStatus::Completed(None))
    ));
    assert_eq!(
        report
            .events
            .iter()
            .filter(|event| matches!(event, VmEvent::FiberCompleted { .. }))
            .count(),
        2
    );
}

#[test]
fn terminal_fibers_retire_and_reuse_the_smallest_available_id() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN 7\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let first = vm.spawn_entry(entry, Vec::new()).unwrap();
    let second = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());

    assert_eq!(first, FiberId(1));
    assert_eq!(second, FiberId(2));
    assert_eq!(
        report
            .events
            .iter()
            .filter_map(|event| match event {
                VmEvent::FiberCompleted { value, .. } => value.clone(),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![VmValue::Integer(7), VmValue::Integer(7)]
    );
    assert_eq!(vm.retire_terminal_fibers(), 2);
    assert_eq!(vm.fiber_status(first), None);
    assert_eq!(vm.fiber_status(second), None);
    assert_eq!(vm.spawn_entry(entry, Vec::new()).unwrap(), FiberId(1));
}

#[test]
fn cancelled_fibers_release_frames_before_id_retirement() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RESULT\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();

    assert_eq!(vm.fiber_frame_count(fiber), Some(1));
    vm.cancel_fiber(fiber).unwrap();
    assert_eq!(vm.fiber_frame_count(fiber), Some(0));
    assert_eq!(vm.retire_terminal_fibers(), 1);
    assert_eq!(vm.fiber_status(fiber), None);
    assert_eq!(vm.spawn_entry(entry, Vec::new()).unwrap(), fiber);
}

#[test]
fn debugger_completion_stop_protects_terminal_fiber_until_continue() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let pause = vm.request_pause().unwrap();
    vm.step(pause.token, fiber, VmStepKind::SourceLine).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let completed_stop = report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::DebugStopped(stop)
                if matches!(stop.reason, erabasic_vm::VmDebugStopReason::FiberCompleted) =>
            {
                Some(stop.clone())
            }
            _ => None,
        })
        .expect("completion step should establish a debugger stop");

    assert_eq!(vm.retire_terminal_fibers(), 0);
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
    vm.continue_execution(completed_stop.token).unwrap();
    assert_eq!(vm.retire_terminal_fibers(), 1);
    assert_eq!(vm.fiber_status(fiber), None);
}

#[test]
fn cancelled_host_wait_rejects_late_completion_before_reusing_id() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
        panic!("fiber should be waiting for its host request");
    };

    vm.cancel_fiber(fiber).unwrap();
    assert!(matches!(
        vm.resume_host(request, HostReady::empty()),
        Err(erabasic_vm::VmError::StaleHostRequest(id)) if id == request
    ));
    assert_eq!(vm.retire_terminal_fibers(), 1);
    assert_eq!(vm.spawn_entry(entry, Vec::new()).unwrap(), fiber);
}
