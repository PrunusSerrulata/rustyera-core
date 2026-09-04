use super::*;
fn column_identity_artifact() -> BytecodeArtifact {
    compile_source(
        r#"@WAITING
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int32"
INPUT
RETURN RESULT:0
@ADD
DT_COLUMN_ADD "t", "other", "int32"
RETURN RESULT:0
@REMOVE
DT_COLUMN_REMOVE "t", "value"
RETURN RESULT:0
@REPLACE
DT_COLUMN_REMOVE "t", "value"
DT_COLUMN_ADD "t", "value", "string"
RETURN RESULT:0
@RELEASE
DT_RELEASE "t"
DT_CREATE "t"
RETURN RESULT:0
"#,
    )
}

fn run_identity_entry(runtime: &mut RuntimeVm, artifact: &BytecodeArtifact, name: &str) -> FiberId {
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap()
        .key;
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    fiber
}

#[test]
fn candidate_state_rejects_stale_column_identities_without_mutating_live_frames() {
    let artifact = column_identity_artifact();
    for change in ["ADD", "REMOVE", "REPLACE", "RELEASE"] {
        let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
        let waiting = run_identity_entry(&mut live, &artifact, "WAITING");
        let candidate = live
            .fork_isolated()
            .unwrap()
            .into_candidate_state()
            .unwrap();
        run_identity_entry(&mut live, &artifact, change);
        assert!(live.fiber_frame_count(waiting).unwrap() > 0);
        let before = live.encode_unrestricted_snapshot().unwrap();
        let error = live.commit_candidate_state(candidate).unwrap_err();
        assert!(
            error.to_string().contains("stale column identity"),
            "{error}"
        );
        assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    }
}

#[test]
fn hot_reload_rejects_removing_a_map_provider_owned_by_an_old_generation() {
    let artifact = map_lifecycle_artifact();
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let waiter = run_identity_entry(&mut runtime, &artifact, "WAITING_MAP");
    let Some(FiberStatus::WaitingHost(request)) = runtime.vm().fiber_status(waiter) else {
        panic!("MAP tail did not wait");
    };
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    let target = compile_source_with_options("@SYSTEM_TITLE\nRETURN\n", &options);
    let before = runtime.encode_unrestricted_snapshot().unwrap();
    let error = runtime.prepare_hot_reload(validated(&target)).unwrap_err();
    assert!(error.to_string().contains("active continuation"), "{error}");
    assert_eq!(runtime.encode_unrestricted_snapshot().unwrap(), before);

    let ready = runtime
        .validate_host_completion(request, VmHostCompletion::Ready(HostReady::empty()))
        .unwrap();
    runtime.commit_host_completion(ready).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::FiberFaulted(..))),
        "{report:?}"
    );
}

#[test]
fn runtime_transaction_rejects_stale_column_identities_before_installing_memory() {
    let artifact = column_identity_artifact();
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT" && global.owner.is_none())
        .unwrap()
        .key;
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    run_identity_entry(&mut live, &artifact, "WAITING");
    let prepared = live
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![erabasic_vm::VmRuntimeWrite {
                variable: result,
                indices: vec![30],
                character: None,
                value: VmValue::Integer(99),
            }],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .unwrap();
    run_identity_entry(&mut live, &artifact, "REPLACE");
    let before = live.encode_unrestricted_snapshot().unwrap();
    let error = live.commit_runtime_state(prepared).unwrap_err();
    assert!(
        error.to_string().contains("stale column identity"),
        "{error}"
    );
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
}

#[test]
fn native_hot_reload_rejects_stale_column_identities_before_switching_generation() {
    let artifact = column_identity_artifact();
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let waiting = run_identity_entry(&mut live, &artifact, "WAITING");
    let generation = live.current_generation();
    live.prepare_hot_reload(validated(&artifact)).unwrap();
    run_identity_entry(&mut live, &artifact, "REMOVE");
    let before = live.encode_unrestricted_snapshot().unwrap();
    let error = live.commit_hot_reload().unwrap_err();
    assert!(
        error.to_string().contains("stale column identity"),
        "{error}"
    );
    assert_eq!(live.current_generation(), generation);
    assert!(live.fiber_frame_count(waiting).unwrap() > 0);
    assert_eq!(live.encode_unrestricted_snapshot().unwrap(), before);
    live.vm_mut().abort_hot_reload();
    live.prepare_hot_reload(validated(&artifact)).unwrap();
    live.commit_hot_reload().unwrap();
    assert_ne!(live.current_generation(), generation);
}

