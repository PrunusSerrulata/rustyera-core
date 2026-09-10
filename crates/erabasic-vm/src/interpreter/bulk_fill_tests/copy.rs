use super::*;

fn copy_body(prefix: i64, start: i64, end: i64, step: i64, offset: i64) -> String {
    format!(
        "EXP:0:0 = 3\nEXP:0:1 = 5\nEXP:0:2 = 0\nEXP:0:3 = 8\n\
         LOCAL = {prefix}\nFOR LOCAL:1, {start}, {end}, {step}\n\
         TCVAR:LOCAL:({offset} + LOCAL:1) = EXP:LOCAL:(LOCAL:1)\nNEXT"
    )
}

#[test]
fn bulk_copy_real_dispatch_matches_scalar_cells_and_logical_instructions() {
    let body = copy_body(0, 0, 4, 1, 4);
    let (mut bulk, mut natives, target) = fixture(&body, crate::VmConfig::default());
    let (mut scalar, mut scalar_natives, _) = fixture(&body, crate::VmConfig::default());
    let bulk_used = dispatch_first_fill(&mut bulk, &mut natives, true, true);
    let scalar_used = dispatch_first_fill(&mut scalar, &mut scalar_natives, false, false);
    let bulk_result = finish(&mut bulk, &mut natives, 100_000);
    let scalar_result = finish(&mut scalar, &mut scalar_natives, 100_000);
    assert_eq!(bulk_used + bulk_result.0, scalar_used + scalar_result.0);
    assert_eq!(bulk_result.1, scalar_result.1);
    assert_eq!(bulk_result.2, scalar_result.2);
    for (index, expected) in [9, 9, 9, 9, 3, 5, 0, 8].into_iter().enumerate() {
        assert_eq!(
            bulk.read_variable(target, &[index as u64], Some(0))
                .unwrap(),
            VmValue::Integer(expected)
        );
    }
}

#[test]
fn bulk_copy_matches_scalar_slices_bounds_steps_and_overflow() {
    for (prefix, start, end, step, offset) in [
        (0, 0, 4, 1, 4),
        (0, 0, 0, 1, 4),
        (0, 3, -1, -1, 4),
        (0, 0, 4, 2, 4),
        (0, -1, 3, 1, 4),
        (0, 0, 4, 1, 14),
        (0, 0, 4, 1, -1),
        (0, 0, 4, 1, i64::MAX),
        (1, 0, 4, 1, 4),
    ] {
        for maximum in [1, 16, 100_000] {
            let body = copy_body(prefix, start, end, step, offset);
            let (mut bulk, mut natives, _) = fixture(&body, crate::VmConfig::default());
            let (mut scalar, mut scalar_natives, _) = fixture(&body, crate::VmConfig::default());
            Arc::make_mut(
                scalar
                    .generations
                    .get_mut(&scalar.current_generation)
                    .unwrap(),
            )
            .disable_bulk_fill_for_test();
            assert_eq!(
                finish(&mut bulk, &mut natives, maximum),
                finish(&mut scalar, &mut scalar_natives, maximum),
                "{body}, budget={maximum}"
            );
        }
    }
}

#[test]
fn bulk_copy_planner_rejects_aliases_changed_rows_and_side_effects() {
    for assignment in [
        "TCVAR:LOCAL:(4 + LOCAL:1) = TCVAR:LOCAL:(LOCAL:1)",
        "TCVAR:LOCAL:(4 + LOCAL:1) = EXP:1:(LOCAL:1)",
        "TCVAR:LOCAL:(4 + LOCAL:1) = EXP:LOCAL:(LOCAL:1)\nRESULT += 1",
        "TCVAR:LOCAL:(4 + LOCAL:1) += EXP:LOCAL:(LOCAL:1)",
    ] {
        let body = format!("FOR LOCAL:1, 0, 4\n{assignment}\nNEXT");
        let (vm, _, _) = fixture(&body, crate::VmConfig::default());
        let program = &vm.generations[&vm.current_generation];
        let function = &program.artifact.functions[0];
        assert!(
            (0..function.code.len())
                .all(|index| program.bulk_fill_loop_plan(function.key, index).is_none()),
            "{body}"
        );
    }
}

