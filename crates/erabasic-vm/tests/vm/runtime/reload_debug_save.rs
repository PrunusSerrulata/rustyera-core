use super::*;
#[test]
fn hot_reload_pins_old_stacks_and_migrates_compatible_state() {
    let (base, entry, variable) = call_artifact(1, vec![2]);
    let (target, _, _) = call_artifact(2, vec![3]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(7))
        .unwrap();
    vm.write_variable(variable, &[1], None, VmValue::Integer(8))
        .unwrap();
    let old = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1,
            maximum_host_calls: 0,
            fiber_quantum: 1,
        },
    );
    vm.prepare_hot_reload(
        &patch,
        &erabasic_compiler::runtime_native_validation_context(&target, &default_host_registry()),
    )
    .unwrap();
    vm.commit_hot_reload().unwrap();
    let new = vm.spawn_entry(entry, Vec::new()).unwrap();
    vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(matches!(
        vm.fiber_status(old),
        Some(FiberStatus::Completed(Some(VmValue::Integer(1))))
    ));
    assert!(matches!(
        vm.fiber_status(new),
        Some(FiberStatus::Completed(Some(VmValue::Integer(2))))
    ));
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(7)
    );
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(8)
    );
    assert_eq!(
        vm.read_variable(variable, &[2], None).unwrap(),
        VmValue::Integer(0)
    );
}

#[test]
fn debugger_pause_step_and_variable_batch_are_coherent_and_atomic() {
    let (artifact, entry, _) = call_artifact(7, vec![1]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let stop = vm.request_pause().unwrap();
    let frame = vm
        .call_stack(stop.token, fiber)
        .unwrap()
        .into_iter()
        .next()
        .expect("root frame");
    assert!(
        vm.operand_stack(stop.token, fiber, frame.id, None, 32)
            .unwrap()
            .values
            .is_empty()
    );
    let page = vm.variables(stop.token, None, 32).unwrap();
    let variable = page.values.first().expect("project variable").clone();
    let mut invalid_target = variable.target.clone();
    invalid_target.target.indices[0] = 99;
    assert!(
        vm.write_variables(
            stop.token,
            &[
                VmDebugVariableWrite {
                    target: variable.target.clone(),
                    value: VmValue::Integer(41),
                    expected_revision: variable.revision,
                },
                VmDebugVariableWrite {
                    target: invalid_target,
                    value: VmValue::Integer(42),
                    expected_revision: variable.revision,
                },
            ],
        )
        .is_err()
    );
    assert_eq!(
        VmDebugInspect::read_variable(&vm, stop.token, &variable.target)
            .unwrap()
            .value,
        VmValue::Integer(0)
    );

    vm.step(stop.token, fiber, VmStepKind::Instruction).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::DebugStopped(_))),
        "{:#?}",
        report.events
    );
}

