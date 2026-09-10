use super::literal_groupmatch_tests::compile_cursor_fixture;
use super::*;
mod copy;

struct NoHost;
impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("array fill fixture must not call Host");
    }
}

fn fixture(body: &str, config: crate::VmConfig) -> (Vm, NativeServiceRegistry, SymbolKey) {
    fixture_source(
        format!("@SYSTEM_TITLE\n#LOCALSIZE 2\n{body}\nRESULT = LOCAL:1\nRETURN RESULT\n"),
        config,
    )
}

fn fixture_source(
    source: String,
    config: crate::VmConfig,
) -> (Vm, NativeServiceRegistry, SymbolKey) {
    fixture_source_custom(source, config, |_| {})
}

fn fixture_source_custom(
    source: String,
    config: crate::VmConfig,
    configure: impl FnOnce(&mut erabasic_bytecode::BytecodeArtifact),
) -> (Vm, NativeServiceRegistry, SymbolKey) {
    let mut artifact = compile_cursor_fixture(source);
    // Keep both storage variants small; bytecode still comes from the real compiler and validator.
    for global in &mut artifact.globals {
        if global.name == "TCVAR" {
            global.dimensions = vec![16];
            global.initial_values.truncate(16);
        } else if global.name == "DA" {
            global.dimensions = vec![2, 8];
            global.initial_values.truncate(16);
        }
    }
    configure(&mut artifact);
    artifact.refresh_ids().unwrap();
    let entry = artifact.functions[0].key;
    let target = artifact
        .globals
        .iter()
        .find(|global| global.name == "TCVAR")
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
    let mut vm = Vm::new(validation.value.unwrap(), config);
    vm.memory.push_character(&artifact, None);
    for index in 0..8 {
        vm.write_variable(target, &[index], Some(0), VmValue::Integer(9))
            .unwrap();
    }
    vm.spawn_entry(entry, Vec::new()).unwrap();
    (vm, natives, target)
}

fn dispatch_first_fill(
    vm: &mut Vm,
    natives: &mut NativeServiceRegistry,
    bulk: bool,
    allow_bulk: bool,
) -> u64 {
    dispatch_first_loop(vm, natives, bulk, allow_bulk, 3)
}

fn dispatch_first_loop(
    vm: &mut Vm,
    natives: &mut NativeServiceRegistry,
    bulk: bool,
    allow_bulk: bool,
    backward: u64,
) -> u64 {
    let id = *vm.fibers.keys().next().unwrap();
    let entry = vm.fibers[&id].frames.last().unwrap().function;
    let program = &vm.generations[&vm.current_generation];
    let instruction = program.artifact.functions[0]
        .code
        .iter()
        .position(|encoded| encoded.opcode == Opcode::ForStart as u16)
        .unwrap();
    let after_loop = program
        .bulk_fill_loop_plan(entry, instruction)
        .unwrap()
        .after_loop;
    let mut used = 0;
    while vm.fibers[&id].frames.last().unwrap().instruction != instruction {
        assert!(used < 100, "fixture did not reach ForStart");
        let report = vm.run_slice(
            &mut NoHost,
            natives,
            RunBudget {
                maximum_instructions: 1,
                ..RunBudget::default()
            },
        );
        assert_eq!(report.instructions, 1);
        assert!(report.events.is_empty());
        used += report.instructions;
    }
    let mut fiber = vm.fibers.remove(&id).unwrap();
    let stack = &fiber.frames.last().unwrap().stack;
    let VmValue::IntegerPlace(counter) = &stack[stack.len() - 4] else {
        panic!("counter operand")
    };
    let counter = counter.clone();
    let end = stack[stack.len() - 2].clone();
    let mut cursor = None;
    let position = vm
        .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
        .unwrap();
    let outcome = vm
        .execute_instruction(
            &mut fiber,
            &position,
            &mut NoHost,
            natives,
            &mut 0,
            ExecutionPolicy {
                allow_function_memo: allow_bulk,
                allow_immediate_host: false,
                remaining_quantum: 100_000,
                remaining_instructions: 100_000,
            },
        )
        .unwrap();
    if bulk {
        let StepOutcome::BulkProgress(additional) = outcome else {
            panic!("real dispatch did not take bulk path")
        };
        used += 1 + additional;
        assert_eq!(fiber.frames.last().unwrap().instruction, after_loop);
        assert_eq!(vm.read_place(&fiber, &counter).unwrap(), end);
        assert_eq!(fiber.backward_branches_without_progress, backward);
    } else {
        assert!(matches!(outcome, StepOutcome::Continue));
        used += 1;
    }
    vm.fibers.insert(id, fiber);
    used
}