#[test]
fn bulk_copy_obeys_bounded_buffer_and_sparse_storage() {
    for end in [256, 257] {
        let source = format!(
            "@SYSTEM_TITLE\n#LOCALSIZE 2\n{}\nRETURN LOCAL:1\n",
            copy_body(0, 0, end, 1, 4)
        );
        let configure = |artifact: &mut erabasic_bytecode::BytecodeArtifact| {
            for global in &mut artifact.globals {
                if matches!(global.name.as_str(), "EXP" | "TCVAR") {
                    global.dimensions = vec![512];
                    global.initial_values.clear();
                }
            }
        };
        let (mut bulk, mut natives, target) =
            fixture_source_custom(source.clone(), crate::VmConfig::default(), configure);
        let (mut scalar, mut scalar_natives, _) =
            fixture_source_custom(source, crate::VmConfig::default(), configure);
        let actual = dispatch_first_loop(
            &mut bulk,
            &mut natives,
            end == 256,
            true,
            u64::try_from(end - 1).unwrap(),
        );
        let expected = dispatch_first_loop(&mut scalar, &mut scalar_natives, false, false, 0);
        finish_copy_pair(
            &mut bulk,
            &mut natives,
            &mut scalar,
            &mut scalar_natives,
            actual,
            expected,
        );
        for (index, expected) in [(4, 3), (5, 5), (6, 0), (7, 8), (end + 3, 0)] {
            assert_eq!(
                bulk.read_variable(target, &[u64::try_from(index).unwrap()], Some(0))
                    .unwrap(),
                VmValue::Integer(expected)
            );
        }
    }
}

fn finish_copy_pair(
    bulk: &mut Vm,
    natives: &mut NativeServiceRegistry,
    scalar: &mut Vm,
    scalar_natives: &mut NativeServiceRegistry,
    actual_prefix: u64,
    expected_prefix: u64,
) {
    let actual = finish(bulk, natives, 100_000);
    let expected = finish(scalar, scalar_natives, 100_000);
    assert!(
        actual
            .1
            .iter()
            .all(|event| !matches!(event, VmEvent::FiberFaulted { .. }))
    );
    assert!(
        actual
            .1
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. }))
    );
    assert_eq!(actual_prefix + actual.0, expected_prefix + expected.0);
    assert_eq!(actual.1, expected.1);
    assert_eq!(actual.2, expected.2);
}

fn array_cell_indices(vm: &Vm, key: SymbolKey, row: u64, column: u64) -> (Vec<u64>, Option<u64>) {
    if vm.generations[&vm.current_generation]
        .global(key)
        .unwrap()
        .storage
        == BytecodeStorage::Character
    {
        (vec![column], Some(row))
    } else {
        (vec![row, column], None)
    }
}

#[test]
fn bulk_copy_supports_project_rows_in_both_directions() {
    for (from, to) in [("DA", "TCVAR"), ("TCVAR", "DA"), ("DA", "DB")] {
        let make = || {
            let source = format!(
                "@SYSTEM_TITLE\n#LOCALSIZE 2\nLOCAL = 1\nFOR LOCAL:1, 0, 4\n{to}:LOCAL:(4 + LOCAL:1) = {from}:LOCAL:(LOCAL:1)\nNEXT\nRETURN LOCAL:1\n"
            );
            let (mut vm, natives, _) =
                fixture_source_custom(source, crate::VmConfig::default(), |artifact| {
                    for global in &mut artifact.globals {
                        if matches!(global.name.as_str(), "DA" | "DB") {
                            global.dimensions = vec![2, 16];
                            global.initial_values.clear();
                        }
                    }
                });
            let program = Arc::clone(&vm.generations[&vm.current_generation]);
            vm.memory.push_character(&program.artifact, None);
            let source = program.global_by_name(from).unwrap().key;
            let target = program.global_by_name(to).unwrap().key;
            for (row, column, value) in [(0, 4, 77), (1, 3, 91), (1, 8, 92), (1, 6, 99)] {
                let (indices, character) = array_cell_indices(&vm, target, row, column);
                vm.write_variable(target, &indices, character, VmValue::Integer(value))
                    .unwrap();
            }
            for (column, value) in [3, 5, 0, 8].into_iter().enumerate() {
                let (indices, character) = array_cell_indices(&vm, source, 1, column as u64);
                vm.write_variable(source, &indices, character, VmValue::Integer(value))
                    .unwrap();
            }
            (vm, natives, target)
        };
        let (mut bulk, mut natives, target) = make();
        let (mut scalar, mut scalar_natives, _) = make();
        let actual = dispatch_first_loop(&mut bulk, &mut natives, true, true, 3);
        let expected = dispatch_first_loop(&mut scalar, &mut scalar_natives, false, false, 0);
        finish_copy_pair(
            &mut bulk,
            &mut natives,
            &mut scalar,
            &mut scalar_natives,
            actual,
            expected,
        );
        for (row, column, value) in [
            (0, 4, 77),
            (1, 3, 91),
            (1, 4, 3),
            (1, 5, 5),
            (1, 6, 0),
            (1, 7, 8),
            (1, 8, 92),
        ] {
            let (indices, character) = array_cell_indices(&bulk, target, row, column);
            assert_eq!(
                bulk.read_variable(target, &indices, character).unwrap(),
                VmValue::Integer(value)
            );
        }
    }
}

