use super::*;

#[test]
fn stable_wait_snapshot_round_trips_and_requires_exact_artifact() {
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    artifact.globals.push(global(
        SymbolKey::derive("test.snapshot", b"dense-zero-array"),
        vec![16_384],
    ));
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
    let direct = vm.encode_snapshot(&natives).unwrap();
    let snapshot = vm.snapshot(&natives).unwrap();
    let bytes = snapshot.encode().unwrap();
    assert_eq!(direct, bytes);
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

#[test]
fn persistent_budget_exhaustion_trips_the_watchdog() {
    let entry = SymbolKey::derive("test.function", b"loop");
    let artifact = artifact(
        vec![function(entry, "LOOP", vec![opcode::jump(Opcode::Jump, 0)])],
        Vec::new(),
    );
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_consecutive_budget_exhaustions: 1,
            ..VmConfig::default()
        },
    );
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let first_report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 4,
            maximum_host_calls: 0,
            fiber_quantum: 2,
        },
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Runnable)
    ));
    assert!(first_report.events.is_empty());
    let report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 4,
            maximum_host_calls: 0,
            fiber_quantum: 2,
        },
    );
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Faulted(ref fault)) if fault.code == VmFaultCode::RunawayExecution
    ));
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::RunawayExecution
    )));
}

#[test]
fn finite_work_spanning_many_fiber_quanta_does_not_trip_budget_watchdog() {
    let entry = SymbolKey::derive("test.function", b"finite");
    let mut code = vec![erabasic_bytecode::EncodedInstruction::new(Opcode::Nop, Vec::new()); 8];
    code.push(opcode::return_value(false));
    let artifact = artifact(vec![function(entry, "FINITE", code)], Vec::new());
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_consecutive_budget_exhaustions: 1,
            ..VmConfig::default()
        },
    );
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 32,
            maximum_host_calls: 0,
            fiber_quantum: 2,
        },
    );

    assert_eq!(report.instructions, 9);
    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
    assert!(!report.events.iter().any(|event| matches!(
        event,
        VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::RunawayExecution
    )));
}

#[test]
fn finite_work_spanning_many_budgets_makes_progress_at_function_returns() {
    let entry = SymbolKey::derive("test.function", b"finite-caller");
    let helper = SymbolKey::derive("test.function", b"finite-helper");
    let mut caller = function(
        entry,
        "FINITE_CALLER",
        vec![
            opcode::call(Opcode::Call, 0, 0, None),
            opcode::call(Opcode::Call, 0, 0, None),
            opcode::call(Opcode::Call, 0, 0, None),
            opcode::return_value(false),
        ],
    );
    caller.imports.push(FunctionImport {
        kind: ImportKind::Function,
        key: helper,
    });
    let helper = function(helper, "FINITE_HELPER", vec![opcode::return_value(false)]);
    let artifact = artifact(vec![caller, helper], Vec::new());
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_consecutive_budget_exhaustions: 1,
            ..VmConfig::default()
        },
    );
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    for _ in 0..4 {
        vm.run_slice(
            &mut host,
            &mut natives,
            RunBudget {
                maximum_instructions: 2,
                maximum_host_calls: 0,
                fiber_quantum: 2,
            },
        );
    }

    assert!(matches!(
        vm.fiber_status(fiber),
        Some(FiberStatus::Completed(None))
    ));
}

fn call_artifact(
    helper_value: i64,
    dimensions: Vec<u64>,
) -> (BytecodeArtifact, SymbolKey, SymbolKey) {
    let main = SymbolKey::derive("test.function", b"main");
    let helper = SymbolKey::derive("test.function", b"helper");
    let variable = SymbolKey::derive("test.variable", b"value");
    let mut main_function = function(
        main,
        "MAIN",
        vec![
            opcode::call(Opcode::Call, 0, 0, Some(BytecodeType::Integer)),
            opcode::return_value(true),
        ],
    );
    main_function.result = Some(BytecodeType::Integer);
    main_function.imports.push(FunctionImport {
        kind: ImportKind::Function,
        key: helper,
    });
    let mut helper_function = function(
        helper,
        "HELPER",
        vec![
            opcode::push_integer(helper_value),
            opcode::return_value(true),
        ],
    );
    helper_function.result = Some(BytecodeType::Integer);
    (
        artifact(
            vec![main_function, helper_function],
            vec![global(variable, dimensions)],
        ),
        main,
        variable,
    )
}

