use super::*;
#[test]
fn prefix_and_postfix_preserve_profile_specific_return_and_storage_at_integer_boundaries() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let snake = profile == erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
        for (expression, cases) in integer_mutation_boundary_cases() {
            let artifact = compile_source_with_options(
                &format!("@SYSTEM_TITLE\nRESULT = {expression}\nRETURN RESULT\n"),
                &AnalyzerOptions {
                    compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                    ..AnalyzerOptions::default()
                },
            );
            let flag = artifact
                .globals
                .iter()
                .find(|global| global.name == "FLAG")
                .unwrap()
                .key;
            let result = artifact
                .globals
                .iter()
                .find(|global| global.name == "RESULT")
                .unwrap()
                .key;
            for (initial, original_return, original_store, snake_return, snake_store) in cases {
                let (returned, stored) = if snake {
                    (snake_return, snake_store)
                } else {
                    (original_return, original_store)
                };
                let mut natives = NativeServiceRegistry::for_artifact(&artifact);
                let mut vm = Vm::new(validated(&artifact), VmConfig::default());
                vm.write_variable(flag, &[0], None, VmValue::Integer(initial))
                    .unwrap();
                let fiber = vm
                    .spawn_entry(artifact.functions[0].key, Vec::new())
                    .unwrap();
                let report = vm.run_slice(
                    &mut ReadyHost::default(),
                    &mut natives,
                    RunBudget::default(),
                );
                assert!(
                    matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))),
                    "{profile}: {expression}, input {initial}: {report:?}"
                );
                assert_eq!(
                    vm.read_variable(result, &[0], None),
                    Ok(VmValue::Integer(returned)),
                    "{profile}: {expression}, input {initial}: return"
                );
                assert_eq!(
                    vm.read_variable(flag, &[0], None),
                    Ok(VmValue::Integer(stored)),
                    "{profile}: {expression}, input {initial}: storage"
                );
            }
        }
    }
}

#[test]
fn snake_unchecked_calls_and_toint_use_the_compiled_native_policy() {
    let artifact = compile_source_with_options(
        r#"@SYSTEM_TITLE
#DIM HIGH
#DIM LOW
#DIMS NUMBER
HIGH = 9223372036854775807
LOW = -9223372036854775807 - 1
RESULT:0 = UNCHECKED_ADD(HIGH, 1)
RESULT:1 = UNCHECKED_SUB(LOW, 1)
RESULT:2 = UNCHECKED_MUL(HIGH, 2)
RESULT:3 = UNCHECKED_NEG(LOW)
NUMBER '= "9223372036854775808"
RESULT:4 = TOINT(NUMBER)
RESULT:5 = TOINT("0b102")
RESULT:6 = TOINT("12.99")
RETURN RESULT:0
"#,
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::default()
        },
    );
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    let fiber = vm
        .spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(
        matches!(vm.fiber_status(fiber), Some(FiberStatus::Completed(_))),
        "{report:?}"
    );
    for (index, expected) in [i64::MIN, i64::MAX, -2, i64::MIN, 0, 0, 12]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            vm.read_variable(result, &[index as u64], None),
            Ok(VmValue::Integer(expected))
        );
    }
}

#[test]
fn snake_toint_does_not_catch_errors_evaluating_its_argument() {
    let artifact = compile_source_with_options(
        r"@SYSTEM_TITLE
#DIMS VALUES, 1
#DIM INDEX
INDEX = 99
RESULT = 73
RESULT = TOINT(VALUES:INDEX)
RETURN
",
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            ..AnalyzerOptions::default()
        },
    );
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());
    vm.spawn_entry(artifact.functions[0].key, Vec::new())
        .unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_eq!(
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(73))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. }))
    );
}