#[test]
fn isolated_candidate_can_create_columns_when_its_base_identity_is_still_current() {
    let artifact = column_identity_artifact();
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let waiting = run_identity_entry(&mut live, &artifact, "WAITING");
    let mut candidate = live.fork_isolated().unwrap();
    run_identity_entry(&mut candidate, &artifact, "ADD");
    live.commit_candidate_state(candidate.into_candidate_state().unwrap())
        .unwrap();
    assert!(live.fiber_frame_count(waiting).unwrap() > 0);
    // A subsequent mutation and snapshot both observe one valid allocator timeline.
    run_identity_entry(&mut live, &artifact, "REPLACE");
    live.encode_unrestricted_snapshot().unwrap();
}

#[test]
fn host_fault_delivery_is_once_and_blocks_snapshot_fork_and_candidate_commit() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let candidate = runtime
        .fork_isolated()
        .unwrap()
        .into_candidate_state()
        .unwrap();
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let request = runtime
        .drive(RunBudget::default(), VmDriveMode::Normal)
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .unwrap();
    let error = erabasic_vm::ExecutionFailure::classified(
        erabasic_vm::FaultCategory::HostContract,
        erabasic_vm::VmFaultCode::Host,
        "bad provider contract",
    );
    let prepared = runtime
        .validate_host_completion(request.id, VmHostCompletion::Error(error))
        .unwrap();
    runtime.commit_host_completion(prepared).unwrap();
    assert!(runtime.has_pending_events());
    assert!(runtime.has_work());
    assert!(!runtime.has_runnable_fibers());
    assert_eq!(runtime.retire_terminal_fibers(), 0);
    assert_eq!(
        runtime.snapshot_eligibility(),
        SnapshotEligibility::Ineligible(vec![
            erabasic_vm::SnapshotBlocker::PendingCompletionEvents,
        ])
    );
    assert!(runtime.encode_unrestricted_snapshot().is_err());
    assert!(runtime.fork_isolated().is_err());
    assert!(runtime.prepare_hot_reload(validated(&artifact)).is_err());
    assert!(runtime.commit_candidate_state(candidate).is_err());
    assert!(matches!(
        runtime.validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty())),
        Err(VmError::StaleHostRequest(_))
    ));
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert_eq!(report.instructions, 0);
    let [VmPortEvent::FiberFaulted(actual, fault)] = report.events.as_slice() else {
        panic!("one original fault must be delivered: {report:?}");
    };
    assert_eq!(*actual, fiber);
    assert_eq!(fault.category, erabasic_vm::FaultCategory::HostContract);
    assert_eq!(fault.generation, request.origin.generation);
    assert_eq!(fault.function, request.origin.function);
    assert_eq!(fault.instruction, request.origin.instruction);
    assert!(!runtime.has_work());
    assert!(
        runtime
            .drive(RunBudget::default(), VmDriveMode::Normal)
            .events
            .is_empty()
    );
}

#[test]
fn candidate_extraction_rejects_an_undelivered_host_fault() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    runtime.spawn_entry(entry, Vec::new()).unwrap();
    let request = runtime
        .drive(RunBudget::default(), VmDriveMode::Normal)
        .events
        .into_iter()
        .find_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .unwrap();
    let prepared = runtime
        .validate_host_completion(
            request.id,
            VmHostCompletion::Error(erabasic_vm::ExecutionFailure::classified(
                erabasic_vm::FaultCategory::HostContract,
                erabasic_vm::VmFaultCode::Host,
                "bad provider contract",
            )),
        )
        .unwrap();
    runtime.commit_host_completion(prepared).unwrap();
    assert!(runtime.into_candidate_state().is_err());
}

