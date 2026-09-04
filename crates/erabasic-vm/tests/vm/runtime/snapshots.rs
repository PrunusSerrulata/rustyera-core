use super::*;
#[test]
#[allow(clippy::too_many_lines)] // Keep the snapshot format and exact-artifact checks together.
fn stable_wait_snapshot_round_trips_and_requires_exact_artifact() {
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    artifact.globals.push(global(
        SymbolKey::derive("test.snapshot", b"dense-zero-array"),
        vec![16_384],
    ));
    fixture_runtime_variables(&mut artifact);
    artifact.refresh_ids().unwrap();
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert_eq!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Eligible
    );
    let snapshot = vm.snapshot(&natives).unwrap();
    let bytes = snapshot.encode().unwrap();
    assert_eq!(vm.encode_snapshot(&natives).unwrap(), bytes);
    let inspection = inspect_snapshot(&bytes, VmConfig::default().maximum_snapshot_bytes).unwrap();
    assert_eq!(inspection.container.magic, "RERAVMS\\0");
    assert_eq!(inspection.container.file_bytes, bytes.len() as u64);
    assert_eq!(
        inspection.state["format_version"],
        erabasic_vm::SNAPSHOT_FORMAT_VERSION
    );
    assert_eq!(
        inspection.state["artifact_id"],
        artifact.manifest.artifact_id.to_string()
    );
    let inspection_json = serde_json::to_string(&inspection).unwrap();
    assert!(inspection_json.contains("rebind_payload"));
    assert!(inspection_json.contains("byte_length"));
    assert!(inspection_json.contains("blake3"));
    assert!(!inspection_json.contains("105,110,112,117,116,45,108,105,110,101"));
    let uncompressed_len =
        usize::try_from(u64::from_le_bytes(bytes[20..28].try_into().unwrap())).unwrap();
    assert!(
        uncompressed_len < 4_096,
        "default dense storage should use sparse snapshot encoding"
    );
    let mut understated = bytes.clone();
    understated[20..28].copy_from_slice(&((uncompressed_len as u64) - 1).to_le_bytes());
    let maximum = bytes.len().max(uncompressed_len);
    assert!(VmSnapshot::decode(&understated, maximum).is_err());
    let decoded = VmSnapshot::decode(&bytes, maximum).unwrap();
    assert_eq!(decoded.encode().unwrap(), bytes);
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        decoded.clone(),
        &mut restore_host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(restored.artifact_id(), artifact.manifest.artifact_id);
    assert_eq!(restore_host.rebound.len(), 1);

    let mut wrong_profile = artifact.clone();
    wrong_profile.manifest.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    wrong_profile.refresh_ids().unwrap();
    let rejected = Vm::restore_snapshot(
        validated(&wrong_profile),
        VmConfig::default(),
        decoded.clone(),
        &mut restore_host,
        &mut natives,
    );
    assert!(
        matches!(rejected, Err(VmError::Snapshot(message)) if message.contains("compatibility"))
    );
    assert_eq!(restore_host.rebound.len(), 1);

    let mut different = artifact.clone();
    different
        .source_map
        .sources
        .push(erabasic_bytecode::SourceRecord {
            relative_path: "other.erb".into(),
            content_hash: Digest::default(),
            byte_len: 0,
            line_starts: vec![0],
        });
    different.refresh_ids().unwrap();
    assert!(
        Vm::restore_snapshot(
            validated(&different),
            VmConfig::default(),
            decoded,
            &mut restore_host,
            &mut natives,
        )
        .is_err()
    );
}

fn snake_fault_artifact(source: &str) -> BytecodeArtifact {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatibility = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    compile_source_with_options(source, &options)
}

fn fault_from_report(report: &erabasic_vm::VmRunReport) -> erabasic_vm::VmFault {
    report
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::FiberFaulted { fault, .. } => Some(fault.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("final VM fault was not published: {report:#?}"))
}

#[test]
fn final_fault_hook_start_at_call_depth_attaches_resource_secondary() {
    let artifact = snake_fault_artifact(
        "@SYSTEM_TITLE\nCALL INNER\nRETURN\n@INNER\n#DIM X, 1\n#DIM INDEX\nINDEX = -1\nX:INDEX = 0\nRETURN\n@BEFORE_ERROR\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_call_depth: 2,
            ..VmConfig::default()
        },
    );
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fault = fault_from_report(&vm.run_slice(&mut host, &mut natives, RunBudget::default()));
    assert_eq!(
        fault.category,
        erabasic_vm::FaultCategory::Script(erabasic_vm::ScriptFaultKind::Bounds)
    );
    let secondary = fault.secondary.expect("hook start failure is secondary");
    assert_eq!(
        secondary.category,
        erabasic_vm::FaultCategory::ResourceLimit
    );
    assert_eq!(secondary.parent_correlation_id, Some(fault.correlation_id));
}