#[test]
fn inclusive_string_comparisons_preserve_equal_and_ordered_values_in_both_profiles() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let artifact = compile_source_with_options(
            r#"@SYSTEM_TITLE
#DIMS L_TEXT
#DIMS R_TEXT
L_TEXT '= "alpha"
R_TEXT '= "alpha"
RESULT = (L_TEXT >= R_TEXT) + (L_TEXT <= R_TEXT)
R_TEXT '= "beta"
RESULT += (L_TEXT <= R_TEXT) * 4
RESULT += (L_TEXT >= R_TEXT) * 8
RESULT += (R_TEXT >= L_TEXT) * 16
RESULT += (R_TEXT <= L_TEXT) * 32
RETURN RESULT
"#,
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::default()
            },
        );
        assert_eq!(
            run_compiled_result(&artifact),
            VmValue::Integer(22),
            "{profile}"
        );
    }
}

#[test]
fn dynamic_method_depth_failure_preserves_the_targets_persistent_argument() {
    let artifact = compile_source(
        r#"@DEPTH_ENTRY
RESULT = GETMETH("PERSISTENT_TARGET", , 37)
RETURN
@PERSISTENT_TARGET, ARG
#FUNCTION
FLAG:1 += 1
RETURNF ARG
"#,
    );
    let target = artifact
        .functions
        .iter()
        .find(|function| function.name == "PERSISTENT_TARGET")
        .unwrap();
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "DEPTH_ENTRY")
        .unwrap()
        .key;
    let argument = target.parameters[0].key;
    assert_eq!(
        artifact
            .globals
            .iter()
            .find(|global| global.key == argument)
            .unwrap()
            .storage,
        BytecodeStorage::FunctionPersistent
    );
    let flag = artifact
        .globals
        .iter()
        .find(|global| global.name == "FLAG")
        .unwrap()
        .key;
    let mut vm = Vm::new(
        validated(&artifact),
        VmConfig {
            maximum_call_depth: 1,
            ..VmConfig::default()
        },
    );
    let mut natives = NativeServiceRegistry::for_artifact(&artifact);
    // Prime the real persistent parameter through a legal root call, rather than
    // asserting that a DYNAMIC local which never existed remained unchanged.
    let primed = vm
        .spawn_entry(target.key, vec![VmValue::Integer(999)])
        .unwrap();
    vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert_eq!(
        vm.fiber_status(primed),
        Some(FiberStatus::Completed(Some(VmValue::Integer(999))))
    );
    assert_eq!(
        vm.read_variable(argument, &[0], None),
        Ok(VmValue::Integer(999))
    );
    vm.write_variable(flag, &[1], None, VmValue::Integer(0))
        .unwrap();
    vm.spawn_entry(entry, Vec::new()).unwrap();
    let report = vm.run_slice(
        &mut ReadyHost::default(),
        &mut natives,
        RunBudget::default(),
    );
    assert!(report.events.iter().any(|event| matches!(event, VmEvent::FiberFaulted { fault, .. } if fault.code == VmFaultCode::ResourceLimit)), "{report:?}");
    assert_eq!(
        vm.read_variable(argument, &[0], None),
        Ok(VmValue::Integer(999))
    );
    assert_eq!(vm.read_variable(flag, &[1], None), Ok(VmValue::Integer(0)));
}

#[test]
fn power_statement_writes_the_destination_instead_of_passing_its_place_as_an_operand() {
    let artifact = compile_source("@SYSTEM_TITLE\nPOWER RESULT, 2, 3\nRETURN RESULT\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(8));
}

#[test]
fn compiled_form_padding_uses_portable_ambiguous_character_columns() {
    let artifact = compile_source("@SYSTEM_TITLE\nRESULTS:0 = %\"■……■\", 12, LEFT%\nRETURN\n");
    assert_eq!(
        run_compiled_string_result(&artifact),
        VmValue::String(format!("■……■{}", " ".repeat(4)))
    );
}

#[test]
fn static_call_target_with_an_inline_comment_executes_in_the_vm() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nCALL TARGET; inline comment\nRETURN RESULT\n@TARGET\nRESULT = 42\nRETURN RESULT\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(42));
}