#[test]
fn debugger_variable_pages_preserve_order_and_terminal_cursor() {
    let entry = SymbolKey::derive("test.function", b"debug-pages");
    let first = SymbolKey::derive("test.variable", b"first");
    let second = SymbolKey::derive("test.variable", b"second");
    let mut second_global = global(second, vec![1]);
    second_global.name = "SECOND".into();
    let artifact = artifact(
        vec![function(
            entry,
            "DEBUG_PAGES",
            vec![opcode::return_value(false)],
        )],
        vec![global(first, vec![1]), second_global],
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let stop = vm.request_pause().unwrap();

    let first_page = vm.variables(stop.token, None, 1).unwrap();
    assert_eq!(first_page.values.len(), 1);
    assert_eq!(first_page.values[0].target.target.variable, first);
    assert_eq!(first_page.next_cursor, Some(1));

    let second_page = vm.variables(stop.token, first_page.next_cursor, 1).unwrap();
    assert_eq!(second_page.values.len(), 1);
    assert_eq!(second_page.values[0].target.target.variable, second);
    assert_eq!(second_page.next_cursor, None);
}

#[test]
fn incompatible_hot_reload_is_rejected_atomically() {
    let (base, _, variable) = call_artifact(1, vec![2]);
    let mut target = base.clone();
    target.globals[0].value_type = BytecodeType::String;
    target.refresh_ids().unwrap();
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.write_variable(variable, &[0], None, VmValue::Integer(11))
        .unwrap();
    let original_id = vm.artifact_id();
    assert!(
        vm.prepare_hot_reload(
            &patch,
            &erabasic_compiler::runtime_native_validation_context(
                &target,
                &default_host_registry()
            )
        )
        .is_err()
    );
    assert!(vm.pending_hot_reload().is_none());
    assert_eq!(vm.artifact_id(), original_id);
    assert_eq!(
        vm.read_variable(variable, &[0], None).unwrap(),
        VmValue::Integer(11)
    );
}

#[test]
fn function_breakpoints_rebind_to_the_new_hot_reload_generation() {
    let (base, entry, _) = call_artifact(1, vec![1]);
    let (target, _, _) = call_artifact(2, vec![1]);
    let patch = create_patch(&base, &target);
    let mut vm = Vm::new(validated(&base), VmConfig::default());
    vm.update_breakpoints(
        &[VmBreakpoint {
            id: 9,
            enabled: true,
            hit_count: 0,
            location: VmBreakpointLocation::Function(entry),
        }],
        &[],
    )
    .unwrap();
    vm.prepare_hot_reload(
        &patch,
        &erabasic_compiler::runtime_native_validation_context(&target, &default_host_registry()),
    )
    .unwrap();
    vm.commit_hot_reload().unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = ReadyHost::default();
    let mut natives = NativeServiceRegistry::default();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(report.events.iter().any(|event| matches!(
        event,
        VmEvent::DebugStopped(stop)
            if matches!(stop.reason, erabasic_vm::VmDebugStopReason::Breakpoint(9))
    )));
}

#[test]
fn traditional_state_overlay_restores_persistent_arrays_without_stacks() {
    let entry = SymbolKey::derive("test.function", b"save");
    let variable = SymbolKey::derive("test.variable", b"save");
    let artifact = artifact(
        vec![function(entry, "SAVE", vec![opcode::return_value(false)])],
        vec![global(variable, vec![2])],
    );
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(variable, &[1], None, VmValue::Integer(42))
        .unwrap();
    let save = vm.export_era_state();
    vm.write_variable(variable, &[1], None, VmValue::Integer(0))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.reset_with_era_state(&save).unwrap();
    assert_eq!(report.restored_variables, 1);
    assert_eq!(vm.fiber_ids().count(), 0);
    assert_eq!(
        vm.read_variable(variable, &[1], None).unwrap(),
        VmValue::Integer(42)
    );
}

#[test]
fn traditional_state_restore_refreshes_calculated_character_count() {
    let artifact =
        compile_source("@SYSTEM_TITLE\nADDVOIDCHARA\nADDVOIDCHARA\nADDVOIDCHARA\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let charanum = artifact
        .globals
        .iter()
        .find(|global| global.name == "CHARANUM")
        .expect("CHARANUM")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { value: None, .. }))
    );

    let save = vm.export_era_state();
    let saved_character_count = i64::try_from(save.characters.len()).unwrap();
    vm.reset_with_era_state(&save).unwrap();
    assert_eq!(
        vm.read_variable(charanum, &[], None),
        Ok(VmValue::Integer(saved_character_count))
    );
}

#[test]
fn ordinary_save_excludes_and_restore_preserves_global_save_variables() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary");
    let global_key = SymbolKey::derive("test.variable", b"global");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_VALUE".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.write_variable(ordinary, &[0], None, VmValue::Integer(11))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(21))
        .unwrap();
    let save = vm.export_era_state();
    assert!(save.variables.contains_key(&ordinary));
    assert!(!save.variables.contains_key(&global_key));

    vm.write_variable(ordinary, &[0], None, VmValue::Integer(12))
        .unwrap();
    vm.write_variable(global_key, &[0], None, VmValue::Integer(22))
        .unwrap();
    vm.reset_with_era_state(&save).unwrap();
    assert_eq!(
        vm.read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(11))
    );
    assert_eq!(
        vm.read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(22))
    );
}