#[test]
fn bulk_copy_source_overrun_preserves_partial_writes_counter_and_fault() {
    let make = || {
        let source = "@SYSTEM_TITLE\n#LOCALSIZE 2\nFOR LOCAL:1, 98, 102\nTCVAR:LOCAL:(4 + LOCAL:1) = EXP:LOCAL:(LOCAL:1)\nNEXT\nRETURN LOCAL:1\n".into();
        let (mut vm, natives, target) =
            fixture_source_custom(source, crate::VmConfig::default(), |artifact| {
                let target = artifact
                    .globals
                    .iter_mut()
                    .find(|g| g.name == "TCVAR")
                    .unwrap();
                target.dimensions = vec![512];
                target.initial_values.clear();
            });
        let program = &vm.generations[&vm.current_generation];
        let source = program.global_by_name("EXP").unwrap();
        assert_eq!(source.dimensions, [100]);
        let key = source.key;
        vm.write_variable(key, &[99], Some(0), VmValue::Integer(42))
            .unwrap();
        for index in 102..106 {
            vm.write_variable(target, &[index], Some(0), VmValue::Integer(9))
                .unwrap();
        }
        (vm, natives, target)
    };
    let (mut bulk, mut natives, target) = make();
    let (mut scalar, mut scalar_natives, _) = make();
    let actual_prefix = dispatch_first_loop(&mut bulk, &mut natives, false, true, 0);
    let expected_prefix = dispatch_first_loop(&mut scalar, &mut scalar_natives, false, false, 0);
    let actual = finish(&mut bulk, &mut natives, 100_000);
    let expected = finish(&mut scalar, &mut scalar_natives, 100_000);
    assert_eq!(actual_prefix + actual.0, expected_prefix + expected.0);
    assert_eq!(actual.1, expected.1);
    assert_eq!(actual.2, expected.2);
    let fault = actual
        .1
        .iter()
        .find_map(|event| match event {
            VmEvent::FiberFaulted { fault, .. } => Some(fault),
            _ => None,
        })
        .unwrap();
    assert_eq!(fault.code, VmFaultCode::Bounds);
    assert_eq!(fault.source.as_ref().unwrap().line, 4);
    let frame = bulk.fibers.values().next().unwrap().frames.last().unwrap();
    let program = &bulk.generations[&bulk.current_generation];
    let counter = program
        .scoped_variable(frame.function, "LOCAL")
        .unwrap()
        .key;
    let counter = PlaceDescriptor {
        variable: counter,
        indices: vec![1],
        frame: Some(frame.id),
        ..PlaceDescriptor::default()
    };
    assert_eq!(
        bulk.read_place(bulk.fibers.values().next().unwrap(), &counter)
            .unwrap(),
        VmValue::Integer(100)
    );
    for (index, value) in [(102, 0), (103, 42), (104, 9), (105, 9)] {
        assert_eq!(
            bulk.read_variable(target, &[index], Some(0)).unwrap(),
            VmValue::Integer(value)
        );
    }
}

