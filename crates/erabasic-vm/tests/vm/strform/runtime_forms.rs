use super::*;
#[test]
fn runtime_form_covers_triples_escapes_interpolation_conditionals_and_calls() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
ADDVOIDCHARA
NAME:0 = Target
CALLNAME:0 = Call
TARGET = 0
MASTER = 0
PLAYER = 0
ASSI = 0
#DIM DYNAMIC VALUE
#DIMS DYNAMIC TEXT
VALUE = 7
TEXT '= "text"
RESULT:9 = TOINT("7")
RESULTS:0 '= STRFORM("A\\s{VALUE,3,LEFT}|%TEXT%|\\@ VALUE == 7 ? yes # no \\@|***|+++|===|///|$$$|{TOINT(\"7\")}|%WRAP()%|\\%")
RETURN
@WRAP
#FUNCTIONS
RETURNF "[user]"
"#,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{:#?}",
        report.events
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String(
            "A 7  |text|yes|Target|Call|Call|Target|Call|7|[user]|%".into()
        ))
    );
}

#[test]
fn runtime_form_uses_the_project_ignore_triple_symbols_compatibility() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.ignore_triple_symbols = true;
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
RESULT = 5
RESULTS:0 '= STRFORM("***\\s{RESULT}")
RETURN
"#,
        &options,
    );
    assert!(artifact.call_compatibility.ignore_triple_symbols);
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("*** 5".into()))
    );
}

#[test]
fn runtime_form_resolves_dynamic_locals_and_statics_before_the_global() {
    let artifact = compile_with_header(
        "#DIMS VALUE\n",
        r#"@SYSTEM_TITLE
VALUE '= "global"
RESULTS:0 '= STRFORM("%VALUE%")
RESULTS:1 '= STRFORM("%READ_LOCAL()%")
RESULTS:2 '= STRFORM("%READ_STATIC()%")
RETURN
@READ_LOCAL
#FUNCTIONS
#DIMS DYNAMIC VALUE
VALUE '= "local"
RETURNF STRFORM("%VALUE%")
@READ_STATIC
#FUNCTIONS
#DIMS STATIC VALUE
VALUE '= "static"
RETURNF STRFORM("%VALUE%")
"#,
        &AnalyzerOptions::analysis_mode(),
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    for (index, expected) in [(0, "global"), (1, "local"), (2, "static")] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn runtime_form_named_indices_prefer_variables_then_functions_then_csv_keys() {
    let mut data = project_data();
    let flag = data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Flag)
        .expect("FLAG name table");
    flag.lookup.insert("INDEX_FUNCTION".into(), 7);
    flag.lookup.insert("INDEX_VARIABLE".into(), 8);
    flag.lookup.insert("INDEX_KEY".into(), 7);
    let artifact = compile_source_with_data(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC INDEX_VARIABLE
INDEX_VARIABLE = 10
FLAG:7 = 70
FLAG:8 = 80
FLAG:9 = 90
FLAG:10 = 100
RESULTS:0 '= STRFORM("{FLAG:INDEX_VARIABLE}")
RESULTS:1 '= STRFORM("{FLAG:INDEX_FUNCTION}")
RESULTS:2 '= STRFORM("{FLAG:INDEX_KEY}")
RETURN
@INDEX_FUNCTION
#FUNCTION
RETURNF 9
"#,
        data,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    for (index, expected) in [(0, "100"), (1, "90"), (2, "70")] {
        assert_eq!(
            vm.read_variable(results, &[index], None),
            Ok(VmValue::String(expected.into()))
        );
    }
}

#[test]
fn runtime_form_resolves_character_data_names_after_the_character_index() {
    let mut data = project_data();
    data.static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Palam)
        .expect("PALAM name table")
        .lookup
        .insert("快Ｃ".into(), 17);
    let artifact = compile_source_with_data(
        r#"@SYSTEM_TITLE
ADDVOIDCHARA
CUP:0:17 = 9
RESULTS:0 '= STRFORM("{CUP:0:快Ｃ}")
RETURN
"#,
        data,
    );
    let results = named_key(&artifact, "RESULTS");
    let (vm, report) = run_entry(&artifact, VmConfig::default());

    assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("9".into()))
    );
}