#[test]
#[allow(clippy::too_many_lines)] // One shared host exercises all return continuation modes.
fn host_return_current_preserves_dynamic_jump_and_event_continuations() {
    for (entry_call, expected_after, expected_later) in [
        ("CALLSTR \"OWNER\"", 1, 0),
        ("JUMPSTR \"OWNER\"", 0, 0),
        ("CALLEVENT EVENTFIRST", 1, 1),
    ] {
        let source = format!(
            r#"@SYSTEM_TITLE
{entry_call}
FLAG:9 = 1
RETURN
@OWNER
JUMPSTR "NEXT"
FLAG:8 = 99
RETURN
@NEXT
FLAG:1 = 7
INPUT
FLAG:2 = 99
RETURN
@EVENTFIRST
#PRI
JUMPSTR "NEXT"
FLAG:8 = 99
RETURN
@EVENTFIRST
#LATER
FLAG:3 += 1
RETURN
"#
        );
        let artifact = compile_source_with_options(
            &source,
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                    erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                ),
                ..AnalyzerOptions::default()
            },
        );
        let entry = artifact
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap()
            .key;
        let flag = artifact
            .globals
            .iter()
            .find(|variable| variable.name == "FLAG")
            .unwrap()
            .key;
        let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
        let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
        let before = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
        let request = before
            .events
            .into_iter()
            .find_map(|event| match event {
                VmPortEvent::HostCall(request) => Some(request),
                _ => None,
            })
            .expect("dynamic child must reach INPUT");
        assert!(request.import.import.name.eq_ignore_ascii_case("INPUT"));
        let completion = runtime
            .validate_host_completion(request.id, VmHostCompletion::ReturnCurrent(None))
            .unwrap();
        runtime.commit_host_completion(completion).unwrap();
        assert!(
            runtime
                .validate_host_completion(request.id, VmHostCompletion::ReturnCurrent(None))
                .is_err()
        );
        let after = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
        assert_eq!(
            after
                .events
                .iter()
                .filter(|event| matches!(event,
            VmPortEvent::FiberCompleted(id, _) if *id == fiber))
                .count(),
            1,
            "{entry_call}: {after:?}"
        );
        assert!(
            !after
                .events
                .iter()
                .any(|event| matches!(event, VmPortEvent::FiberFaulted(_, _)))
        );
        for (index, expected) in [
            (1, 7),
            (2, 0),
            (3, expected_later),
            (8, 0),
            (9, expected_after),
        ] {
            assert_eq!(
                runtime.vm().read_variable(flag, &[index], None),
                Ok(VmValue::Integer(expected)),
                "{entry_call} FLAG:{index}"
            );
        }
        assert!(
            runtime
                .drive(RunBudget::default(), VmDriveMode::Normal)
                .events
                .is_empty()
        );
    }
}

fn map_lifecycle_artifact() -> BytecodeArtifact {
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    compile_source_with_options(
        r#"@WAITING_MAP
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "a", "old")
RESULTS:10 '= MAP_TOSTRING("m", MAP_WAIT())
FLAG:9 = 1
RETURN
@CHECK_WAITING_MAP
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "a", "old")
RESULT:10 = STRFORMCHECK("%MAP_TOSTRING(\"m\", MAP_WAIT())%")
FLAG:9 = 1
RETURN
@MAP_WAIT
#FUNCTIONS
INPUT
RETURNF "|"
@UPDATE_MAP
RESULT = MAP_SET("m", "a", "new")
RETURN
@RECREATE_MAP
RESULT = MAP_RELEASE("m")
RESULT = MAP_CREATE("m")
RESULT = MAP_SET("m", "b", "fresh")
RETURN
@READ_MAP
RESULTS:20 '= MAP_TOSTRING("m")
RETURN
"#,
        &options,
    )
}