#[test]
fn hot_reload_pins_old_stacks_and_migrates_compatible_state() {
    let (base, entry, variable) = call_artifact(1, vec![2]);
    let (target, _, _) = call_artifact(2, vec![3]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(7))
        .unwrap();
    vm.write_variable(variable, &[1], None, VmValue::Integer(8))
        .unwrap();
    let old = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1,
            maximum_host_calls: 0,
            fiber_quantum: 1,
        },
    );
    vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
        .unwrap();
    vm.commit_hot_reload().unwrap();
    let new = vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(old),
        Some(FiberStatus::Completed(Some(VmValue::Integer(1))))
    ));
    assert!(matches!(
        vm.fiber_status(new),
        Some(FiberStatus::Completed(Some(VmValue::Integer(2))))
    ));
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(7)
    );
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(8)
    );
    assert_eq!(
        vm.read_variable(variable, &[2], None).unwrap(),
        VmValue::Integer(0)
    );
}

#[test]
fn debugger_pause_step_and_variable_batch_are_coherent_and_atomic() {
    let (artifact, entry, _) = call_artifact(7, vec![1]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let stop = vm.request_pause().unwrap();
    let frame = vm
        .call_stack(stop.token, fiber)
        .unwrap()
        .into_iter()
        .next()
        .expect("root frame");
    assert!(
        vm.operand_stack(stop.token, fiber, frame.id, None, 32)
            .unwrap()
            .values
            .is_empty()
    );
    let page = vm.variables(stop.token, None, 32).unwrap();
    let variable = page.values.first().expect("project variable").clone();
    let mut invalid_target = variable.target.clone();
    invalid_target.target.indices[0] = 99;
    assert!(
        vm.write_variables(
            stop.token,
            &[
                VmDebugVariableWrite {
                    target: variable.target.clone(),
                    value: VmValue::Integer(41),
                    expected_revision: variable.revision,
                },
                VmDebugVariableWrite {
                    target: invalid_target,
                    value: VmValue::Integer(42),
                    expected_revision: variable.revision,
                },
            ],
        )
        .is_err()
    );
    assert_eq!(
        VmDebugInspect::read_variable(&vm, stop.token, &variable.target)
            .unwrap()
            .value,
        VmValue::Integer(0)
    );

    vm.step(stop.token, fiber, VmStepKind::Instruction).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::DebugStopped(_))),
        "{:#?}",
        report.events
    );
}

#[test]
fn incompatible_hot_reload_is_rejected_atomically() {
    let (base, _, variable) = call_artifact(1, vec![2]);
    let mut target = base.clone();
    target.globals[0].value_type = BytecodeType::String;
    target.refresh_ids().unwrap();
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(11))
        .unwrap();
    let original_id = vm.artifact_id();
    assert!(
        vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
            .is_err()
    );
    assert!(vm.pending_hot_reload().is_none());
    assert_eq!(vm.artifact_id(), original_id);
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(11)
    );
}

#[test]
fn function_breakpoints_rebind_to_the_new_hot_reload_generation() {
    let (base, entry, _) = call_artifact(1, vec![1]);
    let (target, _, _) = call_artifact(2, vec![1]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.update_breakpoints(
        &[VmBreakpoint {
            id: 9,
            enabled: true,
            hit_count: 0,
            location: VmBreakpointLocation::Function(entry),
        }],
        &[],
    )
    .unwrap();
    vm.prepare_hot_reload(&patch, &ValidationContext::for_artifact(&target))
        .unwrap();
    vm.commit_hot_reload().unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::DebugStopped(stop)
            if matches!(stop.reason, erabasic_vm::VmDebugStopReason::Breakpoint(9))
    )));
}

#[test]
fn traditional_state_overlay_restores_persistent_arrays_without_stacks() {
    let entry = SymbolKey::derive("test.function", b"save");
    let variable = SymbolKey::derive("test.variable", b"save");
    let artifact = artifact(
        vec![function(entry, "SAVE", vec![opcode::return_value(false)])],
        vec![global(variable, vec![2])],
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(variable, &[1], None, VmValue::Integer(42))
        .unwrap();
    let save = vm.export_era_state();
    vm.write_variable(variable, &[1], None, VmValue::Integer(0))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.reset_with_era_state(&save).unwrap();
    assert_eq!(report.restored_variables, 1);
    assert_eq!(vm.fiber_ids().count(), 0);
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(42)
    );
}

