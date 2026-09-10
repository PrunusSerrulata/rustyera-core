use super::*;
use crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture;

struct NoHost;
impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("dynamic-call cache fixture must not call Host");
    }
}

fn fixture() -> (Vm, NativeServiceRegistry, SymbolKey) {
    fixture_source(
        "@SYSTEM_TITLE\nTRYCALLFORM TARGET(1, 2)\nTRYCALLFORM ABSENT(3)\nRETURN RESULT\n@TARGET(ARG, ARG:1)\nRESULT = ARG + ARG:1\nRETURN RESULT\n",
    )
}

fn fixture_source(source: &str) -> (Vm, NativeServiceRegistry, SymbolKey) {
    let artifact = compile_cursor_fixture(source.into());
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let validation = erabasic_validator::validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_validator::ValidationContext::for_artifact(&artifact),
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let natives = NativeServiceRegistry::for_artifact(&artifact);
    (
        Vm::new(validation.value.unwrap(), crate::VmConfig::default()),
        natives,
        entry,
    )
}

fn guard_index(program: &ProgramGeneration, entry: SymbolKey) -> usize {
    program
        .function(entry)
        .unwrap()
        .code
        .iter()
        .position(|instruction| instruction.opcode == Opcode::GuardUserArgument as u16)
        .unwrap()
}

#[test]
fn decoded_user_call_bad_origins_keep_errors_with_and_without_cache() {
    let (vm, _, entry) = fixture();
    let original = &vm.generations[&vm.current_generation];
    let instruction = guard_index(original, entry);
    for cached in [true, false] {
        for kind in [
            "out_of_bounds",
            "non_resolve",
            "not_earlier",
            "invalid_payload",
        ] {
            let mut program = (**original).clone();
            let mut cursor = None;
            let mut position = vm
                .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
                .unwrap();
            let resolve = read_u32(position.encoded.payload, 0).unwrap() as usize;
            let mut payload = position.encoded.payload.to_vec();
            let mut expected = invalid("user consumer has no earlier resolve origin");
            match kind {
                "out_of_bounds" => payload[..4].copy_from_slice(&u32::MAX.to_le_bytes()),
                "non_resolve" => {
                    let earlier = (0..resolve)
                        .find(|index| {
                            program.function(entry).unwrap().code[*index].opcode
                                != Opcode::ResolveUserCall as u16
                        })
                        .unwrap();
                    payload[..4].copy_from_slice(&u32::try_from(earlier).unwrap().to_le_bytes());
                }
                "not_earlier" => position.instruction = resolve,
                "invalid_payload" => {
                    let mut artifact = (*program.artifact).clone();
                    let function = artifact
                        .functions
                        .iter_mut()
                        .find(|function| function.key == entry)
                        .unwrap();
                    let mut malformed = function.code[resolve].payload.to_vec();
                    malformed[0] = 0xff;
                    expected = UserCallSpec::decode(&malformed)
                        .map_err(invalid)
                        .unwrap_err();
                    function.code[resolve].payload = malformed.into();
                    program = ProgramGeneration::new(Arc::new(artifact));
                }
                _ => unreachable!(),
            }
            if !cached {
                program.disable_user_call_specs_for_test();
            }
            position.encoded.payload = &payload;
            assert_eq!(
                user_origin(&position, &program).unwrap_err(),
                expected,
                "{cached} {kind}"
            );
        }
    }
}

fn at_guard() -> (Vm, crate::FiberId, SymbolKey) {
    let (mut vm, mut natives, entry) = fixture();
    let id = vm.spawn_entry(entry, Vec::new()).unwrap();
    let instruction = guard_index(&vm.generations[&vm.current_generation], entry);
    for _ in 0..100 {
        if vm.fibers[&id].frames.last().unwrap().instruction == instruction {
            return (vm, id, entry);
        }
        let report = vm.run_slice(
            &mut NoHost,
            &mut natives,
            RunBudget {
                maximum_instructions: 1,
                ..RunBudget::default()
            },
        );
        assert_eq!(report.instructions, 1);
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
        );
    }
    panic!("fixture did not reach a real argument consumer");
}

#[test]
fn decoded_user_call_consumer_rejects_tokens_after_prior_operand_checks() {
    let (vm, id, entry) = at_guard();
    let caller = vm.fibers[&id].frames.last().unwrap();
    for cached in [true, false] {
        let mut program = (*vm.generations[&vm.current_generation]).clone();
        if !cached {
            program.disable_user_call_specs_for_test();
        }
        let mut cursor = None;
        let position = vm
            .instruction_position_at(
                vm.current_generation,
                entry,
                caller.instruction,
                &mut cursor,
            )
            .unwrap();
        assert!(
            decode_user_consumer(caller, &position, Opcode::GuardUserArgument, &program).is_ok()
        );
        let original_payload = position.encoded.payload.to_vec();
        let mut wrong = caller.clone();
        for token in [VmValue::String("wrong target".into()), VmValue::Integer(0)] {
            *wrong.stack.last_mut().unwrap() = token;
            assert_eq!(
                decode_user_consumer(&wrong, &position, Opcode::GuardUserArgument, &program)
                    .err()
                    .unwrap(),
                invalid("user token, generation, origin or slot progress differs")
            );
        }
        for (kind, expected) in [
            ("length", "invalid user consumer operands"),
            ("origin", "user consumer has no earlier resolve origin"),
            ("slot", "user argument slot is out of bounds"),
        ] {
            let mut payload = original_payload.clone();
            match kind {
                "length" => {
                    payload.pop();
                }
                "origin" => payload[..4].copy_from_slice(&u32::MAX.to_le_bytes()),
                "slot" => payload[4..6].copy_from_slice(&u16::MAX.to_le_bytes()),
                _ => unreachable!(),
            }
            let mut invalid_cursor = None;
            let mut invalid_position = vm
                .instruction_position_at(
                    vm.current_generation,
                    entry,
                    caller.instruction,
                    &mut invalid_cursor,
                )
                .unwrap();
            invalid_position.encoded.payload = &payload;
            assert_eq!(
                decode_user_consumer(
                    &wrong,
                    &invalid_position,
                    Opcode::GuardUserArgument,
                    &program
                )
                .err()
                .unwrap(),
                invalid(expected)
            );
        }
    }
}

