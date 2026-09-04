use super::*;
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the case table beside the shared dual-profile and optimization assertions."
)]
fn dynamic_methods_execute_lazily_with_defaults_ref_and_optimization_parity() {
    for snake in [false, true] {
        for optimization in [
            erabasic_compiler::OptimizationLevel::None,
            erabasic_compiler::OptimizationLevel::Basic,
        ] {
            let artifact = compile_with_header_and_compiler(
                METHOD_FIXTURE_HEADER,
                METHOD_FIXTURE_SOURCE,
                &method_options(snake),
                &CompilerOptions {
                    optimization,
                    ..CompilerOptions::default()
                },
            );
            for (case, result, trace, bodies) in [
                ("INTEGER", 42, 0, 1),
                ("PRESENT_SKIPS_FALLBACK", 23, 1234, 1),
                ("MISSING_ONLY_FALLBACK", 90, 19, 0),
                ("EXPLICIT_OMITTED_SLOT", 57, 0, 1),
                ("TRAILING_DEFAULTS", 56, 0, 1),
                ("I64_MIN_IS_VALUE", i64::MIN, 0, 1),
                ("WHOLE_ARRAY_REF_WRITEBACK", 11, 0, 1),
                ("WHOLE_ARRAY_REF_SKIPS_INDEX", 11, 0, 1),
                ("FINITE_RECURSION", 4, 0, 4),
                ("VALUE_CAPTURED_BEFORE_NEXT_ARGUMENT", 102, 4, 1),
                ("CAN_MOVE_DYNAMIC_PATTERN", 3, 0, 1),
                ("ODEKAKEMAP_DYNAMIC_PATTERN", 1, 0, 1),
                ("EVENT_INVISIBLE", 90, 19, 0),
                ("BUILTIN_INVISIBLE", 90, 19, 0),
                ("INTEGER_STATEMENT", 42, 0, 1),
            ] {
                let (vm, report) = run_method_case(
                    &artifact,
                    &format!("METHOD_CASE_{case}"),
                    VmConfig::default(),
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                    "{case}: {report:?}"
                );
                assert_eq!(report.stop, erabasic_vm::VmRunStop::Idle, "{case}");
                for (name, expected) in [
                    ("RESULT", result),
                    ("METHOD_TRACE", trace),
                    ("METHOD_BODY_COUNT", bodies),
                    ("METHOD_INDEX_COUNT", 0),
                ] {
                    assert_eq!(
                        vm.read_variable(named_key(&artifact, name), &[0], None),
                        Ok(VmValue::Integer(expected)),
                        "{case}: {name}"
                    );
                }
                if case.starts_with("WHOLE_ARRAY_REF") {
                    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(11));
                    assert_method_watch(
                        &vm,
                        &artifact,
                        "METHOD_WORDS",
                        1,
                        VmValue::String("changed".into()),
                    );
                }
                if case == "VALUE_CAPTURED_BEFORE_NEXT_ARGUMENT" {
                    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(99));
                }
            }
            for (case, result, trace, bodies) in [
                ("STRING", "zero", 0, 1),
                ("STRING_PRESENT_SKIPS_FALLBACK", "text:2", 124, 1),
                ("STRING_MISSING_ONLY_FALLBACK", "fallback", 19, 0),
                ("STRING_DYNAMIC_PATTERN", "pair:10:11", 0, 1),
                ("FORMATTED_EXPRESSION", "value=42", 1, 1),
                ("STRFORM_RUNTIME_EXPRESSION", "value=42", 0, 1),
                ("STRING_STATEMENT", "zero", 0, 1),
            ] {
                let (vm, report) = run_method_case(
                    &artifact,
                    &format!("METHOD_CASE_{case}"),
                    VmConfig::default(),
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                    "{case}: {report:?}"
                );
                assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String(result.into()));
                assert_method_watch(&vm, &artifact, "METHOD_TRACE", 0, VmValue::Integer(trace));
                assert_method_watch(
                    &vm,
                    &artifact,
                    "METHOD_BODY_COUNT",
                    0,
                    VmValue::Integer(bodies),
                );
            }
            let (vm, report) = run_method_case(
                &artifact,
                "METHOD_CASE_EXIST_ZERO_ARGUMENT_RESOLUTION",
                VmConfig::default(),
            );
            assert!(
                !report
                    .events
                    .iter()
                    .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
                "{report:?}"
            );
            for (index, value) in [1, 2, 0, 0, 0, 1, 0, 0, 0].into_iter().enumerate() {
                assert_method_watch(
                    &vm,
                    &artifact,
                    "RESULT",
                    u64::try_from(index).unwrap(),
                    VmValue::Integer(value),
                );
            }
            assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(0));
        }
    }
}

