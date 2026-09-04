use super::*;
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