#[test]
fn bulk_fill_real_dispatch_covers_character_and_project_storage() {
    for (body, sparse, bulk) in [
        (
            "FOR LOCAL:1, 0, 4\nTCVAR:LOCAL:(LOCAL:1) = 0\nNEXT",
            false,
            true,
        ),
        (
            "FOR LOCAL:1, 0, 4\nDA:LOCAL:(LOCAL:1) = 0\nNEXT",
            false,
            true,
        ),
        ("FOR LOCAL, 0, 4\nDA:FLAG:LOCAL = 0\nNEXT", false, true),
        ("FOR LOCAL, 0, 4\nDA:FLAG:LOCAL = 7\nNEXT", false, true),
        ("FOR LOCAL, 0, 4\nDA:FLAG:LOCAL = 7\nNEXT", true, false),
    ] {
        let prepare = || {
            let (mut vm, natives, _) = fixture(body, crate::VmConfig::default());
            let target = vm.generations[&vm.current_generation]
                .artifact
                .globals
                .iter()
                .find(|global| global.name == "DA")
                .unwrap()
                .key;
            vm.write_variable(target, &[0, 4], None, VmValue::Integer(9))
                .unwrap();
            if sparse {
                let cell = vm
                    .memory
                    .cell_mut(vm.current_generation, target, BytecodeStorage::Project, 0)
                    .unwrap();
                *cell = serde_json::from_slice(&serde_json::to_vec(&*cell).unwrap()).unwrap();
                assert!(cell.integers().is_none());
            }
            (vm, natives, target)
        };
        let (mut optimized, mut natives, target) = prepare();
        let (mut scalar, mut scalar_natives, _) = prepare();
        let dispatched = dispatch_first_fill(&mut optimized, &mut natives, bulk, true);
        let scalar_dispatched = dispatch_first_fill(&mut scalar, &mut scalar_natives, false, false);
        let mut actual = finish(&mut optimized, &mut natives, 100_000);
        actual.0 += dispatched;
        let mut expected = finish(&mut scalar, &mut scalar_natives, 100_000);
        expected.0 += scalar_dispatched;
        assert_eq!(actual, expected, "{body}");
        assert_eq!(
            optimized.read_variable(target, &[0, 4], None).unwrap(),
            VmValue::Integer(9)
        );
        if sparse {
            let global = optimized.generations[&optimized.current_generation]
                .global(target)
                .unwrap();
            assert!(
                optimized
                    .memory
                    .cell(optimized.current_generation, global, 0)
                    .unwrap()
                    .integers()
                    .is_none()
            );
        }
    }
}