#[test]
fn bulk_copy_reference_metadata_disables_each_plan_dependency() {
    let body = "FOR COUNT, 0, 4\nTCVAR:LOCAL:(4 + COUNT) = EXP:LOCAL:COUNT\nNEXT";
    let (vm, _, _) = fixture(body, crate::VmConfig::default());
    let program = &vm.generations[&vm.current_generation];
    let function = &program.artifact.functions[0];
    let instruction = (0..function.code.len())
        .find(|i| program.bulk_fill_loop_plan(function.key, *i).is_some())
        .unwrap();
    let plan = program
        .bulk_fill_loop_plan(function.key, instruction)
        .unwrap();
    let crate::state::BulkArrayOperation::Copy { source, .. } = &plan.operation else {
        panic!("copy plan");
    };
    assert_ne!(plan.prefix, plan.counter);
    for key in [*source, plan.target, plan.prefix, plan.counter] {
        let mut artifact = (*program.artifact).clone();
        artifact
            .runtime_variables
            .iter_mut()
            .find(|symbol| symbol.key == key)
            .unwrap()
            .reference = true;
        let guarded = ProgramGeneration::new(Arc::new(artifact));
        assert!(
            guarded
                .bulk_fill_loop_plan(function.key, instruction)
                .is_none()
        );
    }
}

