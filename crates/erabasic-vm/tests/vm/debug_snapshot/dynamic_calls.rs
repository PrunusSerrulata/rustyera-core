use super::*;
#[test]
fn invalid_compatibility_warning_sites_reject_before_restoring_native_random_state() {
    let artifact = compile_source_with_options(
        concat!(
            "@SYSTEM_TITLE\nRESULT:10 = RAND:1000000\n",
            "FLAG:0 = 9223372036854775807\nRESULT:11 = FLAG:0 + 1\n",
            "RESULT:12 = 1 / FLAG:1\nWAIT\nRETURN RESULT\n",
        ),
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::default()
        },
    );
    assert!(
        artifact
            .native_imports
            .iter()
            .any(|import| import.import.name == "rand")
    );
    let entry = &artifact.functions[0];
    let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry.key, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::WaitingHost(_))),
        "{report:?}"
    );
    let snapshot = vm.snapshot(&natives).unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    let sites = json["compatibility_warning_sites"].as_array().unwrap();
    assert_eq!(sites.len(), 2, "overflow and division-by-zero sites");
    assert_eq!(sites[0][3], 0);
    assert_eq!(sites[1][3], 1);
    assert!(!json["native_states"].as_array().unwrap().is_empty());

    for corruption in ["generation", "function", "instruction", "tag"] {
        let mut corrupted = json.clone();
        corrupt_arithmetic_warning_site(
            &mut corrupted["compatibility_warning_sites"][0],
            corruption,
            entry.code.len(),
        );
        let corrupted: VmSnapshot = serde_json::from_value(corrupted).unwrap();
        let mut restored_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
        let before = serde_json::to_value(vm.snapshot(&restored_natives).unwrap()).unwrap();
        // Different seeds make an accidental restore observable even if it later fails.
        assert_ne!(before["native_states"], json["native_states"]);
        let mut rejected_host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let rejected = Vm::restore_snapshot(
            validated(&artifact),
            VmConfig::default(),
            corrupted,
            &mut rejected_host,
            &mut restored_natives,
        );
        assert!(
            matches!(&rejected, Err(VmError::Snapshot(message)) if message.contains("compatibility diagnostic identity")),
            "{corruption}: {:?}",
            rejected.as_ref().err()
        );
        assert!(rejected_host.rebound.is_empty(), "{corruption}");
        let after = serde_json::to_value(vm.snapshot(&restored_natives).unwrap()).unwrap();
        assert_eq!(
            after["native_states"], before["native_states"],
            "{corruption}"
        );
    }

    let encoded = snapshot.encode().unwrap();
    let decoded = VmSnapshot::decode(&encoded, VmConfig::default().maximum_snapshot_bytes).unwrap();
    let mut restored_natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
    let mut restored_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        decoded,
        &mut restored_host,
        &mut restored_natives,
    )
    .unwrap();
    assert_eq!(restored_host.rebound.len(), 1);
    let restored = serde_json::to_value(restored.snapshot(&restored_natives).unwrap()).unwrap();
    assert_eq!(
        restored["compatibility_warning_sites"],
        json["compatibility_warning_sites"]
    );
    assert_eq!(restored["native_states"], json["native_states"]);
}

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
        "../../../../../tools/runtime-tester/fixture-reference/erb/oracle.erb"
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
