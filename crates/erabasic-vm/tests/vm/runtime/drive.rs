use super::*;
#[test]
fn runtime_port_preserves_cooperative_host_calls_across_reverse_completions() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fibers = [
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
    ];
    let report = runtime.drive(
        RunBudget {
            maximum_instructions: 16,
            maximum_host_calls: 2,
            fiber_quantum: 1,
        },
        VmDriveMode::Normal,
    );
    let requests = report
        .events
        .into_iter()
        .filter_map(|event| match event {
            VmPortEvent::HostCall(request) => Some(request),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests
            .iter()
            .map(|request| request.fiber)
            .collect::<Vec<_>>(),
        fibers
    );
    assert!(requests.iter().all(|request| {
        request.arguments == [VmValue::Integer(7)] && request.import.import.name == "operation"
    }));

    for request in requests.iter().rev() {
        let prepared = runtime
            .validate_host_completion(request.id, VmHostCompletion::Ready(HostReady::empty()))
            .unwrap();
        assert_eq!(
            runtime.commit_host_completion(prepared).unwrap(),
            request.fiber
        );
    }

    let report = runtime.drive(RunBudget::default(), VmDriveMode::Normal);
    let mut completed = report
        .events
        .into_iter()
        .filter_map(|event| match event {
            VmPortEvent::FiberCompleted(fiber, None) => Some(fiber),
            _ => None,
        })
        .collect::<Vec<_>>();
    completed.sort();
    let mut expected = fibers.to_vec();
    expected.sort();
    assert_eq!(completed, expected);
}

#[derive(Default)]
struct RecordingImmediateHost {
    calls: Vec<(FiberId, i64)>,
}

impl VmHost for RecordingImmediateHost {
    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        if request.normalized_name != "OPERATION" {
            return ImmediateHostCallResult::Unsupported;
        }
        let Some(VmValue::Integer(value)) = request.arguments.first() else {
            return ImmediateHostCallResult::Unsupported;
        };
        self.calls.push((request.fiber, *value));
        ImmediateHostCallResult::Ready(HostReady::empty())
    }

    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        panic!("the test operation must stay on the immediate lane");
    }
}

fn repeated_host_artifact() -> (BytecodeArtifact, SymbolKey) {
    let (mut artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    artifact.functions[0].code = vec![
        opcode::push_integer(1),
        opcode::call(Opcode::CallHost, 0, 1, None),
        opcode::push_integer(2),
        opcode::call(Opcode::CallHost, 0, 1, None),
        opcode::return_value(false),
    ];
    artifact.refresh_ids().unwrap();
    (artifact, entry)
}

#[test]
fn immediate_host_calls_continue_within_a_fiber_quantum() {
    let (artifact, entry) = repeated_host_artifact();
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fibers = [
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
    ];
    let mut host = RecordingImmediateHost::default();
    let report = runtime.drive_with_immediate_host(
        RunBudget {
            maximum_instructions: 4,
            maximum_host_calls: 4,
            fiber_quantum: 32,
        },
        VmDriveMode::Normal,
        &mut host,
    );

    assert_eq!(host.calls, vec![(fibers[0], 1), (fibers[0], 2)]);
    assert!(
        report
            .events
            .iter()
            .all(|event| !matches!(event, VmPortEvent::HostCall(_)))
    );
    assert_eq!(report.stop, VmPortStop::BudgetExhausted);
}

#[test]
fn immediate_host_calls_switch_fibers_at_the_quantum_boundary() {
    let (artifact, entry) = repeated_host_artifact();
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fibers = [
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
        runtime.spawn_entry(entry, Vec::new()).unwrap(),
    ];
    let mut host = RecordingImmediateHost::default();
    runtime.drive_with_immediate_host(
        RunBudget {
            maximum_instructions: 32,
            maximum_host_calls: 4,
            fiber_quantum: 2,
        },
        VmDriveMode::Normal,
        &mut host,
    );

    assert_eq!(
        host.calls,
        vec![
            (fibers[0], 1),
            (fibers[1], 1),
            (fibers[0], 2),
            (fibers[1], 2)
        ]
    );
}

#[test]
fn immediate_host_calls_obey_the_host_call_budget() {
    let (artifact, entry) = repeated_host_artifact();
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    runtime.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = RecordingImmediateHost::default();
    let first = runtime.drive_with_immediate_host(
        RunBudget {
            maximum_instructions: 32,
            maximum_host_calls: 1,
            fiber_quantum: 32,
        },
        VmDriveMode::Normal,
        &mut host,
    );
    assert_eq!(first.stop, VmPortStop::BudgetExhausted);
    assert_eq!(host.calls.len(), 1);
    assert!(first.events.is_empty());

    let second =
        runtime.drive_with_immediate_host(RunBudget::default(), VmDriveMode::Normal, &mut host);
    assert_eq!(host.calls.len(), 2);
    assert!(matches!(
        second.events.as_slice(),
        [VmPortEvent::FiberCompleted(_, None)]
    ));
}

#[derive(Default)]
struct AcceptingImmediateHost {
    calls: Vec<String>,
}

impl VmHost for AcceptingImmediateHost {
    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        self.calls.push(request.normalized_name.to_owned());
        ImmediateHostCallResult::Ready(HostReady::empty())
    }

    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        panic!("RuntimeVm captures ordinary Host calls before the caller dispatches them");
    }
}

#[test]
fn queued_diagnostic_is_an_ordering_barrier_for_immediate_host_calls() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nGOTO CHOICE\nSELECTCASE 0\nCASE 0\n$CHOICE\nRESULT = 1\nENDSELECT\nPRINT 7\nRETURN\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    runtime.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = AcceptingImmediateHost::default();

    let report =
        runtime.drive_with_immediate_host(RunBudget::default(), VmDriveMode::Normal, &mut host);

    assert!(host.calls.is_empty());
    assert!(matches!(
        report.events.as_slice(),
        [VmPortEvent::Diagnostic { .. }, VmPortEvent::HostCall(_)]
    ));
}

#[test]
fn debug_step_is_an_ordering_barrier_for_immediate_host_calls() {
    let (artifact, entry) = host_artifact(HostSnapshotCapability::StableWait);
    let mut runtime = RuntimeVm::new(validated(&artifact), VmConfig::default());
    let fiber = runtime.spawn_entry(entry, Vec::new()).unwrap();
    let pause = runtime.request_pause().unwrap();
    runtime
        .step(pause.token, fiber, VmStepKind::Instruction)
        .unwrap();
    let mut host = AcceptingImmediateHost::default();
    let first =
        runtime.drive_with_immediate_host(RunBudget::default(), VmDriveMode::Normal, &mut host);
    let stop = first
        .events
        .iter()
        .find_map(|event| match event {
            VmPortEvent::DebugStopped(stop) => Some(stop.clone()),
            _ => None,
        })
        .expect("the first stepped instruction must stop before CallHost");

    runtime
        .step(stop.token, fiber, VmStepKind::Instruction)
        .unwrap();
    let second =
        runtime.drive_with_immediate_host(RunBudget::default(), VmDriveMode::Normal, &mut host);

    assert!(host.calls.is_empty());
    assert!(matches!(
        second.events.as_slice(),
        [VmPortEvent::HostCall(_), VmPortEvent::DebugStopped(_)]
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

pub(super) fn call_artifact(
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