#[test]
fn final_fault_hook_watchdog_preserves_primary_without_recursion() {
    let artifact = snake_fault_artifact(
        "@SYSTEM_TITLE\n#DIM X, 1\n#DIM INDEX\nINDEX = -1\nX:INDEX = 0\nRETURN\n@BEFORE_ERROR\nWHILE 1\nWEND\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_backward_branches_without_progress: 8,
            ..VmConfig::default()
        },
    );
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fault = fault_from_report(&vm.run_slice(&mut host, &mut natives, RunBudget::default()));
    assert_eq!(fault.message, "variable index cannot be negative");
    let secondary = fault.secondary.expect("hook watchdog is secondary");
    assert_eq!(
        secondary.category,
        erabasic_vm::FaultCategory::ResourceLimit
    );
    assert!(secondary.secondary.is_none());
}

#[test]
fn prepared_reload_is_atomically_rejected_after_fault_hook_starts() {
    let artifact = snake_fault_artifact(
        "@SYSTEM_TITLE\n#DIM X, 1\n#DIM INDEX\nINDEX = -1\nX:INDEX = 0\nRETURN\n@BEFORE_ERROR\nINPUT\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.prepare_hot_reload_artifact(validated(&artifact))
        .unwrap();
    let generation = vm.current_generation();
    let artifact_id = vm.artifact_id();
    let pending_id = vm.pending_hot_reload().unwrap().target_artifact_id();
    let mut host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::HostPending { .. }))
    );
    let error = vm.commit_hot_reload().unwrap_err();
    assert!(error.to_string().contains("final-fault hook"), "{error}");
    assert_eq!(vm.current_generation(), generation);
    assert_eq!(vm.artifact_id(), artifact_id);
    assert_eq!(
        vm.pending_hot_reload().unwrap().target_artifact_id(),
        pending_id
    );
}

#[test]
fn postmortem_debug_keeps_original_fault_site_and_continue_stays_faulted() {
    let artifact = snake_fault_artifact(
        "@SYSTEM_TITLE\n#DIM X, 1\n#DIM INDEX\nINDEX = -1\nX:INDEX = 0\nRETURN\n@BEFORE_ERROR\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fault = fault_from_report(&vm.run_slice(&mut host, &mut natives, RunBudget::default()));
    assert_eq!(fault.function_name, "SYSTEM_TITLE");
    assert_eq!(fault.source.as_ref().map(|source| source.line), Some(5));
    let stop = vm.request_pause().unwrap();
    let frames = vm.call_stack(stop.token, fiber).unwrap();
    assert_eq!(frames.first().unwrap().function_name, "SYSTEM_TITLE");
    vm.continue_execution(stop.token).unwrap();
    assert!(
        vm.run_slice(&mut host, &mut natives, RunBudget::default())
            .events
            .is_empty()
    );
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::Faulted(actual)) if actual.correlation_id == fault.correlation_id)
    );
}

#[test]
fn cancelling_a_waiting_fault_hook_attaches_cancellation_secondary() {
    let artifact = snake_fault_artifact(
        "@SYSTEM_TITLE\n#DIM X, 1\n#DIM INDEX\nINDEX = -1\nX:INDEX = 0\nRETURN\n@BEFORE_ERROR\nINPUT\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmPortEvent::HostCall(_)))
    );
    runtime.cancel_fiber(fiber).unwrap();
    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    let fault = report
        .events
        .iter()
        .find_map(|event| match event {
            VmPortEvent::FiberFaulted(id, fault) if *id == fiber => Some(fault),
            _ => None,
        })
        .expect("cancelled hook fault event");
    assert_eq!(fault.message, "variable index cannot be negative");
    assert_eq!(
        fault.secondary.as_ref().map(|fault| fault.category),
        Some(erabasic_vm::FaultCategory::Cancellation)
    );
}