#[test]
fn pending_map_candidate_rejects_stale_object_updates_and_recreated_bindings() {
    let artifact = map_lifecycle_artifact();
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS" && global.owner.is_none())
        .unwrap()
        .key;
    for (change, captured, current) in [
        ("UPDATE_MAP", "a=new", "a=new"),
        ("RECREATE_MAP", "a=old", "b=fresh"),
    ] {
        let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
        let waiter = run_identity_entry(&mut live, &artifact, "WAITING_MAP");
        let Some(FiberStatus::WaitingHost(request)) = live.vm().fiber_status(waiter) else {
            panic!("MAP tail did not wait");
        };
        let candidate = live.fork_isolated().unwrap();
        assert!(matches!(
            candidate.snapshot_eligibility(),
            SnapshotEligibility::Ineligible(ref blockers)
                if blockers.iter().any(|blocker| matches!(
                    blocker,
                    SnapshotBlocker::NativeService(message)
                        if message.contains("candidate MAP roots")
                ))
        ));
        assert!(candidate.snapshot().is_err());
        assert!(candidate.encode_snapshot().is_err());
        let candidate = candidate.into_candidate_state().unwrap();
        run_identity_entry(&mut live, &artifact, change);
        let before = live.encode_unrestricted_snapshot().unwrap();
        assert!(matches!(
            live.commit_candidate_state(candidate),
            Err(VmError::InvalidState(_))
        ));
        assert_eq!(
            live.encode_unrestricted_snapshot().unwrap(),
            before,
            "{change}: failed commit changed live state"
        );
        assert!(live.fiber_frame_count(waiter).unwrap() >= 2);
        run_identity_entry(&mut live, &artifact, "READ_MAP");
        assert_eq!(
            live.vm().read_variable(results, &[20], None).unwrap(),
            VmValue::String(current.into())
        );
        let ready = live
            .validate_host_completion(request, VmHostCompletion::Ready(HostReady::empty()))
            .unwrap();
        live.commit_host_completion(ready).unwrap();
        let resumed = live.drive(RunBudget::default(), VmDriveMode::Normal);
        assert!(
            !resumed
                .events
                .iter()
                .any(|event| matches!(event, VmPortEvent::FiberFaulted(..))),
            "{change}: {resumed:?}"
        );
        assert!(
            resumed
                .events
                .iter()
                .any(|event| matches!(event, VmPortEvent::FiberCompleted(id, _) if *id == waiter)),
            "{change}: {resumed:?}"
        );
        assert_eq!(
            live.vm().read_variable(results, &[10], None).unwrap(),
            VmValue::String(captured.into())
        );
    }
}

/// Inspect the raw encoder before any eligibility/snapshot call can prune leases.
/// Restore uses the existing snapshot/Native validators, not a test MAP decoder.
fn assert_map_cleanup_can_restore_raw_state(
    runtime: &RuntimeVm,
    artifact: &BytecodeArtifact,
    expected_rebinds: usize,
) {
    let bytes = runtime.encode_unrestricted_snapshot().unwrap();
    let inspection = inspect_snapshot(&bytes, VmConfig::default().maximum_snapshot_bytes).unwrap();
    for fiber in inspection.state["fibers"].as_object().unwrap().values() {
        for frame in fiber["frames"].as_array().unwrap() {
            assert!(frame["map_calls"].as_array().unwrap().is_empty());
            assert!(frame["runtime_form"].is_null());
        }
    }
    let snapshot = VmSnapshot::decode(&bytes, VmConfig::default().maximum_snapshot_bytes).unwrap();
    let mut natives = NativeServiceRegistry::for_artifact(artifact);
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut restored = Vm::restore_snapshot(
        validated(artifact),
        VmConfig::default(),
        snapshot,
        &mut host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(host.rebound.len(), expected_rebinds);
    let read = artifact
        .functions
        .iter()
        .find(|function| function.name == "READ_MAP")
        .unwrap()
        .key;
    let reader = restored.spawn_entry(read, Vec::new()).unwrap();
    let report = restored.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber, .. } if *fiber == reader)
        ),
        "{report:?}"
    );
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS" && global.owner.is_none())
        .unwrap()
        .key;
    assert_eq!(
        restored.read_variable(results, &[20], None).unwrap(),
        VmValue::String("a=old".into())
    );
}

