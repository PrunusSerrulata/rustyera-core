use super::*;
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
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let artifact = compile_source_with_options(
            "@SYSTEM_TITLE\nDUMPRAND\nRESULT:0 = RAND:1000000\nRANDOMIZE 4321\nINITRAND\nRESULT:1 = RAND:1000000\nRETURN RESULT\n",
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::default()
            },
        );
        let entry = artifact.functions[0].key;
        let global_key = |name| {
            artifact
                .globals
                .iter()
                .find(|global| global.name == name)
                .unwrap()
                .key
        };
        let result = global_key("RESULT");
        let randdata = global_key("RANDDATA");
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
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
        assert!(matches!(
            vm.read_variable(randdata, &[0], None),
            Ok(VmValue::Integer(value)) if value != 0
        ));
        assert_eq!(
            vm.read_variable(randdata, &[624], None),
            Ok(VmValue::Integer(624))
        );
        assert_eq!(
            vm.read_variable(result, &[0], None),
            vm.read_variable(result, &[1], None),
            "{profile}",
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn rand_stream_survives_stable_snapshot_and_temporary_reseed_replay() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let artifact = compile_source_with_options(
            concat!(
                "@SYSTEM_TITLE\nRESULT:10 = RAND:1000000000\nDUMPRAND\nWAIT\n",
                "IF FLAG:0\nRANDOMIZE 4321\nRESULT:9 = RAND:1000000000\nINITRAND\nENDIF\n",
                "RESULT:11 = RAND:1000000000\nRESULT:12 = RAND:1000000000\n",
                "RESULT:13 = RAND:1000000000\nDUMPRAND\nWAIT\nRETURN RESULT\n",
            ),
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::default()
            },
        );
        let key = |name| {
            artifact
                .globals
                .iter()
                .find(|global| global.name == name)
                .unwrap()
                .key
        };
        let observed = |vm: &Vm| {
            let samples = (11..14)
                .map(|index| vm.read_variable(key("RESULT"), &[index], None).unwrap())
                .collect::<Vec<_>>();
            let state = (0..625)
                .map(|index| vm.read_variable(key("RANDDATA"), &[index], None).unwrap())
                .collect::<Vec<_>>();
            (samples, state)
        };
        let mut host = PendingHost {
            stability: HostWaitStability::StableInput,
            rebound: Vec::new(),
        };
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&artifact, 1234);
        let mut vm = Vm::new(validated(&artifact), VmConfig::default());
        let fiber = vm
            .spawn_entry(artifact.functions[0].key, Vec::new())
            .unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        let Some(FiberStatus::WaitingHost(request)) = vm.fiber_status(fiber) else {
            panic!("{profile}: expected first stable wait: {report:?}");
        };
        let saved = vm.encode_snapshot(&natives).unwrap();
        vm.resume_host(request, HostReady::empty()).unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(
            matches!(vm.fiber_status(fiber), Some(FiberStatus::WaitingHost(_))),
            "{report:?}"
        );
        let expected = observed(&vm);

        for temporary_replay in [false, true] {
            let mut restored_natives =
                NativeServiceRegistry::for_artifact_with_seed(&artifact, 9876);
            let mut restore_host = PendingHost {
                stability: HostWaitStability::StableInput,
                rebound: Vec::new(),
            };
            let mut restored = Vm::restore_snapshot(
                validated(&artifact),
                VmConfig::default(),
                VmSnapshot::decode(&saved, VmConfig::default().maximum_snapshot_bytes).unwrap(),
                &mut restore_host,
                &mut restored_natives,
            )
            .unwrap();
            assert_eq!(restore_host.rebound.len(), 1);
            restored
                .write_variable(
                    key("FLAG"),
                    &[0],
                    None,
                    VmValue::Integer(i64::from(temporary_replay)),
                )
                .unwrap();
            let Some(FiberStatus::WaitingHost(request)) = restored.fiber_status(fiber) else {
                panic!("restored RAND artifact must retain the stable wait");
            };
            restored.resume_host(request, HostReady::empty()).unwrap();
            let report = restored.run_slice(
                &mut restore_host,
                &mut restored_natives,
                RunBudget::default(),
            );
            assert!(
                matches!(
                    restored.fiber_status(fiber),
                    Some(FiberStatus::WaitingHost(_))
                ),
                "{report:?}"
            );
            assert_eq!(
                observed(&restored),
                expected,
                "{profile}: temporary replay={temporary_replay}"
            );
        }
    }
}