#[test]
fn runtime_form_rejects_private_column_import_before_side_effects() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
DT_CREATE "t"
DT_COLUMN_ADD "t", "value", "int64", 1
DT_COLUMN_OPTIONS "t", "value", DEFAULT, 9
RESULT:10 = 0
RESULTS:0 '= STRFORM("{BUMP()}%DT__COLUMN_RESOLVE(\"value\", \"t\")%")
RETURN
@BUMP
#FUNCTION
RESULT:10 += 1
RETURNF 1
"#,
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(
        fault.message.contains("internal column operation"),
        "{fault:?}"
    );
    assert_eq!(
        vm.read_variable(named_key(&artifact, "RESULT"), &[10], None),
        Ok(VmValue::Integer(0))
    );
}

#[test]
fn runtime_form_keeps_user_methods_named_like_private_column_imports() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%DT__COLUMN_RESOLVE()%")
RETURN
@DT__COLUMN_RESOLVE
#FUNCTIONS
RETURNF "user-method"
"#,
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_eq!(
        vm.read_variable(named_key(&artifact, "RESULTS"), &[0], None),
        Ok(VmValue::String("user-method".into()))
    );
}

#[test]
fn runtime_form_rejects_non_lvalue_mutation_before_any_user_side_effect() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULT = 0
RESULTS:0 '= STRFORM("{BUMP()}|{(RESULT + 1)++}")
RETURN
@BUMP
#FUNCTION
RESULT += 1
RETURNF 1
"#,
    );
    let result = named_key(&artifact, "RESULT");
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    let fault = take_fault(report);
    assert!(matches!(
        fault.category,
        erabasic_vm::FaultCategory::Script(
            erabasic_vm::ScriptFaultKind::Parse | erabasic_vm::ScriptFaultKind::Argument
        )
    ));
    assert_eq!(
        vm.read_variable(result, &[], None),
        Ok(VmValue::Integer(0)),
        "preflight must reject before BUMP executes"
    );
}

#[test]
fn runtime_form_survives_tiny_budgets_and_executes_user_side_effects_once() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULT:1 = 0
RESULTS:0 '= STRFORM("{BUMP()}")
RETURN
@BUMP
#FUNCTION
RESULT:1 += 1
RETURNF RESULT:1
"#,
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let result = named_key(&artifact, "RESULT");
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let budget = RunBudget {
        maximum_instructions: 1,
        maximum_host_calls: 1,
        fiber_quantum: 1,
    };
    let mut exhausted = 0;
    let mut completed = false;
    for _ in 0..100 {
        let report = vm.run_slice(&mut ReadyHost::default(), &mut natives, budget);
        if report.stop == erabasic_vm::VmRunStop::BudgetExhausted {
            exhausted += 1;
        }
        completed |= report.events.iter().any(
            |event| matches!(event, VmEvent::FiberCompleted { fiber: done, .. } if *done == fiber),
        );
        if report.stop == erabasic_vm::VmRunStop::Idle {
            break;
        }
    }
    assert!(exhausted > 1, "the continuation must cross multiple slices");
    assert!(completed);
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("1".into()))
    );
}

#[test]
fn runtime_form_reports_malformed_missing_type_bounds_and_resource_errors() {
    for (template, expected) in [
        ("value={RESULT", VmFaultCode::Native),
        ("{DOES_NOT_EXIST}", VmFaultCode::MissingSymbol),
        ("{RESULTS}", VmFaultCode::TypeMismatch),
        ("{RESULT:999999}", VmFaultCode::Bounds),
    ] {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"{template}\")\nRETURN\n"
        ));
        let (_, report) = run_entry(&artifact, VmConfig::default());
        assert_eq!(take_fault(report).code, expected, "{template:?}");
    }

    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULTS:0 '= STRFORM(\"a{RESULT}b{RESULT}c{RESULT}\")\nRETURN\n",
    );
    let (_, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_operand_stack: 4,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
}

#[test]
fn recursive_runtime_forms_stop_at_the_vm_call_depth_without_rust_recursion() {
    let artifact = compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%RECURSE(0)%")
RETURN
@RECURSE(ARG)
#FUNCTIONS
RETURNF STRFORM("%RECURSE(ARG + 1)%")
"#,
    );
    let (_, report) = run_entry(
        &artifact,
        VmConfig {
            maximum_call_depth: 8,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
}

struct RuntimeFormHost {
    calls: usize,
    pending: bool,
    value: String,
}

impl VmHost for RuntimeFormHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        assert!(request.import.name.eq_ignore_ascii_case("LOADTEXT"));
        self.calls += 1;
        if self.pending {
            HostCallResult::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: b"runtime-form-loadtext".to_vec(),
            }
        } else {
            HostCallResult::Ready(HostReady {
                value: Some(VmValue::String(self.value.clone())),
                writes: Vec::new(),
            })
        }
    }
}