#[test]
fn bulk_copy_preserves_dense_sparse_values_and_exact_cell_revisions() {
    for (dense_source, dense_target) in [(false, false), (true, false), (false, true), (true, true)]
    {
        let make = || {
            let source = "@SYSTEM_TITLE\n#LOCALSIZE 2\nFOR LOCAL:1, 0, 4\nTCVAR:LOCAL:(4 + LOCAL:1) = EXP:LOCAL:(LOCAL:1)\nNEXT\nRETURN LOCAL:1\n".into();
            let (mut vm, natives, target) =
                fixture_source_custom(source, crate::VmConfig::default(), |artifact| {
                    for global in &mut artifact.globals {
                        let dense = match global.name.as_str() {
                            "EXP" => dense_source,
                            "TCVAR" => dense_target,
                            _ => continue,
                        };
                        global.dimensions = vec![512];
                        global.initial_values = if dense {
                            vec![erabasic_bytecode::BytecodeConstant::Integer(0); 512]
                        } else {
                            Vec::new()
                        };
                    }
                });
            let key = vm.generations[&vm.current_generation]
                .global_by_name("EXP")
                .unwrap()
                .key;
            for (index, value) in [3, 5, 0, 8].into_iter().enumerate() {
                vm.write_variable(key, &[index as u64], Some(0), VmValue::Integer(value))
                    .unwrap();
            }
            (vm, natives, target, key)
        };
        let (mut bulk, mut natives, target, source) = make();
        let (mut scalar, mut scalar_natives, _, _) = make();
        let program = Arc::clone(&bulk.generations[&bulk.current_generation]);
        let target = program.global(target).unwrap();
        let source = program.global(source).unwrap();
        let source_cell = bulk
            .memory
            .cell(bulk.current_generation, source, 0)
            .unwrap();
        let target_cell = bulk
            .memory
            .cell(bulk.current_generation, target, 0)
            .unwrap();
        assert_eq!(source_cell.integers().is_some(), dense_source);
        assert_eq!(target_cell.integers().is_some(), dense_target);
        let revisions = (source_cell.revision(), target_cell.revision());
        let actual = dispatch_first_loop(&mut bulk, &mut natives, true, true, 3);
        let expected = dispatch_first_loop(&mut scalar, &mut scalar_natives, false, false, 0);
        assert_eq!(
            bulk.memory
                .cell(bulk.current_generation, source, 0)
                .unwrap()
                .revision(),
            revisions.0
        );
        let target_cell = bulk
            .memory
            .cell(bulk.current_generation, target, 0)
            .unwrap();
        assert_eq!(target_cell.revision(), revisions.1 + 4);
        assert_eq!(target_cell.integers().is_some(), dense_target);
        assert_eq!(target_cell.get(6), Some(VmValue::Integer(0)));
        finish_copy_pair(
            &mut bulk,
            &mut natives,
            &mut scalar,
            &mut scalar_natives,
            actual,
            expected,
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn bulk_copy_direct_path_retains_debug_stack_budget_and_trace_barriers() {
    let (mut vm, _, target) = fixture(&copy_body(0, 0, 4, 1, 4), crate::VmConfig::default());
    let id = *vm.fibers.keys().next().unwrap();
    let mut fiber = vm.fibers.remove(&id).unwrap();
    let frame = fiber.frames.last().unwrap();
    let entry = frame.function;
    let program = &vm.generations[&vm.current_generation];
    let instruction = (0..program.artifact.functions[0].code.len())
        .find(|index| program.bulk_fill_loop_plan(entry, *index).is_some())
        .unwrap();
    let plan = program.bulk_fill_loop_plan(entry, instruction).unwrap();
    let logical = 4 * plan.iteration_instructions + 2;
    let peak = plan.stack_peak;
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
        remaining_quantum: u32::try_from(logical).unwrap(),
        remaining_instructions: logical,
    };
    let before = fiber.clone();
    let memory_before = vm.memory.clone();
    let revisions_before = memory_revisions(&vm.memory);
    let mut leased_counter = counter.clone();
    leased_counter.backing = Some(crate::ArrayBackingId(1));
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &leased_counter, 0, 4, 1, allowed)
            .unwrap(),
        None
    );
    assert_eq!(vm.memory, memory_before);
    assert_eq!(memory_revisions(&vm.memory), revisions_before);
    assert_eq!(fiber, before);
    for name in ["EXP", "TCVAR"] {
        let program = Arc::clone(&vm.generations[&vm.current_generation]);
        let definition = program.global_by_name(name).unwrap();
        let cell = vm
            .memory
            .cell_mut(vm.current_generation, definition.key, definition.storage, 0)
            .unwrap();
        let dimensions = cell.dimensions.clone();
        cell.dimensions.push(1);
        let malformed = vm.memory.clone();
        let revisions = memory_revisions(&vm.memory);
        assert_eq!(
            vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
                .unwrap(),
            None
        );
        assert_eq!(vm.memory, malformed);
        assert_eq!(memory_revisions(&vm.memory), revisions);
        assert_eq!(fiber, before);
        vm.memory
            .cell_mut(vm.current_generation, definition.key, definition.storage, 0)
            .unwrap()
            .dimensions = dimensions;
    }
    for blocked in [
        ExecutionPolicy {
            allow_function_memo: false,
            ..allowed
        },
        ExecutionPolicy {
            remaining_quantum: u32::try_from(logical - 1).unwrap(),
            ..allowed
        },
        ExecutionPolicy {
            remaining_instructions: logical - 1,
            ..allowed
        },
    ] {
        assert_eq!(
            vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, blocked)
                .unwrap(),
            None
        );
        assert_eq!(fiber, before);
    }
    vm.config.maximum_operand_stack = peak - 1;
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        None
    );
    vm.config.maximum_operand_stack = peak;
    let runnable = std::mem::take(&mut vm.runnable);
    let program = Arc::clone(&vm.generations[&vm.current_generation]);
    let mut traced_function = program.function(entry).unwrap().clone();
    traced_function.result = Some(BytecodeType::Integer);
    vm.begin_path_memo(
        &fiber,
        fiber.frames.last().unwrap().id,
        &traced_function,
        Vm::path_memo_head(vm.current_generation, entry, &[]).unwrap(),
        &[],
        1000,
    );
    assert!(vm.path_memo_is_active_for(id));
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        None
    );
    vm.abort_path_memo(id);
    vm.runnable = runnable;
    vm.config.maximum_backward_branches_without_progress = 2;
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        None
    );
    assert_eq!(fiber, before);
    assert_eq!(
        vm.read_variable(target, &[4], Some(0)).unwrap(),
        VmValue::Integer(9)
    );
    vm.config.maximum_backward_branches_without_progress = 10_000;
    assert_eq!(
        vm.try_bulk_fill_loop(&mut fiber, &position, &counter, 0, 4, 1, allowed)
            .unwrap(),
        Some(logical - 1)
    );
    assert_eq!(
        vm.read_variable(target, &[4], Some(0)).unwrap(),
        VmValue::Integer(0)
    );
}

fn memory_revisions(memory: &crate::Memory) -> Vec<u64> {
    assert!(memory.legacy.is_empty());
    memory
        .shared
        .values()
        .chain(memory.statics.values())
        .chain(
            memory
                .characters
                .iter()
                .flat_map(|character| character.values()),
        )
        .map(crate::VariableCell::revision)
        .collect()
}
