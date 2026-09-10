use super::*;
use crate::interpreter::literal_groupmatch_tests::compile_cursor_fixture_with_options;
use crate::{Fiber, Vm, VmError};
use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};

const LENGTH: usize = 512;

#[derive(Clone)]
struct Fixture {
    vm: Vm,
    fiber: Fiber,
    place: PlaceDescriptor,
    target: SymbolKey,
    string: bool,
}

struct NoHost;
impl crate::VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("local write fixture must not call Host");
    }
}

fn fixture(profile: CompatibilityProfileId, dense: bool, reference: bool, string: bool) -> Fixture {
    let local = "VALUE";
    let declaration = if string { "DIMS" } else { "DIM" };
    let body = if reference {
        format!("CALL CALLEE, {local}\nRETURN\n@CALLEE(TEXT)\n#{declaration} REF TEXT\nRETURN\n")
    } else {
        "RETURN\n".into()
    };
    let mut options = erabasic_analyzer::AnalyzerOptions::analysis_mode();
    options.compatibility = CompatibilityIdentity::for_profile(profile);
    let mut artifact = compile_cursor_fixture_with_options(
        format!("@SYSTEM_TITLE\n#{declaration} DYNAMIC {local}, {LENGTH}\n{body}"),
        &options,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|f| f.name == "SYSTEM_TITLE")
        .unwrap()
        .key;
    let target = artifact
        .globals
        .iter_mut()
        .find(|g| g.name == local && g.owner == Some(entry))
        .unwrap();
    assert_eq!(target.storage, BytecodeStorage::FunctionLocal);
    target.initial_values = if dense {
        vec![
            if string {
                BytecodeConstant::String(String::new())
            } else {
                BytecodeConstant::Integer(0)
            };
            LENGTH
        ]
    } else {
        Vec::new()
    };
    let target = target.key;
    artifact.refresh_ids().unwrap();
    let validation = erabasic_validator::validate_bytecode(
        artifact.clone().into_unvalidated(),
        &erabasic_compiler::runtime_native_validation_context(
            &artifact,
            &erabasic_compiler::default_host_registry(),
        ),
    );
    assert!(
        validation.diagnostics.is_empty(),
        "{:?}",
        validation.diagnostics
    );
    let mut vm = Vm::new(validation.value.unwrap(), crate::VmConfig::default());
    let id = vm.spawn_entry(entry, Vec::new()).unwrap();
    if reference {
        let callee = artifact
            .functions
            .iter()
            .find(|f| f.name == "CALLEE")
            .unwrap()
            .key;
        let mut natives = crate::NativeServiceRegistry::for_artifact(&artifact);
        for _ in 0..100 {
            if vm.fibers[&id].frames.last().unwrap().function == callee {
                break;
            }
            let report = vm.run_slice(
                &mut NoHost,
                &mut natives,
                crate::RunBudget {
                    maximum_instructions: 1,
                    ..crate::RunBudget::default()
                },
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, crate::VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
        }
        assert_eq!(vm.fibers[&id].frames.last().unwrap().function, callee);
    }
    finish_fixture(vm, id, &artifact, target, (dense, reference, string))
}

fn finish_fixture(
    mut vm: Vm,
    id: crate::FiberId,
    artifact: &BytecodeArtifact,
    target: SymbolKey,
    (dense, reference, string): (bool, bool, bool),
) -> Fixture {
    let mut fiber = vm.fibers.remove(&id).unwrap();
    let frame = fiber.frames.last().unwrap();
    let variable = if reference {
        artifact
            .globals
            .iter()
            .find(|g| g.name == "TEXT" && g.owner == Some(frame.function))
            .unwrap()
            .key
    } else {
        target
    };
    let place = PlaceDescriptor {
        variable,
        fiber: Some(id),
        frame: Some(frame.id),
        ..PlaceDescriptor::default()
    };
    let cell = fiber
        .frames
        .first_mut()
        .unwrap()
        .locals
        .get_mut(&target)
        .unwrap();
    assert_eq!(
        matches!(
            &cell.values,
            VariableValues::SparseStrings { .. } | VariableValues::SparseIntegers { .. }
        ),
        !dense
    );
    cell.set(
        0,
        if string {
            VmValue::String("玄関🙂".repeat(4096))
        } else {
            VmValue::Integer(71)
        },
    )
    .unwrap();
    let result = Fixture {
        vm,
        fiber,
        place,
        target,
        string,
    };
    if reference {
        let binding = result.fiber.frames.last().unwrap().locals[&variable]
            .first_place()
            .unwrap();
        assert!(binding.backing.is_some());
        result
            .vm
            .checked_array_backing(&result.fiber, &binding)
            .unwrap();
    }
    result
}

impl Fixture {
    fn cell(&self) -> &VariableCell {
        &self.fiber.frames.first().unwrap().locals[&self.target]
    }
    fn value(&self, value: i64) -> VmValue {
        if self.string {
            VmValue::String(if value == 0 {
                String::new()
            } else {
                value.to_string()
            })
        } else {
            VmValue::Integer(value)
        }
    }
    fn assert_binding(
        &self,
        before: &VariableCell,
        leases: &crate::state::array_leases::ArrayLeases,
    ) {
        assert_eq!(&self.vm.memory.array_leases, leases);
        if self.place.variable != self.target {
            let binding = &self.fiber.frames.last().unwrap().locals[&self.place.variable];
            assert_eq!(binding, before);
            assert_eq!(binding.revision(), before.revision());
            self.vm
                .checked_array_backing(&self.fiber, &binding.first_place().unwrap())
                .unwrap();
        }
    }
}

#[test]
fn local_place_writes_preserve_dense_sparse_values_bindings_and_revisions() {
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        for dense in [false, true] {
            for reference in [false, true] {
                for string in [false, true] {
                    let initial = fixture(profile, dense, reference, string);
                    for operation in 0..3 {
                        assert_success(initial.clone(), operation);
                        assert_errors(initial.clone(), operation);
                    }
                }
            }
        }
    }
}