#[test]
fn quiescent_vm_snapshot_round_trips_without_host_wait_rebinding() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RESULT\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(_))
    ));
    assert_eq!(vm.retire_terminal_fibers(), 1);
    assert_eq!(vm.fiber_status(fiber), None);
    assert_eq!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Eligible
    );

    let snapshot = vm.snapshot(&natives).unwrap();
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let restored = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        snapshot,
        &mut restore_host,
        &mut natives,
    )
    .unwrap();
    assert!(restore_host.rebound.is_empty());
    assert_eq!(restored.fiber_status(fiber), None);
}

#[test]
fn retired_fiber_history_does_not_grow_quiescent_snapshots() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RESULT\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);

    vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    vm.retire_terminal_fibers();
    let baseline = vm.encode_snapshot(&natives).unwrap();
    for _ in 0..512 {
        assert_eq!(vm.spawn_entry(entry, Vec::new()).unwrap(), FiberId(1));
        vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert_eq!(vm.retire_terminal_fibers(), 1);
    }
    let after = vm.encode_snapshot(&natives).unwrap();

    assert_eq!(vm.fiber_ids().count(), 0);
    assert!(
        after.len() <= baseline.len().saturating_add(64),
        "retired fiber history grew the snapshot from {} to {} bytes",
        baseline.len(),
        after.len()
    );
}

#[test]
fn snapshot_restore_retires_completed_fiber_history_and_normalizes_ids() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RESULT\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let config = VmConfig {
        maximum_fibers: 1,
        ..VmConfig::default()
    };
    let mut vm = Vm::new(validated(&artifact), config);
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    for _ in 0..2 {
        vm.spawn_entry(entry, Vec::new()).unwrap();
        vm.run_slice(&mut host, &mut natives, RunBudget::default());
    }
    assert_eq!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Eligible
    );

    let snapshot = vm.snapshot(&natives).unwrap();
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut restored = Vm::restore_snapshot(
        validated(&artifact),
        config,
        snapshot,
        &mut restore_host,
        &mut natives,
    )
    .unwrap();
    assert_eq!(restored.fiber_ids().count(), 0);
    assert_eq!(restored.spawn_entry(entry, Vec::new()).unwrap(), FiberId(1));
}

#[test]
fn transient_qte_wait_cannot_be_snapshotted() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = PendingHost {
        stability: HostWaitStability::Transient,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::TransientHostWait(fiber))
    ));
    assert!(vm.encode_snapshot(&natives).is_err());

    let diagnostic_bytes = vm.encode_unrestricted_snapshot(&natives).unwrap();
    let diagnostic = VmSnapshot::decode(&diagnostic_bytes, diagnostic_bytes.len() * 2).unwrap();
    let mut restore_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let error = Vm::restore_snapshot(
        validated(&artifact),
        VmConfig::default(),
        diagnostic,
        &mut restore_host,
        &mut natives,
    )
    .err()
    .expect("an unrestricted transient snapshot must remain unrestorable");
    assert!(
        error
            .to_string()
            .contains("stable or quiescent primary fiber")
    );
}

#[test]
fn never_snapshot_capability_accepts_only_transient_host_waits() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::Never);
    let mut stable_vm = Vm::new(validated(&artifact), VmConfig::default());
    let stable_fiber = stable_vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut stable_host = PendingHost {
        stability: HostWaitStability::StableInput,
        rebound: Vec::new(),
    };
    let mut natives = NativeServiceRegistry::default();
    stable_vm.run_slice(&mut stable_host, &mut natives, RunBudget::default());
    assert!(matches!(
        stable_vm.fiber_status(stable_fiber),
        Some(FiberStatus::Faulted(ref fault)) if fault.code == VmFaultCode::Host
    ));

    let mut transient_vm = Vm::new(validated(&artifact), VmConfig::default());
    let transient_fiber = transient_vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut transient_host = PendingHost {
        stability: HostWaitStability::Transient,
        rebound: Vec::new(),
    };
    transient_vm.run_slice(&mut transient_host, &mut natives, RunBudget::default());
    assert!(matches!(
        transient_vm.fiber_status(transient_fiber),
        Some(FiberStatus::WaitingHost(_))
    ));
    assert!(matches!(
        transient_vm.snapshot_eligibility(&natives),
        SnapshotEligibility::Ineligible(ref blockers)
            if blockers.contains(&SnapshotBlocker::TransientHostWait(transient_fiber))
    ));
}

#[test]
fn host_resume_is_typed_and_late_responses_are_stale() {
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
    vm.resume_host(request, HostReady::empty()).unwrap();
    assert!(vm.resume_host(request, HostReady::empty()).is_err());
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
}