#[test]
fn traditional_state_restore_refreshes_calculated_character_count() {
    let artifact =
        compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let charanum = artifact
        .globals
        .iter()
        .find(|global| global.name == "CHARANUM")
        .expect("CHARANUM")
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
            .any(|event| matches!(event, VmEvent::FiberCompleted { value: None, .. }))
    );

    let save = vm.export_era_state();
    let saved_character_count = i64::try_from(save.characters.len()).unwrap();
    vm.reset_with_era_state(&save).unwrap();
    assert_eq!(
        vm.read_variable(charanum, &[], None),
        Ok(VmValue::Integer(saved_character_count))
    );
}

#[test]
fn ordinary_save_excludes_and_restore_preserves_global_save_variables() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary");
    let global_key = SymbolKey::derive("test.variable", b"global");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_VALUE".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(ordinary, &[0], None, VmValue::Integer(11))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(21))
        .unwrap();
    let save = vm.export_era_state();
    assert!(save.variables.contains_key(&ordinary));
    assert!(!save.variables.contains_key(&global_key));

    vm.write_variable(ordinary, &[0], None, VmValue::Integer(12))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(22))
        .unwrap();
    vm.reset_with_era_state(&save).unwrap();
    assert_eq!(
        vm.read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(11))
    );
    assert_eq!(
        vm.read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(22))
    );
}

#[test]
fn global_overlay_transaction_changes_only_global_save_storage() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary-overlay");
    let global_key = SymbolKey::derive("test.variable", b"global-overlay");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY_OVERLAY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_OVERLAY".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    vm.vm_mut()
        .write_variable(ordinary, &[0], None, VmValue::Integer(10))
        .unwrap();
    let mut state = vm.vm().export_era_state_for(EraSaveScope::Global);
    state.variables.get_mut(&global_key).unwrap().values[0] = VmValue::Integer(20);
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::OverlayGlobal(Box::new(state)))
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();
    assert_eq!(
        vm.vm().read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(10))
    );
    assert_eq!(
        vm.vm().read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(20))
    );
}

#[test]
fn isolated_fork_copies_memory_without_copying_live_execution() {
    let key = SymbolKey::derive("test.variable", b"candidate");
    let artifact = artifact(Vec::new(), vec![global(key, vec![1])]);
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(7))
        .unwrap();

    let mut candidate = live.fork_isolated().unwrap();
    assert_eq!(
        candidate.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    candidate
        .vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(9))
        .unwrap();

    assert_eq!(
        live.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    assert!(!candidate.has_runnable_fibers());
}

#[test]
fn compiled_arithmetic_executes_and_updates_project_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = (2 + 3) * 4\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(20));
}

#[test]
fn compiled_assignment_matches_reference_smoke_input() {
    // The macOS/Windows reference smoke suite executes the exact `RESULT = 9`
    // statement and observes RESULT=9 through the C# VM watch projection.
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = 9\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(9));
}

#[test]
fn dynamic_try_resolves_before_arguments_and_form_call_invokes_target() {
    let artifact = compile_source(
        "@ORACLE_COMPAT\nRESULT = 0\nTRYCALLFORM ORACLE_MISSING(1 / LOCAL)\nCALLFORM ORACLE_DYNAMIC_{1}(4)\nRETURN RESULT\n@ORACLE_DYNAMIC_1(ARG)\nFLAG:0 = ARG\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_COMPAT")
        .expect("entry")
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
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(4)));
}

#[test]
fn formatted_try_call_resolves_a_unicode_function_before_catch() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\n\
         #DIM REQUEST_ID\n\
         REQUEST_ID = 2005\n\
         RESULT = 0\n\
         RESULTS:0 = IRAI_一般{REQUEST_ID % 1000}\n\
         TRYCCALLFORM IRAI_一般{REQUEST_ID % 1000}(2, REQUEST_ID, \"依頼実行時\")\n\
         CATCH\n\
         FLAG:0 = -1\n\
         ENDCATCH\n\
         RETURN RESULT\n\
         @IRAI_一般5(CHARA, IRAI_ID, SCENE)\n\
         #DIM CHARA\n\
         #DIM IRAI_ID\n\
         #DIMS SCENE\n\
         FLAG:0 = CHARA + IRAI_ID + (SCENE == \"依頼実行時\")\n\
         RETURN RESULT\n",
        &AnalyzerOptions::default(),
    );
    assert!(
        artifact
            .functions
            .iter()
            .any(|function| function.name == "IRAI_一般5"),
        "dynamic target was omitted from the compiled artifact"
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
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
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("IRAI_一般5".into()))
    );
    assert_eq!(
        vm.read_variable(flag, &[0], None),
        Ok(VmValue::Integer(2_008))
    );
}