#[test]
fn scalar_ref_parameters_store_aliases_and_mutate_the_callers_arrays() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALL MUTATE_REF(VALUES)\nRETURN RESULT\n@MUTATE_REF(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
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
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn dynamic_calls_bind_variable_arguments_as_refs_or_values_from_the_target_signature() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM VALUES, 3\nVALUES:1 = 3\nCALLFORM MUTATE_{1}(VALUES)\nCALLFORM READ_{1}(VALUES:1)\nRETURN RESULT\n@MUTATE_1(NUMBERS)\n#DIM REF NUMBERS\nNUMBERS:1 = 7\nRETURN RESULT\n@READ_1(VALUE)\n#DIM VALUE\nRESULT:1 = VALUE\nRETURN RESULT\n",
    );
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
        .key;
    let values = artifact
        .globals
        .iter()
        .find(|global| global.name == "VALUES")
        .expect("VALUES")
        .key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .expect("RESULT")
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
        vm.read_variable(values, &[1], None),
        Ok(VmValue::Integer(7))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(7))
    );
}

#[test]
fn place_writes_preserve_project_function_character_and_ref_storage() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM VALUES, 3\n\
         #DIMS WORDS, 3\n\
         ADDVOIDCHARA\n\
         FLAG:0 = 10\n\
         CFLAG:0:0 = 20\n\
         CALL WRITE_PERSISTENT\n\
         FLAG:10 = READ_STATIC()\n\
         FLAG:11 = READ_STATIC()\n\
         CALL WRITE_REFS(VALUES, WORDS)\n\
         FLAG:12 = VALUES:1\n\
         FLAG:13 = CFLAG:0:0\n\
         FLAG:14 = FLAG:0\n\
         RESULTS:0 '= WORDS:0\n\
         RESULTS:1 '= WORDS:1\n\
         RETURN\n\
         @WRITE_PERSISTENT\n\
         LOCAL:0 = 30\n\
         FLAG:15 = LOCAL:0\n\
         RETURN\n\
         @READ_STATIC\n\
         #FUNCTION\n\
         #DIM STATIC CACHE, 1\n\
         CACHE:0 += 1\n\
         RETURNF CACHE:0\n\
         @WRITE_REFS(NUMBERS, TEXTS)\n\
         #DIM REF NUMBERS\n\
         #DIMS REF TEXTS\n\
         VARSET NUMBERS, 40\n\
         SPLIT \"left,right\", \",\", TEXTS\n\
         RETURN\n",
    );
    let key = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name == name && global.owner.is_none())
            .expect(name)
            .key
    };
    let flag = key("FLAG");
    let results = key("RESULTS");
    let entry = artifact
        .functions
        .iter()
        .find(|function| function.name == "SYSTEM_TITLE")
        .expect("SYSTEM_TITLE")
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
        (10..=15)
            .map(|index| vm.read_variable(flag, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 40, 20, 10, 30].map(VmValue::Integer).to_vec()
    );
    assert_eq!(
        (0..=1)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["left", "right"]
            .map(|value| VmValue::String(value.into()))
            .to_vec()
    );
}

#[test]
fn public_place_writes_still_reject_immutable_variables() {
    let artifact = compile_source("@SYSTEM_TITLE\n#DIM CONST LOCKED = 7\nRETURN LOCKED\n");
    let locked = artifact
        .globals
        .iter()
        .find(|global| global.name == "LOCKED")
        .expect("LOCKED")
        .key;
    let mut vm = Vm::new(validated(&artifact), VmConfig::default());

    assert!(matches!(
        vm.write_variable(locked, &[0], None, VmValue::Integer(9)),
        Err(VmError::InvalidState(message)) if message == "variable is immutable"
    ));
}

#[test]
fn dynamic_calls_convert_integer_places_for_string_value_parameters() {
    let mut options = AnalyzerOptions::analysis_mode();
    options.compatible_function_argument_auto_convert = true;
    let artifact = compile_source_with_options(
        "@SYSTEM_TITLE\n#DIM SKILLNUM\nSKILLNUM = 7\nCALLFORM FRIEND_SKILL_DOWNBASE, SKILLNUM\nRETURN RESULT\n@FRIEND_SKILL_DOWNBASE, ARGS\nRESULTS:0 = %ARGS%\nRETURN RESULT\n",
        &options,
    );
    assert_eq!(
        run_compiled_string_result(&artifact),
        VmValue::String("7".into())
    );
}