fn assert_success(mut fixture: Fixture, operation: usize) {
    let before = fixture.cell().clone();
    let binding = fixture.fiber.frames.last().unwrap().locals[&fixture.place.variable].clone();
    let leases = fixture.vm.memory.array_leases.clone();
    let mut expected = before.to_values();
    let value = fixture.value(19);
    match operation {
        0 => {
            fixture
                .vm
                .write_place(
                    &mut fixture.fiber,
                    &PlaceDescriptor {
                        indices: vec![1],
                        ..fixture.place.clone()
                    },
                    value.clone(),
                )
                .unwrap();
            expected[1] = value;
        }
        1 => {
            fixture
                .vm
                .fill_place_array_range(&mut fixture.fiber, &fixture.place, 1, 3, value.clone())
                .unwrap();
            expected[1..3].fill(value);
        }
        2 => {
            expected = vec![fixture.value(0); LENGTH];
            expected[511] = value;
            fixture
                .vm
                .write_place_array(&mut fixture.fiber, &fixture.place, expected.clone())
                .unwrap();
        }
        _ => unreachable!(),
    }
    assert_eq!(fixture.cell().to_values(), expected);
    assert_eq!(fixture.cell().revision(), before.revision() + 1);
    fixture.assert_binding(&binding, &leases);
}

fn assert_errors(mut fixture: Fixture, operation: usize) {
    let before = fixture.cell().clone();
    let binding = fixture.fiber.frames.last().unwrap().locals[&fixture.place.variable].clone();
    let leases = fixture.vm.memory.array_leases.clone();
    let value = fixture.value(19);
    let result = match operation {
        0 => fixture.vm.write_place(
            &mut fixture.fiber,
            &PlaceDescriptor {
                indices: vec![LENGTH as u64],
                ..fixture.place.clone()
            },
            value,
        ),
        1 => fixture.vm.fill_place_array_range(
            &mut fixture.fiber,
            &fixture.place,
            1,
            LENGTH + 1,
            value,
        ),
        2 => fixture
            .vm
            .write_place_array(&mut fixture.fiber, &fixture.place, vec![value]),
        _ => unreachable!(),
    };
    match (operation, result) {
        (0, Err(VmError::ScriptFailure(failure))) => {
            let expected = before.read_execution(&[LENGTH as u64]).unwrap_err();
            assert_eq!(failure, expected);
            assert_eq!(failure.code, crate::VmFaultCode::Bounds);
        }
        (1 | 2, Err(VmError::InvalidArguments(message))) => {
            let expected = if operation == 1 {
                if fixture.place.variable == fixture.target {
                    "variable fill range is outside its storage"
                } else {
                    "array fill range or scalar type differs"
                }
            } else if fixture.place.variable != fixture.target {
                "array replacement length or scalar type differs"
            } else {
                "array replacement differs from its storage shape or type"
            };
            assert_eq!(message, expected);
        }
        (_, result) => panic!("unexpected write rejection: {result:?}"),
    }
    assert_eq!(fixture.cell(), &before);
    assert_eq!(fixture.cell().revision(), before.revision());
    fixture.assert_binding(&binding, &leases);
}
