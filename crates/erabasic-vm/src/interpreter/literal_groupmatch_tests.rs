use super::*;
use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_compiler::{CompilerOptions, compile_project, default_host_registry};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_validator::{ValidationContext, validate_bytecode};

struct NoHost;
impl VmHost for NoHost {
    fn call(&mut self, _: crate::HostCallRequest) -> crate::HostCallResult {
        panic!("literal match must not call Host");
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "One real cursor fixture compares every fast-path barrier before executing the bulk step."
)]
fn literal_groupmatch_cursor_executes_bulk_and_preserves_all_fallback_barriers() {
    for (declaration, literals, needle) in [
        ("#DIM VALUE", "7, 0, 7", VmValue::Integer(7)),
        ("#DIM VALUE", "7, 0, 7", VmValue::Integer(-7)),
        (
            "#DIMS VALUE",
            "\"same\", \"other\", \"same\"",
            VmValue::String("same".into()),
        ),
    ] {
        let source = format!(
            "@SYSTEM_TITLE\n{declaration}\nRESULT = GROUPMATCH(VALUE, {literals})\nRETURN\n"
        );
        let mut artifact = compile_cursor_fixture(source);
        if needle == VmValue::Integer(-7) {
            // Exercise a valid directly encoded negative literal, rather than Push + Unary.
            for encoded in artifact
                .functions
                .iter_mut()
                .flat_map(|function| &mut function.code)
            {
                if encoded.opcode == Opcode::PushInteger as u16
                    && encoded.payload.as_ref() == 7_i64.to_le_bytes()
                {
                    encoded.payload = (-7_i64).to_le_bytes().to_vec().into();
                }
            }
            artifact.refresh_ids().unwrap();
        }
        let entry = artifact.functions[0].key;
        let validation = validate_bytecode(
            artifact.clone().into_unvalidated(),
            &ValidationContext::for_artifact(&artifact),
        );
        assert!(
            validation.diagnostics.is_empty(),
            "{:#?}",
            validation.diagnostics
        );
        let validated = validation.value.unwrap();
        let mut vm = Vm::new(validated, crate::VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact(&artifact);
        let id = vm.spawn_entry(entry, Vec::new()).unwrap();
        let mut fiber = vm.fibers.remove(&id).unwrap();
        let mut cursor = None;
        for instruction in 0..artifact.functions[0].code.len() {
            let position = vm
                .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
                .unwrap();
            assert_eq!(
                position.variable,
                position
                    .resolved_program
                    .unwrap()
                    .0
                    .instruction_global(0, instruction)
            );
        }
        let instruction = (0..artifact.functions[0].code.len())
            .find(|instruction| {
                vm.instruction_position_at(vm.current_generation, entry, *instruction, &mut cursor)
                    .unwrap()
                    .literal_group_match
                    .is_some()
            })
            .expect("actual cursor-resolved literal plan");
        let position = vm
            .instruction_position_at(vm.current_generation, entry, instruction, &mut cursor)
            .unwrap();
        fiber.frames.last_mut().unwrap().instruction = instruction;
        fiber.frames.last_mut().unwrap().stack = vec![needle];
        let policy = ExecutionPolicy {
            allow_function_memo: true,
            allow_immediate_host: false,
            remaining_quantum: 4,
            remaining_instructions: 4,
        };
        for blocked in [
            ExecutionPolicy {
                allow_function_memo: false,
                ..policy
            },
            ExecutionPolicy {
                remaining_quantum: 3,
                ..policy
            },
            ExecutionPolicy {
                remaining_instructions: 3,
                ..policy
            },
        ] {
            let before = fiber.frames.last().unwrap().stack.clone();
            assert_eq!(
                vm.try_literal_group_match(&mut fiber, &position, blocked),
                None
            );
            assert_eq!(fiber.frames.last().unwrap().instruction, instruction);
            assert_eq!(fiber.frames.last().unwrap().stack, before);
        }
        vm.config.maximum_operand_stack = 3;
        let before = fiber.frames.last().unwrap().stack.clone();
        assert_eq!(
            vm.try_literal_group_match(&mut fiber, &position, policy),
            None
        );
        assert_eq!(fiber.frames.last().unwrap().instruction, instruction);
        assert_eq!(fiber.frames.last().unwrap().stack, before);
        vm.config.maximum_operand_stack = 4;
        let result = vm
            .execute_instruction(
                &mut fiber,
                &position,
                &mut NoHost,
                &mut natives,
                &mut 0,
                policy,
            )
            .unwrap();
        assert!(matches!(result, StepOutcome::BulkProgress(3)));
        assert_eq!(fiber.frames.last().unwrap().instruction, instruction + 4);
        assert_eq!(fiber.frames.last().unwrap().stack, [VmValue::Integer(2)]);
    }
}

#[test]
fn instruction_cursor_refresh_preserves_failure_order_and_committed_switches() {
    let artifact = compile_cursor_fixture("@SYSTEM_TITLE\nRETURN\n@OTHER\nRETURN\n".into());
    let entry = artifact.functions[0].key;
    let other = artifact.functions[1].key;
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    let mut vm = Vm::new(validation.value.unwrap(), crate::VmConfig::default());
    let first = vm.current_generation;
    let second = crate::GenerationId(first.0 + 1);
    let absent = crate::GenerationId(second.0 + 1);
    let second_program = Arc::new(ProgramGeneration::new(Arc::new(artifact)));
    vm.generations.insert(second, second_program);
    let mut cursor = None;
    vm.instruction_position_at(first, entry, 0, &mut cursor)
        .unwrap();
    let missing = SymbolKey::derive("cursor-test", b"missing-function");
    for (generation, function, error) in [
        (first, missing, VmError::MissingFunction(missing)),
        (second, missing, VmError::MissingFunction(missing)),
        (
            absent,
            entry,
            VmError::InvalidState("frame generation was reclaimed".into()),
        ),
        (
            absent,
            missing,
            VmError::InvalidState("frame generation was reclaimed".into()),
        ),
    ] {
        let before = cursor.clone().unwrap();
        assert_eq!(
            vm.instruction_position_at(generation, function, 0, &mut cursor)
                .err(),
            Some(error)
        );
        assert_cursor_matches(cursor.as_ref().unwrap(), &before);
    }
    for (generation, function, index) in [(first, other, 1), (second, entry, 0)] {
        assert_eq!(
            vm.instruction_position_at(generation, function, usize::MAX, &mut cursor)
                .err(),
            Some(VmError::InvalidState(
                "instruction pointer left its function".into()
            ))
        );
        let expected = FunctionCursor {
            generation,
            function,
            index,
            program: Arc::clone(vm.generations.get(&generation).unwrap()),
        };
        assert_cursor_matches(cursor.as_ref().unwrap(), &expected);
        vm.instruction_position_at(generation, function, 0, &mut cursor)
            .unwrap();
        assert_cursor_matches(cursor.as_ref().unwrap(), &expected);
    }
}

fn assert_cursor_matches(actual: &FunctionCursor, expected: &FunctionCursor) {
    assert_eq!(actual.generation, expected.generation);
    assert_eq!(actual.function, expected.function);
    assert_eq!(actual.index, expected.index);
    assert!(Arc::ptr_eq(&actual.program, &expected.program));
}

fn compile_cursor_fixture(source: String) -> erabasic_bytecode::BytecodeArtifact {
    let analysis = analyze_project(
        AnalysisInput {
            project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
                .data
                .unwrap(),
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source),
            }],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    compile_project(
        analysis.project.as_ref().unwrap(),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
    )
    .artifact
    .unwrap()
}
