use super::*;
use erabasic_validator::{ValidationContext, validate_bytecode};

struct NoHost;
impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("literal SELECT fixture must not call Host");
    }
}

fn fixture(source: &str, optimized: bool) -> (Vm, NativeServiceRegistry, SymbolKey) {
    let artifact = literal_groupmatch_tests::compile_cursor_fixture(source.into());
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validation.value.unwrap(), crate::VmConfig::default());
    if !optimized {
        Arc::make_mut(vm.generations.get_mut(&vm.current_generation).unwrap())
            .disable_literal_select_for_test();
    }
    vm.spawn_entry(entry, Vec::new()).unwrap();
    (vm, natives, entry)
}

fn compare(source: &str, budget: u64, maximum_stack: usize) -> Vec<VmEvent> {
    let (mut fast, mut fast_natives, _) = fixture(source, true);
    let (mut scalar, mut scalar_natives, _) = fixture(source, false);
    fast.config.maximum_operand_stack = maximum_stack;
    scalar.config.maximum_operand_stack = maximum_stack;
    for _ in 0..1000 {
        let run = RunBudget {
            maximum_instructions: budget,
            ..RunBudget::default()
        };
        let actual = fast.run_slice(&mut NoHost, &mut fast_natives, run);
        let expected = scalar.run_slice(&mut NoHost, &mut scalar_natives, run);
        assert_eq!(actual, expected, "{source}");
        assert_eq!(
            fast.encode_unrestricted_snapshot(&fast_natives).unwrap(),
            scalar
                .encode_unrestricted_snapshot(&scalar_natives)
                .unwrap()
        );
        if actual.stop != VmRunStop::BudgetExhausted {
            return actual.events;
        }
    }
    panic!("fixture did not terminate");
}

#[test]
fn literal_select_type_errors_preserve_scalar_fault_and_popped_operands() {
    for (selector, first, indexed) in [("\"wrong\"", "1", true), ("9", "\"wrong\"", false)] {
        let source = format!(
            "@SYSTEM_TITLE\nSELECTCASE {selector}\nCASE {first}\nRETURN 1\nCASE 2\nRETURN 2\nCASE 3\nRETURN 3\nCASE 4\nRETURN 4\nENDSELECT\nRETURN\n"
        );
        let (vm, _, _) = fixture(&source, true);
        let program = &vm.generations[&vm.current_generation];
        let pc = program.artifact.functions[0]
            .code
            .iter()
            .position(|instruction| instruction.opcode == Opcode::SelectStart as u16)
            .unwrap();
        assert_eq!(program.literal_select_plan(0, pc).is_some(), indexed);
        let events = compare(&source, 1000, 1000);
        assert!(events.iter().any(|event| matches!(event, VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::TypeMismatch)), "{events:?}");
    }
}

fn integer_source(selector: i64, extra: &str) -> String {
    format!(
        "@SYSTEM_TITLE\nSELECTCASE {selector}\nCASE 1, 3\nRESULT = 11\nCASE 3\nRESULT = 22\nCASE 5\nRESULT = 33\nCASE 7\nRESULT = 44\n{extra}ENDSELECT\nRETURN RESULT\n"
    )
}

#[test]
fn literal_select_preserves_first_duplicate_misses_and_slice_snapshots() {
    for selector in [1, 3, 5, 7, 9] {
        for extra in ["", "CASEELSE\nRESULT = 55\n"] {
            for budget in [1, 4, 7, 10, 13, 1000] {
                compare(&integer_source(selector, extra), budget, 1000);
            }
        }
    }
}

#[test]
fn literal_select_preserves_exact_string_equality() {
    for selector in ["映姫", "映姬", "ABC", "abc", "missing"] {
        let source = format!(
            "@SYSTEM_TITLE\nSELECTCASE \"{selector}\"\nCASE \"映姫\", \"映姬\"\nRESULT = 1\nCASE \"ABC\"\nRESULT = 2\nCASE \"abc\"\nRESULT = 3\nCASE \"映姫\"\nRESULT = 4\nENDSELECT\nRETURN RESULT\n"
        );
        compare(&source, 1000, 1000);
    }
}

#[test]
fn literal_select_preserves_nested_scopes_and_unstructured_entry() {
    for source in [
        "@SYSTEM_TITLE\nFOR LOCAL, 0, 2\nSELECTCASE 2\nCASE 1\nRESULT = 1\nCASE 2\nSELECTCASE 4\nCASE 1\nRESULT = 1\nCASE 2\nRESULT = 2\nCASE 3\nRESULT = 3\nCASE 4\nRESULT += 4\nENDSELECT\nCASE 3\nRESULT = 3\nCASE 4\nRESULT = 4\nENDSELECT\nNEXT\nRETURN RESULT\n",
        "@SYSTEM_TITLE\nGOTO INSIDE\nSELECTCASE 9\nCASE 1\n$INSIDE\nRESULT = 1\nCASE 2\nRESULT = 2\nCASE 3\nRESULT = 3\nCASE 4\nRESULT = 4\nENDSELECT\nRETURN RESULT\n",
    ] {
        for budget in [1, 7, 1000] {
            compare(source, budget, 1000);
        }
    }
}

#[test]
fn literal_select_stack_faults_keep_scalar_pc_and_mutations() {
    for maximum in [0, 1, 2] {
        for selector in [1, 5, 9] {
            compare(
                &integer_source(selector, "CASEELSE\nRESULT = 99\n"),
                1000,
                maximum,
            );
        }
    }
}

