use super::*;

#[derive(Clone, Copy, Debug)]
enum WriteRoute {
    Variable,
    Place,
}

fn write(
    vm: &mut Vm,
    fiber: &mut Fiber,
    definition: &erabasic_bytecode::BytecodeGlobal,
    route: WriteRoute,
    index: u64,
    value: VmValue,
) -> Result<(), VmError> {
    match route {
        WriteRoute::Variable => vm.write_variable_resolved(
            fiber,
            vm.current_generation,
            definition,
            &[index],
            None,
            None,
            value,
        ),
        WriteRoute::Place => vm.write_place(
            fiber,
            &PlaceDescriptor {
                variable: definition.key,
                indices: vec![index],
                ..PlaceDescriptor::default()
            },
            value,
        ),
    }
}

#[test]
fn owned_writes_preserve_observation_and_failed_write_atomicity() {
    for route in [WriteRoute::Variable, WriteRoute::Place] {
        for observation in ["absent", "active", "invalid", "other_fiber"] {
            let (mut vm, artifact) = compile_vm("@READ\n#FUNCTION\nRETURNF 1\n");
            let function = artifact
                .functions
                .iter()
                .find(|f| f.name == "READ")
                .unwrap();
            let definition = artifact
                .globals
                .iter()
                .find(|g| g.name == "RESULTS")
                .unwrap();
            let id = vm.spawn_entry(function.key, Vec::new()).unwrap();
            let mut fiber = vm.fibers[&id].clone();
            vm.runnable.clear();
            if observation != "absent" {
                vm.begin_path_memo(
                    &fiber,
                    fiber.frames.last().unwrap().id,
                    function,
                    PathMemoHead {
                        generation: vm.current_generation,
                        function: function.key,
                    },
                    &[],
                    10_000,
                );
                if observation == "invalid" {
                    vm.active_path_memo.borrow_mut().as_mut().unwrap().valid = false;
                } else if observation == "other_fiber" {
                    let other = FiberId(id.0 + 1);
                    vm.active_path_memo.borrow_mut().as_mut().unwrap().fiber = other;
                    vm.active_path_memo_fiber.set(Some(other));
                }
            }
            let value = VmValue::String("玄関🙂".repeat(256));
            let generation = vm.current_generation;
            let revision = vm
                .memory
                .cell(generation, definition, 0)
                .unwrap()
                .revision();
            write(&mut vm, &mut fiber, definition, route, 2, value.clone()).unwrap();
            let cell = vm.memory.cell(generation, definition, 0).unwrap();
            assert_eq!(cell.read(&[2]), Ok(value.clone()));
            assert_eq!(cell.revision(), revision + 1);
            assert_observer(&vm, observation, id, definition.key, &value);
            for (index, bad_value) in [
                (u64::MAX, value.clone()),
                (2, VmValue::Integer(7)),
                (u64::MAX, VmValue::Integer(7)),
            ] {
                let mut expected_cell = vm.memory.cell(generation, definition, 0).unwrap().clone();
                let expected_error = match route {
                    WriteRoute::Variable => expected_cell
                        .write(&[index], bad_value.clone())
                        .map_err(VmError::InvalidState),
                    WriteRoute::Place => expected_cell
                        .write_execution(&[index], bad_value.clone())
                        .map_err(VmError::ScriptFailure),
                }
                .unwrap_err();
                assert_eq!(
                    write(&mut vm, &mut fiber, definition, route, index, bad_value),
                    Err(expected_error)
                );
                let cell = vm.memory.cell(generation, definition, 0).unwrap();
                assert_eq!(cell.revision(), revision + 1);
                assert_eq!(cell.read(&[2]), Ok(value.clone()));
                assert_observer(&vm, observation, id, definition.key, &value);
            }
        }
    }
}

fn assert_observer(vm: &Vm, observation: &str, id: FiberId, variable: SymbolKey, value: &VmValue) {
    let active = vm.active_path_memo.borrow();
    if observation == "absent" {
        assert!(active.is_none());
        assert_eq!(vm.active_path_memo_fiber.get(), None);
        return;
    }
    let active = active.as_ref().expect("observer must remain installed");
    let owner = if observation == "other_fiber" {
        FiberId(id.0 + 1)
    } else {
        id
    };
    assert_eq!(active.fiber, owner);
    assert_eq!(vm.active_path_memo_fiber.get(), Some(owner));
    assert_eq!(active.valid, observation != "invalid");
    if observation != "active" {
        assert!(active.mutations.is_empty());
        return;
    }
    let [
        PathMemoMutation::Write {
            place,
            value: retained,
        },
    ] = active.mutations.as_slice()
    else {
        panic!(
            "active observer must contain exactly one Write: {:?}",
            active.mutations
        );
    };
    assert_eq!(place.variable, variable);
    assert_eq!(place.generation, vm.current_generation);
    assert_eq!(place.character, 0);
    assert_eq!(place.indices, [2]);
    assert_eq!(retained, value);
}