#[test]
fn dynamic_method_signature_faults_preserve_only_name_evaluation_side_effects() {
    for snake in [false, true] {
        let artifact = compile_with_header(
            METHOD_FIXTURE_HEADER,
            METHOD_FIXTURE_SOURCE,
            &method_options(snake),
        );
        for (case, code) in [
            ("MISSING_NO_FALLBACK", VmFaultCode::MissingSymbol),
            ("MISSING_OMITTED_FALLBACK", VmFaultCode::MissingSymbol),
            ("ORDINARY_FUNCTION", VmFaultCode::TypeMismatch),
            ("WRONG_INTEGER_RETURN", VmFaultCode::TypeMismatch),
            ("WRONG_STRING_RETURN", VmFaultCode::TypeMismatch),
            ("WRONG_ARGUMENT_TYPE", VmFaultCode::TypeMismatch),
            ("MISSING_REQUIRED_ARGUMENT", VmFaultCode::TypeMismatch),
            ("MISSING_REF", VmFaultCode::TypeMismatch),
            ("EXPRESSION_NOT_REF", VmFaultCode::TypeMismatch),
            ("WRONG_REF_TYPE", VmFaultCode::TypeMismatch),
            ("WRONG_REF_RANK", VmFaultCode::TypeMismatch),
            ("EXTRA_ARGUMENT_POLICY", VmFaultCode::TypeMismatch),
        ] {
            let (vm, report) = run_method_case(
                &artifact,
                &format!("METHOD_CASE_{case}"),
                VmConfig::default(),
            );
            if snake && case == "EXTRA_ARGUMENT_POLICY" {
                assert!(
                    report
                        .events
                        .iter()
                        .any(|event| { matches!(event, VmEvent::FiberCompleted { .. }) }),
                    "{report:?}"
                );
                assert!(
                    !report
                        .events
                        .iter()
                        .any(|event| { matches!(event, VmEvent::FiberFaulted { .. }) }),
                    "{report:?}"
                );
                assert_eq!(report.events.iter().filter(|event| {
                    matches!(event, VmEvent::Diagnostic { code, .. } if code == "compat.call.excess_arguments")
                }).count(), 1);
                for (name, expected) in [
                    ("RESULT", 2),
                    ("METHOD_TRACE", 12),
                    ("METHOD_BODY_COUNT", 1),
                    ("METHOD_INDEX_COUNT", 0),
                    ("METHOD_EVENT_COUNT", 0),
                ] {
                    assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
                }
                continue;
            }
            assert_eq!(take_fault(report).code, code, "{case}");
            // Faulted runtime-tester sessions cannot expose debug watches: inspect VM storage directly.
            for (name, expected) in [
                ("METHOD_TRACE", 1),
                ("METHOD_BODY_COUNT", 0),
                ("METHOD_INDEX_COUNT", 0),
                ("METHOD_EVENT_COUNT", 0),
            ] {
                assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
            }
            assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(10));
            assert_method_watch(
                &vm,
                &artifact,
                "METHOD_WORDS",
                1,
                VmValue::String("unchanged".into()),
            );
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Keep the typed expression matrix beside its shared side-effect assertions."
)]
fn runtime_form_dynamic_methods_share_lazy_resolution_and_capture_semantics() {
    let expressions = [
        (
            r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), METHOD_ARG(3))"#,
            "23",
            1234,
            1,
        ),
        (
            r#"GETMETH(METHOD_NAME("MISSING"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
            "90",
            19,
            0,
        ),
        (r#"GETMETH("METHOD_DEFAULT", , , 7)"#, "57", 0, 1),
        (
            r#"GETMETH("METHOD_ECHO", , (-9223372036854775807 - 1))"#,
            "-9223372036854775808",
            0,
            1,
        ),
        (
            r#"GETMETH("METHOD_REF", , METHOD_VALUES:METHOD_INDEX(), METHOD_WORDS:METHOD_INDEX())"#,
            "11",
            0,
            1,
        ),
        (
            r#"GETMETH("METHOD_PAIR", , METHOD_VALUES:0, METHOD_MUTATE_VALUE())"#,
            "102",
            4,
            1,
        ),
        (
            r#"GETMETH("METHOD_PAIR", , GETMETH("METHOD_ECHO", , 2), 3)"#,
            "23",
            4,
            2,
        ),
        (r#"EXISTMETH("METHOD_DEFAULT")"#, "1", 0, 0),
        (
            r#"GETMETH("METHOD_PAIR", , "a" < "b", (1 ? 2 # 3))"#,
            "12",
            4,
            1,
        ),
        (r#"GETMETHS("FORM_STRING_ECHO", , "a" + "b")"#, "ab", 0, 1),
        (r#"GETMETHS("FORM_STRING_ECHO", , "a" * 2)"#, "aa", 0, 1),
        (r#"GETMETHS("FORM_STRING_ECHO", , 2 * "b")"#, "bb", 0, 1),
        (
            r#"GETMETHS("FORM_STRING_ECHO", , (0 ? "a" # "b"))"#,
            "b",
            0,
            1,
        ),
    ];
    let mut source = METHOD_FIXTURE_SOURCE.to_owned()
        + "\n@FORM_STRING_ECHO(TEXT)\n#FUNCTIONS\n#DIMS DYNAMIC TEXT\nMETHOD_BODY_COUNT += 1\nRETURNF TEXT\n";
    for (index, (expression, _, _, _)) in expressions.iter().enumerate() {
        let escaped = expression.replace('"', "\\\"");
        let form = if expression.starts_with("GETMETHS") {
            format!("%{escaped}%")
        } else {
            format!("{{{escaped}}}")
        };
        write!(
            source,
            "\n@FORM_METHOD_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{form}\")\nRETURN\n"
        )
        .unwrap();
    }
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    for (index, (_, output, trace, bodies)) in expressions.iter().enumerate() {
        let (vm, report) = run_method_case(
            &artifact,
            &format!("FORM_METHOD_{index}"),
            VmConfig::default(),
        );
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{index}: {report:?}"
        );
        assert_method_watch(
            &vm,
            &artifact,
            "RESULTS",
            0,
            VmValue::String((*output).into()),
        );
        assert_method_watch(&vm, &artifact, "METHOD_TRACE", 0, VmValue::Integer(*trace));
        assert_method_watch(
            &vm,
            &artifact,
            "METHOD_BODY_COUNT",
            0,
            VmValue::Integer(*bodies),
        );
        assert_method_watch(&vm, &artifact, "METHOD_INDEX_COUNT", 0, VmValue::Integer(0));
    }
}

#[test]
fn runtime_form_method_faults_do_not_evaluate_fallback_actuals_or_ref_indices() {
    let expressions = [
        r#"GETMETH(METHOD_NAME("METHOD_ORDINARY"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_TEXT"), METHOD_FALLBACK(), METHOD_ARG(2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_ECHO"), METHOD_FALLBACK(), METHOD_STRING_ARG())"#,
        r#"GETMETH(METHOD_NAME("METHOD_REQUIRED"), METHOD_FALLBACK())"#,
        r#"GETMETH(METHOD_NAME("METHOD_REF_INT"), METHOD_FALLBACK(), METHOD_MATRIX:METHOD_INDEX():0)"#,
        r#"GETMETH(METHOD_NAME("METHOD_REF_INT"), METHOD_FALLBACK(), METHOD_WORDS:METHOD_INDEX())"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), "x" - 1)"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), -"x")"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), ("x" ? 1 # 2))"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), "x" == 1)"#,
        r#"GETMETH(METHOD_NAME("METHOD_PAIR"), METHOD_FALLBACK(), METHOD_ARG(2), METHOD_ECHO("x" - 1))"#,
    ];
    let mut source = METHOD_FIXTURE_SOURCE.to_owned();
    for (index, expression) in expressions.iter().enumerate() {
        let escaped = expression.replace('"', "\\\"");
        write!(source, "\n@FORM_FAULT_{index}\nCALL METHOD_RESET\nRESULTS:0 '= STRFORM(\"{{{escaped}}}\")\nRETURN\n").unwrap();
    }
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    for index in 0..expressions.len() {
        let (vm, report) = run_method_case(
            &artifact,
            &format!("FORM_FAULT_{index}"),
            VmConfig::default(),
        );
        assert_eq!(
            take_fault(report).code,
            VmFaultCode::TypeMismatch,
            "{index}"
        );
        for (name, expected) in [
            // Invalid source types fail before evaluating the dynamic target.
            // The first six cases have valid source types and fail only when
            // the computed target's runtime signature is bound.
            ("METHOD_TRACE", i64::from(index < 6)),
            ("METHOD_BODY_COUNT", 0),
            ("METHOD_INDEX_COUNT", 0),
        ] {
            assert_method_watch(&vm, &artifact, name, 0, VmValue::Integer(expected));
        }
    }
}

#[test]
fn dynamic_method_recursion_stops_at_vm_call_depth() {
    let artifact = compile_with_header(
        METHOD_FIXTURE_HEADER,
        METHOD_FIXTURE_SOURCE,
        &method_options(true),
    );
    let (vm, report) = run_method_case(
        &artifact,
        "METHOD_CASE_FINITE_RECURSION",
        VmConfig {
            maximum_call_depth: 3,
            ..VmConfig::default()
        },
    );
    assert_eq!(take_fault(report).code, VmFaultCode::ResourceLimit);
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_method_ref_forwarding_resolves_the_original_array_owner() {
    let source = METHOD_FIXTURE_SOURCE.to_owned()
        + r#"
@FORWARD_ENTRY
CALL METHOD_RESET
RESULT:0 = GETMETH("FORWARD_ARRAYS", , METHOD_VALUES, METHOD_WORDS)
RETURN RESULT:0
@FORWARD_ARRAYS(NUMBERS, TEXTS)
#FUNCTION
#DIM REF NUMBERS
#DIMS REF TEXTS
RETURNF GETMETH("METHOD_REF", , NUMBERS:METHOD_INDEX(), TEXTS:METHOD_INDEX())
@NESTED_ENTRY
CALL METHOD_RESET
RESULT:0 = GETMETH("METHOD_PAIR", , GETMETH("METHOD_ECHO", , 2), 3)
RETURN RESULT:0
"#;
    let artifact = compile_with_header(METHOD_FIXTURE_HEADER, &source, &method_options(true));
    let (vm, report) = run_method_case(&artifact, "FORWARD_ENTRY", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(11));
    assert_method_watch(&vm, &artifact, "METHOD_VALUES", 0, VmValue::Integer(11));
    assert_method_watch(
        &vm,
        &artifact,
        "METHOD_WORDS",
        1,
        VmValue::String("changed".into()),
    );
    assert_method_watch(&vm, &artifact, "METHOD_INDEX_COUNT", 0, VmValue::Integer(0));
    let (vm, report) = run_method_case(&artifact, "NESTED_ENTRY", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(23));
    assert_method_watch(&vm, &artifact, "METHOD_BODY_COUNT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_methods_keep_existing_optional_and_conversion_policy() {
    let mut options = method_options(true);
    options.compatible_function_argument_optional = true;
    options.compatible_function_argument_auto_convert = true;
    let artifact = compile_with_header(
        "",
        r#"@SYSTEM_TITLE
RESULTS:0 '= GETMETHS("POLICY_TEXT", , 123)
RESULTS:1 '= GETMETHS("POLICY_TEXT")
RESULT:0 = EXISTMETH("POLICY_TEXT")
RETURN RESULT:0
@POLICY_TEXT(TEXT)
#FUNCTIONS
#DIMS DYNAMIC TEXT
RETURNF TEXT
"#,
        &options,
    );
    let (vm, report) = run_method_case(&artifact, "SYSTEM_TITLE", VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert_method_watch(&vm, &artifact, "RESULTS", 0, VmValue::String("123".into()));
    assert_method_watch(&vm, &artifact, "RESULTS", 1, VmValue::String(String::new()));
    assert_method_watch(&vm, &artifact, "RESULT", 0, VmValue::Integer(2));
}

#[test]
fn dynamic_method_and_runtime_form_lookups_stay_in_the_callers_generation() {
    let source = r#"@METHOD_ENTRY
#FUNCTION
RETURNF GETMETH("VALUE_METHOD") + EXISTMETH("NEW_METHOD") * 100
@FORM_ENTRY
#FUNCTIONS
RETURNF STRFORM("{GETMETH(\"VALUE_METHOD\")}:{EXISTMETH(\"NEW_METHOD\")}")
@VALUE_METHOD
#FUNCTION
RETURNF 1
"#;
    let updated =
        source.replace("RETURNF 1", "RETURNF 2") + "\n@NEW_METHOD\n#FUNCTION\nRETURNF 7\n";
    let base = compile_source_with_options(source, &method_options(true));
    let target = compile_source_with_options(&updated, &method_options(true));
    let patch = create_patch(&base, &target);
    for (name, pause_opcode, old_result, new_result) in [
        (
            "METHOD_ENTRY",
            Opcode::ResolveUserCall,
            VmValue::Integer(1),
            VmValue::Integer(102),
        ),
        (
            "FORM_ENTRY",
            Opcode::CallNative,
            VmValue::String("1:0".into()),
            VmValue::String("2:1".into()),
        ),
    ] {
        let entry = base
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        let instructions = entry
            .code
            .iter()
            .position(|instruction| instruction.opcode == pause_opcode as u16)
            .unwrap()
            + 1;
        let mut vm = Vm::new(validated(&base), VmConfig::default());
        let mut natives = NativeServiceRegistry::for_artifact_with_seed(&base, 123_456);
        let mut host = ReadyHost::default();
        let old = vm.spawn_entry(entry.key, Vec::new()).unwrap();
        let report = vm.run_slice(
            &mut host,
            &mut natives,
            RunBudget {
                maximum_instructions: u64::try_from(instructions).unwrap(),
                maximum_host_calls: 0,
                fiber_quantum: 4096,
            },
        );
        assert_eq!(report.stop, erabasic_vm::VmRunStop::BudgetExhausted);
        vm.prepare_hot_reload(
            &patch,
            &erabasic_compiler::runtime_native_validation_context(
                &target,
                &default_host_registry(),
            ),
        )
        .unwrap();
        vm.commit_hot_reload().unwrap();
        let new = vm.spawn_entry(entry.key, Vec::new()).unwrap();
        let report = vm.run_slice(&mut host, &mut natives, RunBudget::default());
        assert!(
            !report
                .events
                .iter()
                .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
            "{name}: {report:?}"
        );
        assert_eq!(
            vm.fiber_status(old),
            Some(FiberStatus::Completed(Some(old_result)))
        );
        assert_eq!(
            vm.fiber_status(new),
            Some(FiberStatus::Completed(Some(new_result)))
        );
    }
}
