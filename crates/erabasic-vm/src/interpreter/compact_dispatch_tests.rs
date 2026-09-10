use super::*;
use erabasic_validator::{ValidationContext, validate_bytecode};

struct NoHost;

impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("basic dispatch must not call Host");
    }
}

fn fixture(source: &str) -> (Vm, Fiber, NativeServiceRegistry) {
    let artifact = literal_groupmatch_tests::compile_cursor_fixture(source.into());
    let entry = artifact.functions[0].key;
    let validated = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(
        validated.diagnostics.is_empty(),
        "{:?}",
        validated.diagnostics
    );
    let mut vm = Vm::new(validated.value.unwrap(), crate::VmConfig::default());
    let natives = NativeServiceRegistry::for_artifact(&artifact);
    let id = vm.spawn_entry(entry, Vec::new()).unwrap();
    let fiber = vm.fibers.remove(&id).unwrap();
    (vm, fiber, natives)
}

fn execute(
    vm: &mut Vm,
    fiber: &mut Fiber,
    natives: &mut NativeServiceRegistry,
    opcode: Opcode,
    payload: &[u8],
) -> Result<StepOutcome, StepError> {
    let frame = fiber.frames.last().unwrap();
    let position = InstructionPosition {
        resolved_program: None,
        generation: frame.generation,
        function: frame.function,
        instruction: frame.instruction,
        variable: None,
        literal_group_match: None,
        encoded: DispatchInstruction {
            opcode: opcode as u16,
            payload,
        },
    };
    vm.execute_instruction(
        fiber,
        &position,
        &mut NoHost,
        natives,
        &mut 0,
        ExecutionPolicy {
            allow_function_memo: false,
            allow_immediate_host: false,
            remaining_quantum: 1,
            remaining_instructions: 1,
        },
    )
}

#[test]
fn compact_dispatch_preserves_stack_limit_and_failed_instruction_state() {
    let (mut vm, mut fiber, mut natives) = fixture("@SYSTEM_TITLE\nRETURN\n");
    vm.config.maximum_operand_stack = 1;
    let value = 7_i64.to_le_bytes();
    assert!(matches!(
        execute(
            &mut vm,
            &mut fiber,
            &mut natives,
            Opcode::PushInteger,
            &value
        ),
        Ok(StepOutcome::Continue)
    ));
    let error = execute(
        &mut vm,
        &mut fiber,
        &mut natives,
        Opcode::PushInteger,
        &value,
    )
    .err()
    .unwrap();
    assert_eq!(error.category, crate::FaultCategory::ResourceLimit);
    assert_eq!(error.code, VmFaultCode::ResourceLimit);
    assert_eq!(error.message, "maximum operand stack exceeded");
    let frame = fiber.frames.last().unwrap();
    assert_eq!(frame.instruction, 2);
    assert_eq!(frame.stack, [VmValue::Integer(7), VmValue::Integer(7)]);
}

#[test]
fn compact_dispatch_counts_implicit_context_slots() {
    let (mut vm, mut fiber, mut natives) = fixture("@SYSTEM_TITLE\nRETURN\n");
    vm.config.maximum_operand_stack = 1;
    fiber
        .frames
        .last_mut()
        .unwrap()
        .existvar_checks
        .push(existvar::ExistVarCheckpoint {
            begin: 0,
            failure: 0,
            stack_index: 0,
            user_calls: 0,
            caught: false,
        });
    assert!(matches!(
        execute(&mut vm, &mut fiber, &mut natives, Opcode::Nop, &[]),
        Ok(StepOutcome::Continue)
    ));
    let error = execute(
        &mut vm,
        &mut fiber,
        &mut natives,
        Opcode::PushInteger,
        &7_i64.to_le_bytes(),
    )
    .err()
    .unwrap();
    assert_eq!(error.category, crate::FaultCategory::ResourceLimit);
    assert_eq!(fiber.frames.last().unwrap().stack.len(), 1);
    assert_eq!(fiber.frames.last().unwrap().existvar_checks.len(), 1);
}

#[test]
fn compact_dispatch_unboxes_payload_and_operand_failures_without_reclassification() {
    let (mut vm, mut fiber, mut natives) = fixture("@SYSTEM_TITLE\nRETURN\n");
    for (opcode, code) in [
        (Opcode::PushInteger, VmFaultCode::InvalidInstruction),
        (Opcode::Pop, VmFaultCode::StackUnderflow),
    ] {
        let error = execute(&mut vm, &mut fiber, &mut natives, opcode, &[])
            .err()
            .unwrap();
        assert_eq!(error.category, crate::FaultCategory::InternalInvariant);
        assert_eq!(error.code, code);
        assert!(!error.message.is_empty());
        assert!(fiber.frames.last().unwrap().stack.is_empty());
    }
    assert_eq!(fiber.frames.last().unwrap().instruction, 2);
}

#[test]
fn compact_dispatch_structured_jump_checks_stack_before_publishing_diagnostic() {
    let source =
        "@SYSTEM_TITLE\nGOTO INNER\nSELECTCASE 1\nCASE 1\n$INNER\nRESULT = 5\nENDSELECT\nRETURN\n";
    for over_limit in [false, true] {
        let (mut vm, mut fiber, mut natives) = fixture(source);
        let frame = fiber.frames.last().unwrap();
        let function = frame.function;
        let generation = frame.generation;
        let code = &vm.generations[&generation].function(function).unwrap().code;
        let index = code
            .iter()
            .position(|i| i.opcode == Opcode::Jump as u16)
            .unwrap();
        let payload = code[index].payload.to_vec();
        fiber.frames.last_mut().unwrap().instruction = index;
        if over_limit {
            vm.config.maximum_operand_stack = 0;
            fiber
                .frames
                .last_mut()
                .unwrap()
                .stack
                .push(VmValue::Integer(1));
        }
        let result = execute(&mut vm, &mut fiber, &mut natives, Opcode::Jump, &payload);
        if over_limit {
            assert_eq!(
                result.err().unwrap().category,
                crate::FaultCategory::ResourceLimit
            );
        } else {
            assert!(matches!(
                result,
                Ok(StepOutcome::Diagnostic {
                    code: STRUCTURED_GOTO_DIAGNOSTIC_CODE,
                    message: STRUCTURED_GOTO_DIAGNOSTIC_MESSAGE,
                    notification: crate::VmDiagnosticNotification::LogOnly,
                })
            ));
        }
        assert_eq!(fiber.frames.last().unwrap().select_values.len(), 1);
    }
}