#[test]
#[allow(clippy::too_many_lines)] // All termination modes share the same pending MAP fixture.
fn pending_map_cancel_host_error_and_check_recovery_release_native_and_frame_leases() {
    let artifact = map_lifecycle_artifact();
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG" && global.owner.is_none())
        .unwrap()
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT" && global.owner.is_none())
        .unwrap()
        .key;
    for ending in ["cancel", "host_contract", "script_check"] {
        let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
        let entry = if ending == "script_check" {
            "CHECK_WAITING_MAP"
        } else {
            "WAITING_MAP"
        };
        let fiber = run_identity_entry(&mut runtime, &artifact, entry);
        let Some(FiberStatus::WaitingHost(request)) = runtime.vm().fiber_status(fiber) else {
            panic!("MAP tail did not reach INPUT");
        };
        let waiting = inspect_snapshot(
            &runtime.encode_unrestricted_snapshot().unwrap(),
            VmConfig::default().maximum_snapshot_bytes,
        )
        .unwrap()
        .state;
        let frames = waiting["fibers"][fiber.0.to_string()]["frames"]
            .as_array()
            .unwrap();
        if ending == "script_check" {
            assert!(frames.iter().any(|frame| !frame["runtime_form"].is_null()));
        } else {
            assert!(
                frames
                    .iter()
                    .any(|frame| !frame["map_calls"].as_array().unwrap().is_empty())
            );
        }
        if ending == "cancel" {
            runtime.cancel_fiber(fiber).unwrap();
            assert_eq!(
                runtime.vm().fiber_status(fiber),
                Some(FiberStatus::Cancelled)
            );
        } else {
            let failure = if ending == "script_check" {
                erabasic_vm::ExecutionFailure::script(
                    erabasic_vm::ScriptFaultKind::Operation,
                    VmFaultCode::Host,
                    "test script input failure",
                )
            } else {
                erabasic_vm::ExecutionFailure::classified(
                    erabasic_vm::FaultCategory::HostContract,
                    VmFaultCode::Host,
                    "test provider contract failure",
                )
            };
            let completion = runtime
                .validate_host_completion(request, VmHostCompletion::Error(failure))
                .unwrap();
            runtime.commit_host_completion(completion).unwrap();
            let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
            if ending == "script_check" {
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmPortEvent::FiberFaulted(..))),
                    "{report:?}"
                );
                assert!(
                    report.events.iter().any(
                        |event| matches!(event, VmPortEvent::FiberCompleted(id, _) if *id == fiber)
                    ),
                    "{report:?}"
                );
                assert_eq!(
                    runtime.vm().read_variable(result, &[10], None).unwrap(),
                    VmValue::Integer(0)
                );
                assert_eq!(
                    runtime.vm().read_variable(flag, &[9], None).unwrap(),
                    VmValue::Integer(1)
                );
            } else {
                assert!(
                    report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmPortEvent::FiberFaulted(id, fault)
                    if *id == fiber && fault.category == erabasic_vm::FaultCategory::HostContract)),
                    "{report:?}"
                );
                assert_eq!(
                    runtime.vm().read_variable(flag, &[9], None).unwrap(),
                    VmValue::Integer(0)
                );
            }
        }
        let expected_rebinds = if ending == "host_contract" {
            let primary = run_identity_entry(&mut runtime, &artifact, "MAP_WAIT");
            let Some(FiberStatus::WaitingHost(request)) = runtime.vm().fiber_status(primary) else {
                panic!("stable MAP cleanup primary did not reach INPUT");
            };
            let prepared = runtime
                .validate_host_completion(
                    request,
                    VmHostCompletion::Pending {
                        stability: HostWaitStability::StableInput,
                        rebind_payload: Vec::new(),
                    },
                )
                .unwrap();
            runtime.commit_host_completion(prepared).unwrap();
            1
        } else {
            0
        };
        assert_map_cleanup_can_restore_raw_state(&runtime, &artifact, expected_rebinds);
    }
}