#[test]
fn bulk_fill_path_memo_replays_ranges_and_revalidates_character_prefix() {
    let source = "@SYSTEM_TITLE\nRESULT = CLEAR_RANGE()\nRETURN RESULT\n\
        @CLEAR_RANGE\n#FUNCTION\n#LOCALSIZE 2\n\
        FOR LOCAL:1, 1, 7\nTCVAR:(FLAG:0):(LOCAL:1) = 0\nNEXT\nRETURNF 7\n";
    let (mut vm, mut natives, target) = fixture_source(source.into(), crate::VmConfig::default());
    let artifact = Arc::clone(&vm.generations[&vm.current_generation].artifact);
    let entry = artifact.functions[0].key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    vm.memory.push_character(&artifact, None);
    for character in [0, 1] {
        vm.write_variable(target, &[0], Some(character), VmValue::Integer(17))
            .unwrap();
        vm.write_variable(target, &[7], Some(character), VmValue::Integer(19))
            .unwrap();
        vm.write_variable(target, &[2], Some(character), VmValue::Integer(23))
            .unwrap();
    }
    let first = finish(&mut vm, &mut natives, 100_000);
    assert!(
        !first
            .1
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
    assert!(
        !vm.path_memo_cache.is_empty(),
        "first fill must be captured"
    );
    let replays = vm.path_memo_replays;
    vm.write_variable(target, &[2], Some(0), VmValue::Integer(29))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    finish(&mut vm, &mut natives, 100_000);
    assert!(
        vm.path_memo_replays > replays,
        "same prefix must physically replay"
    );
    assert_eq!(
        vm.read_variable(target, &[2], Some(0)).unwrap(),
        VmValue::Integer(0)
    );
    vm.write_variable(flag, &[0], None, VmValue::Integer(1))
        .unwrap();
    vm.write_variable(target, &[0], Some(1), VmValue::Integer(31))
        .unwrap();
    vm.write_variable(target, &[7], Some(1), VmValue::Integer(37))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let changed = finish(&mut vm, &mut natives, 100_000);
    assert!(
        !changed
            .1
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
    assert_eq!(
        vm.read_variable(target, &[2], Some(1)).unwrap(),
        VmValue::Integer(0)
    );
    for (character, before, after) in [(0, 17, 19), (1, 31, 37)] {
        assert_eq!(
            vm.read_variable(target, &[0], Some(character)).unwrap(),
            VmValue::Integer(before)
        );
        assert_eq!(
            vm.read_variable(target, &[7], Some(character)).unwrap(),
            VmValue::Integer(after)
        );
    }
}

fn finish(
    vm: &mut Vm,
    natives: &mut NativeServiceRegistry,
    maximum: u64,
) -> (u64, Vec<VmEvent>, Vec<u8>) {
    let mut instructions = 0;
    let mut events = Vec::new();
    for _ in 0..10_000 {
        let report = vm.run_slice(
            &mut NoHost,
            natives,
            RunBudget {
                maximum_instructions: maximum,
                ..RunBudget::default()
            },
        );
        instructions += report.instructions;
        events.extend(report.events);
        if report.stop == crate::VmRunStop::Idle {
            return (
                instructions,
                events,
                vm.encode_unrestricted_snapshot(natives).unwrap(),
            );
        }
    }
    panic!("bounded fixture did not finish");
}

#[test]
fn bulk_fill_character_indexed_counter_matches_scalar_slices_and_snapshot() {
    for (start, end, step) in [(1, 7, 1), (0, 1, 1), (0, 0, 1), (7, 1, -1), (-1, 3, 1)] {
        let body = format!("FOR LOCAL:1, {start}, {end}, {step}\nTCVAR:LOCAL:(LOCAL:1) = 0\nNEXT");
        // Same budgets preserve the serialized scheduler exhaustion counter too.
        for maximum in [1, 16, 100_000] {
            let (mut bulk, mut bulk_natives, target) = fixture(&body, crate::VmConfig::default());
            let (mut scalar, mut scalar_natives, _) = fixture(&body, crate::VmConfig::default());
            Arc::make_mut(
                scalar
                    .generations
                    .get_mut(&scalar.current_generation)
                    .unwrap(),
            )
            .disable_bulk_fill_for_test();
            let bulk_result = finish(&mut bulk, &mut bulk_natives, maximum);
            let scalar_result = finish(&mut scalar, &mut scalar_natives, maximum);
            assert_eq!(bulk_result, scalar_result, "{body}");
            if start == 1 && end == 7 && step == 1 {
                for index in 0..8 {
                    assert_eq!(
                        bulk.read_variable(target, &[index], Some(0)).unwrap(),
                        VmValue::Integer(if (1..7).contains(&index) { 0 } else { 9 })
                    );
                }
            }
        }
    }
}

#[test]
fn bulk_fill_fallback_preserves_character_fault_and_backward_budget() {
    let (probe, _, target) = fixture("", crate::VmConfig::default());
    let length = probe.generations[&probe.current_generation]
        .global(target)
        .unwrap()
        .dimensions[0];
    let absent_character = i64::try_from(probe.memory.characters.len()).unwrap();
    for (prefix, start, end, backward) in [
        (absent_character, 0, 7, 10_000),
        (-1, 0, 7, 10_000),
        (0, length - 1, length + 1, 10_000),
        (0, 0, 7, 2),
    ] {
        let body = format!(
            "LOCAL = {prefix}\nFOR LOCAL:1, {start}, {end}\nTCVAR:LOCAL:(LOCAL:1) = 0\nNEXT"
        );
        let config = crate::VmConfig {
            maximum_backward_branches_without_progress: backward,
            ..crate::VmConfig::default()
        };
        let (mut bulk, mut bulk_natives, _) = fixture(&body, config);
        let (mut scalar, mut scalar_natives, _) = fixture(&body, config);
        Arc::make_mut(
            scalar
                .generations
                .get_mut(&scalar.current_generation)
                .unwrap(),
        )
        .disable_bulk_fill_for_test();
        let actual = finish(&mut bulk, &mut bulk_natives, 100_000);
        assert!(
            actual
                .1
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{body}"
        );
        assert_eq!(
            actual,
            finish(&mut scalar, &mut scalar_natives, 100_000),
            "{body}"
        );
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One prepared cursor compares all pre-mutation barriers and the exact successful bulk accounting."
)]
fn bulk_fill_direct_path_obeys_debug_stack_and_slice_barriers() {
    let (mut vm, _, target) = fixture(
        "FOR LOCAL:1, 0, 4\nTCVAR:LOCAL:(LOCAL:1) = 0\nNEXT",
        crate::VmConfig::default(),
    );
    let id = *vm.fibers.keys().next().unwrap();
    let mut fiber = vm.fibers.remove(&id).unwrap();
    let frame = fiber.frames.last().unwrap();
    let entry = frame.function;
    let program = &vm.generations[&vm.current_generation];
    let instruction = (0..program.artifact.functions[0].code.len())
        .find(|instruction| program.bulk_fill_loop_plan(entry, *instruction).is_some())
        .unwrap();
    let plan = program.bulk_fill_loop_plan(entry, instruction).unwrap();
    assert_eq!(plan.iteration_instructions, 8);
    let after_loop = plan.after_loop;
    let counter = PlaceDescriptor {
        variable: plan.counter,
        indices: vec![1],
        fiber: Some(id),
        frame: Some(frame.id),
        ..PlaceDescriptor::default()
    };
    let mut cursor = None;
    let position = vm
        .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
        .unwrap();
    let allowed = ExecutionPolicy {
        allow_function_memo: true,
        allow_immediate_host: false,
        remaining_quantum: 34,
        remaining_instructions: 34,
    };
    let before = fiber.clone();
    for blocked in [
        ExecutionPolicy {
            allow_function_memo: false,
            ..allowed
        },
        ExecutionPolicy {
            remaining_quantum: 33,
            ..allowed
        },
        ExecutionPolicy {
            remaining_instructions: 33,
            ..allowed
        },
    ] {
        assert_eq!(
            vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, blocked)
                .unwrap(),
            None
        );
        assert_eq!(fiber, before);
        assert_eq!(
            vm.read_variable(target, &[0], Some(0)).unwrap(),
            VmValue::Integer(9)
        );
    }
    vm.config.maximum_operand_stack = 2;
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        None
    );
    assert_eq!(fiber, before);
    vm.config.maximum_operand_stack = 3;
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        Some(33)
    );
    assert_eq!(fiber.frames.last().unwrap().instruction, after_loop);
    assert_eq!(
        vm.read_variable(target, &[0], Some(0)).unwrap(),
        VmValue::Integer(0)
    );
    assert_eq!(
        vm.read_variable(target, &[4], Some(0)).unwrap(),
        VmValue::Integer(9)
    );
    assert_eq!(fiber.backward_branches_without_progress, 3);
}