#[test]
fn decoded_user_call_origin_borrows_and_falls_back_without_weakening_checks() {
    let (vm, _, entry) = fixture();
    let mut program = (*vm.generations[&vm.current_generation]).clone();
    let instruction = program
        .function(entry)
        .unwrap()
        .code
        .iter()
        .position(|instruction| instruction.opcode == Opcode::GuardUserArgument as u16)
        .unwrap();
    let mut cursor = None;
    let position = vm
        .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
        .unwrap();
    let (resolve, cached) = user_origin(&position, &program).unwrap();
    assert!(matches!(cached, Cow::Borrowed(_)));
    let expected = cached.into_owned();
    program.disable_user_call_specs_for_test();
    let (fallback_resolve, fallback) = user_origin(&position, &program).unwrap();
    assert_eq!(fallback_resolve, resolve);
    assert!(matches!(fallback, Cow::Owned(_)));
    assert_eq!(*fallback, expected);
}

#[test]
fn dynamic_user_call_borrowed_bindings_keep_nested_capture_and_defaults() {
    let source = "@SYSTEM_TITLE\nTRYCALLFORM TARGET(1, , NEXT_VALUE())\nRETURN RESULT\n\
        @NEXT_VALUE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 2\n\
        @TARGET(ARG, ARGS = \"fallback\", ARG:1 = 3)\nRESULT = ARG * 10 + ARG:1\nRESULTS '= ARGS\nRETURN RESULT\n";
    for maximum in [1, 16, 100_000] {
        let (mut cached, mut natives, entry) = fixture_source(source);
        let (mut uncached, mut other_natives, _) = fixture_source(source);
        for program in uncached.generations.values_mut() {
            Arc::make_mut(program).disable_user_call_specs_for_test();
        }
        cached.spawn_entry(entry, Vec::new()).unwrap();
        uncached.spawn_entry(entry, Vec::new()).unwrap();
        assert_eq!(
            finish(&mut cached, &mut natives, maximum),
            finish(&mut uncached, &mut other_natives, maximum)
        );
        let program = &cached.generations[&cached.current_generation];
        for (name, value) in [
            ("RESULT", VmValue::Integer(12)),
            ("RESULTS", VmValue::String("fallback".into())),
            ("FLAG", VmValue::Integer(1)),
        ] {
            let key = program
                .artifact
                .globals
                .iter()
                .find(|global| global.name == name)
                .unwrap()
                .key;
            assert_eq!(cached.read_variable(key, &[0], None).unwrap(), value);
        }
    }
}

#[derive(Debug, PartialEq)]
struct ExecutionObservation {
    instructions: u64,
    events: Vec<VmEvent>,
    snapshot: Vec<u8>,
    slices: Vec<(u64, crate::VmRunStop, Vec<VmEvent>)>,
}

fn finish(vm: &mut Vm, natives: &mut NativeServiceRegistry, maximum: u64) -> ExecutionObservation {
    let mut instructions = 0;
    let mut events = Vec::new();
    let mut slices = Vec::new();
    for _ in 0..1_000 {
        let report = vm.run_slice(
            &mut NoHost,
            natives,
            RunBudget {
                maximum_instructions: maximum,
                ..RunBudget::default()
            },
        );
        instructions += report.instructions;
        slices.push((report.instructions, report.stop, report.events.clone()));
        events.extend(report.events);
        if report.stop == crate::VmRunStop::Idle {
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{events:?}"
            );
            return ExecutionObservation {
                instructions,
                events,
                snapshot: vm.encode_unrestricted_snapshot(natives).unwrap(),
                slices,
            };
        }
    }
    panic!("bounded dynamic-call fixture did not finish");
}

#[test]
fn decoded_user_call_cache_preserves_complete_state_events_and_slice_accounting() {
    for maximum in [1, 16, 100_000] {
        let (mut cached, mut cached_natives, entry) = fixture();
        let (mut uncached, mut uncached_natives, _) = fixture();
        for generation in uncached.generations.values_mut() {
            Arc::make_mut(generation).disable_user_call_specs_for_test();
        }
        cached.spawn_entry(entry, Vec::new()).unwrap();
        uncached.spawn_entry(entry, Vec::new()).unwrap();
        assert_eq!(
            finish(&mut cached, &mut cached_natives, maximum),
            finish(&mut uncached, &mut uncached_natives, maximum)
        );
    }
}