#[test]
fn literal_select_real_dispatch_and_readonly_barriers() {
    let (mut vm, mut natives, entry) = fixture(&integer_source(3, ""), true);
    let program = vm.generations[&vm.current_generation].clone();
    let pc = program.artifact.functions[0]
        .code
        .iter()
        .position(|encoded| encoded.opcode == Opcode::SelectStart as u16)
        .unwrap();
    let target = program
        .literal_select_plan(0, pc)
        .expect("compiled CASE plan")
        .target(&VmValue::Integer(3))
        .unwrap();
    assert_eq!(
        target.additional_instructions, 6,
        "all labels in the first OR group execute"
    );
    let id = *vm.fibers.keys().next().unwrap();
    let mut fiber = vm.fibers.remove(&id).unwrap();
    fiber.frames.last_mut().unwrap().instruction = pc;
    fiber.frames.last_mut().unwrap().stack = vec![VmValue::Integer(3)];
    let mut cursor = None;
    let position = vm
        .instruction_position_at(vm.current_generation, entry, pc, &mut cursor)
        .unwrap();
    let policy = ExecutionPolicy {
        allow_function_memo: true,
        allow_immediate_host: false,
        remaining_quantum: 7,
        remaining_instructions: 7,
    };
    for blocked in [
        ExecutionPolicy {
            allow_function_memo: false,
            ..policy
        },
        ExecutionPolicy {
            remaining_quantum: 6,
            ..policy
        },
        ExecutionPolicy {
            remaining_instructions: 6,
            ..policy
        },
    ] {
        assert_eq!(vm.try_literal_select(&mut fiber, &position, blocked), None);
        assert_eq!(fiber.frames.last().unwrap().instruction, pc);
        assert_eq!(fiber.frames.last().unwrap().stack, [VmValue::Integer(3)]);
    }
    fiber
        .frames
        .last_mut()
        .unwrap()
        .select_values
        .push(VmValue::Integer(0));
    assert_eq!(vm.try_literal_select(&mut fiber, &position, policy), None);
    fiber.frames.last_mut().unwrap().select_values.clear();
    fiber.frames.last_mut().unwrap().stack = vec![VmValue::String("3".into())];
    assert_eq!(vm.try_literal_select(&mut fiber, &position, policy), None);
    fiber.frames.last_mut().unwrap().stack = vec![VmValue::Integer(3)];
    vm.config.maximum_operand_stack = 1;
    assert_eq!(vm.try_literal_select(&mut fiber, &position, policy), None);
    vm.config.maximum_operand_stack = 2;
    let outcome = vm
        .execute_instruction(
            &mut fiber,
            &position,
            &mut NoHost,
            &mut natives,
            &mut 0,
            policy,
        )
        .unwrap();
    assert!(matches!(outcome, StepOutcome::BulkProgress(6)));
    let frame = fiber.frames.last().unwrap();
    assert_eq!(frame.instruction, target.instruction);
    assert!(frame.stack.is_empty());
    assert_eq!(frame.select_values, [VmValue::Integer(3)]);
}

#[test]
fn literal_select_dynamic_and_range_chains_are_not_indexed() {
    for label in ["LOCAL", "1 TO 3", "IS >= 2"] {
        let source = format!(
            "@SYSTEM_TITLE\nSELECTCASE 5\nCASE {label}\nRESULT = 1\nCASE 5\nRESULT = 2\nCASE 6\nRESULT = 3\nCASE 7\nRESULT = 4\nENDSELECT\nRETURN RESULT\n"
        );
        let (vm, _, _) = fixture(&source, true);
        let program = &vm.generations[&vm.current_generation];
        for pc in 0..program.artifact.functions[0].code.len() {
            assert!(program.literal_select_plan(0, pc).is_none());
        }
        compare(&source, 1000, 1000);
    }
}

#[test]
fn literal_select_function_memo_and_rebuilt_generations_match_scalar() {
    let source = "@SYSTEM_TITLE\nRESULT = CHOOSE(4) + CHOOSE(4) + CHOOSE(9)\nRETURN RESULT\n@CHOOSE(ARG)\n#FUNCTION\nSELECTCASE ARG\nCASE 1\nRETURNF 11\nCASE 2\nRETURNF 22\nCASE 3\nRETURNF 33\nCASE 4\nRETURNF 44\nCASEELSE\nRETURNF 55\nENDSELECT\nRETURNF 0\n";
    for budget in [1, 19, 1000] {
        compare(source, budget, 1000);
    }
    let (vm, _, _) = fixture(source, true);
    let old = &vm.generations[&vm.current_generation];
    let mut artifact = old.artifact.as_ref().clone();
    let index = artifact
        .functions
        .iter()
        .position(|function| function.name == "CHOOSE")
        .unwrap();
    let function = &mut artifact.functions[index];
    let pc = function
        .code
        .iter()
        .position(|instruction| instruction.opcode == Opcode::SelectStart as u16)
        .unwrap();
    let label = &mut function.code[pc + 1];
    assert_eq!(label.opcode, Opcode::PushInteger as u16);
    label.payload = 9_i64.to_le_bytes().to_vec().into();
    artifact.refresh_ids().unwrap();
    let fresh = ProgramGeneration::new(Arc::new(artifact));
    assert_ne!(
        old.literal_select_plan(index, pc)
            .unwrap()
            .target(&VmValue::Integer(9))
            .unwrap()
            .instruction,
        fresh
            .literal_select_plan(index, pc)
            .unwrap()
            .target(&VmValue::Integer(9))
            .unwrap()
            .instruction
    );
}