#[test]
fn bulk_fill_plan_recognizes_disjoint_local_cells_and_rejects_mutated_prefix() {
    for (prefix, counter, fill, expected) in [
        ("LOCAL", "LOCAL:1", "0", true),
        ("LOCAL:0", "LOCAL:1", "0", true),
        ("LOCAL", "LOCAL:0", "0", false),
        ("LOCAL:1", "LOCAL:1", "0", false),
        ("LOCAL", "LOCAL:1", "1", false),
        ("LOCAL", "LOCAL:1", "LOCAL", false),
    ] {
        let body = format!("FOR {counter}, 0, 4\nTCVAR:({prefix}):({counter}) = {fill}\nNEXT");
        let (vm, _, _) = fixture(&body, crate::VmConfig::default());
        let program = &vm.generations[&vm.current_generation];
        let function = &program.artifact.functions[0];
        assert_eq!(
            (0..function.code.len()).any(|instruction| program
                .bulk_fill_loop_plan(function.key, instruction)
                .is_some()),
            expected,
            "{body}"
        );
    }
}

#[test]
fn bulk_fill_reference_metadata_disables_planning() {
    let (vm, _, target) = fixture(
        "FOR LOCAL:1, 0, 4\nTCVAR:LOCAL:(LOCAL:1) = 0\nNEXT",
        crate::VmConfig::default(),
    );
    let program = &vm.generations[&vm.current_generation];
    let function = &program.artifact.functions[0];
    let instruction = (0..function.code.len())
        .find(|instruction| {
            program
                .bulk_fill_loop_plan(function.key, *instruction)
                .is_some()
        })
        .unwrap();
    let counter = program
        .bulk_fill_loop_plan(function.key, instruction)
        .unwrap()
        .counter;
    for key in [counter, target] {
        let mut artifact = (*program.artifact).clone();
        artifact
            .runtime_variables
            .iter_mut()
            .find(|symbol| symbol.key == key)
            .expect("compiled runtime variable metadata")
            .reference = true;
        let guarded = ProgramGeneration::new(Arc::new(artifact));
        assert!(
            guarded
                .bulk_fill_loop_plan(function.key, instruction)
                .is_none()
        );
    }
}