fn host_runtime_form_artifact() -> BytecodeArtifact {
    compile_source(
        r#"@SYSTEM_TITLE
RESULTS:0 '= STRFORM("%READ_HOST()%")
RETURN
@READ_HOST
#FUNCTIONS
RETURNF LOADTEXT("fixture.txt")
"#,
    )
}

#[test]
fn runtime_form_obeys_host_caps_and_resumes_after_a_suspended_user_function() {
    let artifact = host_runtime_form_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = RuntimeFormHost {
        calls: 0,
        pending: true,
        value: "unused".into(),
    };

    let capped = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1_000,
            maximum_host_calls: 0,
            fiber_quantum: 1,
        },
    );
    assert_eq!(capped.stop, erabasic_vm::VmRunStop::BudgetExhausted);
    assert_eq!(host.calls, 0);

    let suspended = vm.run_slice(
        &mut host,
        &mut natives,
        RunBudget {
            maximum_instructions: 1_000,
            maximum_host_calls: 1,
            fiber_quantum: 1,
        },
    );
    let request = suspended
        .events
        .iter()
        .find_map(|event| match event {
            VmEvent::HostPending {
                fiber: waiting,
                request,
            } if *waiting == fiber => Some(*request),
            _ => None,
        })
        .expect("runtime FORM user function must suspend at LOADTEXT");
    assert_eq!(host.calls, 1);
    vm.resume_host(
        request,
        HostReady {
            value: Some(VmValue::String("resumed".into())),
            writes: Vec::new(),
        },
    )
    .unwrap();
    let resumed = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    completed_without_fault(&resumed, fiber);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("resumed".into()))
    );
}

#[test]
fn era_restart_discards_a_suspended_runtime_form_and_starts_cleanly() {
    let artifact = host_runtime_form_artifact();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let results = named_key(&artifact, "RESULTS");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let old = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut host = RuntimeFormHost {
        calls: 0,
        pending: true,
        value: "restarted".into(),
    };
    let suspended = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    assert!(
        suspended
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::HostPending { fiber, .. } if *fiber == old))
    );

    let state = vm.export_era_state();
    vm.reset_with_era_state(&state).unwrap();
    assert!(vm.fiber_status(old).is_none());
    assert_eq!(vm.fiber_ids().count(), 0);

    host.pending = false;
    let restarted = vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
    completed_without_fault(&report, restarted);
    assert_eq!(
        vm.read_variable(results, &[0], None),
        Ok(VmValue::String("restarted".into()))
    );
}

#[test]
fn instruction_step_treats_context_free_runtime_form_work_as_one_native_call() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULTS '= STRFORM(\"a{1 + 2}b\")\nRETURN\n");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let strform_instruction = artifact
        .functions
        .iter()
        .find(|function| function.key == entry)
        .and_then(|function| {
            function.code.iter().position(|instruction| {
                Opcode::try_from(instruction.opcode) == Ok(Opcode::CallNative)
            })
        })
        .expect("compiled STRFORM call");
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let fiber = vm.spawn_entry(entry, Vec::new()).unwrap();
    let mut expanded = vm.request_pause().unwrap();
    for instruction in 0..=strform_instruction {
        vm.step(expanded.token, fiber, VmStepKind::Instruction)
            .unwrap();
        let report = vm.run_slice(
            &mut ReadyHost::default(),
            &mut natives,
            RunBudget::default(),
        );
        if instruction == strform_instruction {
            assert!(
                report.instructions > 1,
                "continuation work must complete behind the STRFORM after-hook"
            );
        } else {
            assert_eq!(report.instructions, 1);
        }
        expanded = report
            .events
            .iter()
            .find_map(|event| match event {
                VmEvent::DebugStopped(stop) => Some(stop.clone()),
                _ => None,
            })
            .expect("instruction debug stop");
    }
    let frame = vm
        .call_stack(expanded.token, fiber)
        .unwrap()
        .into_iter()
        .next()
        .expect("SYSTEM_TITLE frame");
    let operands = vm
        .operand_stack(expanded.token, fiber, frame.id, None, 16)
        .unwrap();
    assert!(
        operands
            .values
            .iter()
            .any(|operand| operand.value == VmValue::String("a3b".into())),
        "{operands:#?}"
    );
}