#[test]
fn global_overlay_transaction_changes_only_global_save_storage() {
    let ordinary = SymbolKey::derive("test.variable", b"ordinary-overlay");
    let global_key = SymbolKey::derive("test.variable", b"global-overlay");
    let mut ordinary_definition = global(ordinary, vec![1]);
    ordinary_definition.name = "ORDINARY_OVERLAY".into();
    let mut global_definition = global(global_key, vec![1]);
    global_definition.name = "GLOBAL_OVERLAY".into();
    global_definition.persistence = BytecodePersistence::GlobalSave;
    let artifact = artifact(Vec::new(), vec![ordinary_definition, global_definition]);
    let mut vm = RuntimeVm::new(validated(&artifact), VmConfig::default());
    vm.vm_mut()
        .write_variable(ordinary, &[0], None, VmValue::Integer(10))
        .unwrap();
    let mut state = vm.vm().export_era_state_for(EraSaveScope::Global);
    state.variables.get_mut(&global_key).unwrap().values[0] = VmValue::Integer(20);
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::OverlayGlobal(Box::new(state)))
        .unwrap();
    vm.commit_runtime_state(prepared).unwrap();
    assert_eq!(
        vm.vm().read_variable(ordinary, &[0], None),
        Ok(VmValue::Integer(10))
    );
    assert_eq!(
        vm.vm().read_variable(global_key, &[0], None),
        Ok(VmValue::Integer(20))
    );
}

#[test]
fn isolated_fork_copies_memory_without_copying_live_execution() {
    let key = SymbolKey::derive("test.variable", b"candidate");
    let artifact = artifact(Vec::new(), vec![global(key, vec![1])]);
    let mut live = RuntimeVm::new(validated(&artifact), VmConfig::default());
    live.vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(7))
        .unwrap();

    let mut candidate = live.fork_isolated().unwrap();
    assert_eq!(
        candidate.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    candidate
        .vm_mut()
        .write_variable(key, &[0], None, VmValue::Integer(9))
        .unwrap();

    assert_eq!(
        live.vm().read_variable(key, &[0], None),
        Ok(VmValue::Integer(7))
    );
    assert!(!candidate.has_runnable_fibers());
}

#[test]
fn compiled_arithmetic_executes_and_updates_project_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = (2 + 3) * 4\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(20));
}

#[test]
fn compiled_assignment_matches_reference_smoke_input() {
    // The macOS/Windows reference smoke suite executes the exact `RESULT = 9`
    // statement and observes RESULT=9 through the C# VM watch projection.
    let artifact = compile_source("@SYSTEM_TITLE\nRESULT = 9\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(9));
}

#[test]
fn dynamic_try_resolves_before_arguments_and_form_call_invokes_target() {
    let artifact = compile_source(
        "@ORACLE_COMPAT\nRESULT = 0\nTRYCALLFORM ORACLE_MISSING(1 / LOCAL)\nCALLFORM ORACLE_DYNAMIC_{1}(4)\nRETURN RESULT\n@ORACLE_DYNAMIC_1(ARG)\nFLAG:0 = ARG\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "ORACLE_COMPAT")
        .expect("entry")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(vm.read_variable(flag, &[0], None), Ok(VmValue::Integer(4)));
}

#[test]
fn formatted_try_call_resolves_a_unicode_function_before_catch() {
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\n\
         #DIM REQUEST_ID\n\
         REQUEST_ID = 2005\n\
         RESULT = 0\n\
         RESULTS:0 = IRAI_一般{REQUEST_ID % 1000}\n\
         TRYCCALLFORM IRAI_一般{REQUEST_ID % 1000}(2, REQUEST_ID, \"依頼実行時\")\n\
         CATCH\n\
         FLAG:0 = -1\n\
         ENDCATCH\n\
         RETURN RESULT\n\
         @IRAI_一般5(CHARA, IRAI_ID, SCENE)\n\
         #DIM CHARA\n\
         #DIM IRAI_ID\n\
         #DIMS SCENE\n\
         FLAG:0 = CHARA + IRAI_ID + (SCENE == \"依頼実行時\")\n\
         RETURN RESULT\n",
        &AnalyzerOptions::default(),
    );
    assert!(
        artifact
            .functions
            .iter()
            .any(|function| function.name == "IRAI_一般5"),
        "dynamic target was omitted from the compiled artifact"
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .expect("FLAG")
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .expect("RESULTS")
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("IRAI_一般5".into()))
    );
    assert_eq!(
        vm.read_variable(flag, &[0], None),
        Ok(VmValue::Integer(2_008))
    );
}
