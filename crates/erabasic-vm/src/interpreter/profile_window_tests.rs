use super::*;

struct NoHost;
impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("no Host expected");
    }
}

fn fixture(source: &str) -> (Vm, NativeServiceRegistry, FiberId) {
    let artifact = literal_groupmatch_tests::compile_cursor_fixture(source.into());
    let entry = artifact.functions[0].key;
    let validated = erabasic_validator::validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_validator::ValidationContext::for_artifact(&artifact),
    )
    .value
    .unwrap();
    let mut vm = Vm::new(validated, crate::VmConfig::default());
    let natives = NativeServiceRegistry::for_artifact(&artifact);
    let id = vm.spawn_entry(entry, Vec::new()).unwrap();
    (vm, natives, id)
}

fn prime(vm: &mut Vm, id: FiberId) {
    let frame = vm.fibers[&id].frames.last().unwrap();
    let (generation, function, frame_id) = (frame.generation, frame.function, frame.id);
    // Leave the global periodic phase immediately before a sample. This setup is
    // outside the action window; subsequent observations use the actual scheduler.
    for _ in 0..1023 {
        vm.instruction_profile.observe_dispatch(
            generation,
            function,
            0,
            Opcode::Nop as u16,
            Some(crate::instruction_profile::DispatchContext {
                fiber: id,
                frame: frame_id,
            }),
        );
    }
    vm.instruction_profile_boundary(true);
}

fn window(vm: &Vm) -> serde_json::Value {
    serde_json::to_value(vm.instruction_profile_snapshot()).unwrap()["dispatchWindows"].clone()
}

#[test]
fn instruction_profile_windows_classify_actual_step_outcome_carriers() {
    let error = || StepError::new(VmFaultCode::TypeMismatch, "test failure");
    for (outcome, drained, expected) in [
        (Err(error()), false, "fault"),
        (
            Ok(StepOutcome::BulkFailure {
                additional_instructions: 3,
                error: error(),
            }),
            false,
            "fault",
        ),
        (
            Ok(StepOutcome::Diagnostic {
                code: "test",
                message: "test",
                notification: crate::VmDiagnosticNotification::LogOnly,
            }),
            false,
            "diagnostic",
        ),
        (Ok(StepOutcome::Continue), true, "diagnostic"),
        (Ok(StepOutcome::BulkProgress(3)), false, "control_boundary"),
    ] {
        let (mut vm, _, id) = fixture("@SYSTEM_TITLE\nRETURN\n");
        prime(&mut vm, id);
        let frame = vm.fibers[&id].frames.last().unwrap();
        vm.instruction_profile.observe_dispatch(
            frame.generation,
            frame.function,
            0,
            Opcode::Nop as u16,
            Some(crate::instruction_profile::DispatchContext {
                fiber: id,
                frame: frame.id,
            }),
        );
        vm.finish_profile_dispatch(Opcode::Nop as u16, &outcome, drained);
        assert_eq!(window(&vm)["counts"][0]["termination"], expected);
        assert!(window(&vm)["pending"].is_null());
    }
}

#[test]
fn instruction_profile_windows_untaken_branch_still_ends_the_sequence() {
    let (mut vm, mut natives, id) = fixture("@SYSTEM_TITLE\nIF LOCAL\nRESULT = 1\nENDIF\nRETURN\n");
    let code = &vm.generations[&vm.current_generation].artifact.functions[0].code;
    let pc = code
        .iter()
        .position(|instruction| instruction.opcode == Opcode::JumpIfFalse as u16)
        .unwrap();
    let frame = vm.fibers.get_mut(&id).unwrap().frames.last_mut().unwrap();
    frame.instruction = pc;
    frame.stack = vec![VmValue::Integer(1)];
    prime(&mut vm, id);
    let report = vm.run_slice(
        &mut NoHost,
        &mut natives,
        RunBudget {
            maximum_instructions: 1,
            ..RunBudget::default()
        },
    );
    assert!(report.events.is_empty());
    assert_eq!(vm.fibers[&id].frames.last().unwrap().instruction, pc + 1);
    let snapshot = window(&vm);
    assert_eq!(
        snapshot["counts"][0]["opcodes"],
        serde_json::json!([Opcode::JumpIfFalse as u16])
    );
    assert_eq!(snapshot["counts"][0]["termination"], "control_boundary");
}

#[test]
fn instruction_profile_windows_scheduler_fault_tail_is_not_a_successful_run() {
    let (mut vm, mut natives, id) = fixture("@SYSTEM_TITLE\nRESULT = LOCAL + 2\nRETURN\n");
    let code = &vm.generations[&vm.current_generation].artifact.functions[0].code;
    let pc = code
        .iter()
        .position(|instruction| instruction.opcode == Opcode::LoadVariable as u16)
        .unwrap();
    assert_eq!(code[pc + 1].opcode, Opcode::PushInteger as u16);
    assert_eq!(
        u16::from_le_bytes(code[pc].payload[16..18].try_into().unwrap()),
        0
    );
    vm.fibers
        .get_mut(&id)
        .unwrap()
        .frames
        .last_mut()
        .unwrap()
        .instruction = pc;
    vm.config.maximum_operand_stack = 1;
    prime(&mut vm, id);
    let report = vm.run_slice(&mut NoHost, &mut natives, RunBudget::default());
    assert_eq!(report.instructions, 2);
    assert!(report.events.iter().any(|event| matches!(event, VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::ResourceLimit)));
    let snapshot = window(&vm);
    assert_eq!(
        snapshot["counts"][0]["opcodes"],
        serde_json::json!([Opcode::LoadVariable as u16, Opcode::PushInteger as u16])
    );
    assert_eq!(snapshot["counts"][0]["termination"], "fault");
}
