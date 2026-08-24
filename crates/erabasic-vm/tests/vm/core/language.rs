use super::*;
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

#[test]
fn while_false_branch_skips_past_wend_and_finite_loops_terminate() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM ITERATIONS\nWHILE ITERATIONS < 3\nITERATIONS ++\nWEND\nWHILE 0\nITERATIONS = 99\nWEND\nRETURN ITERATIONS\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(3));
}

#[test]
fn bare_return_zeros_result_zero_and_preserves_the_remaining_result_array() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\nRESULT:1 = 7\nCALL HELPER\nRETURN (RESULT:0 == 0) && (RESULT:1 == 7)\n@HELPER\nRESULT = 99\nRETURN\n",
    );
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn logical_operators_short_circuit_their_right_operand() {
    for (expression, expected) in [
        ("1 || VALUES:1", 1),
        ("0 && VALUES:1", 0),
        ("0 !& VALUES:1", 1),
        ("1 !| VALUES:1", 0),
        ("1 && 7", 1),
        ("0 || 7", 1),
        ("1 !& 0", 1),
        ("0 !| 0", 1),
    ] {
        let artifact = compile_source(&format!(
            "@SYSTEM_TITLE\n#DIM VALUES, 1\nRETURN {expression}\n"
        ));
        assert_eq!(
            run_compiled_result(&artifact),
            VmValue::Integer(expected),
            "{expression}"
        );
    }
}

#[test]
fn rand_pseudo_variable_uses_the_random_native_instead_of_schema_storage() {
    let artifact = compile_source("@SYSTEM_TITLE\nRETURN RAND:1\n");
    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(0));
}

#[test]
fn repeat_updates_count_and_continue_runs_rend_increment() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT = 0\n\
         REPEAT 4\n\
             SIF COUNT == 1\n\
                 CONTINUE\n\
             RESULT += COUNT + 1\n\
         REND\n\
         RESULT = RESULT * 10 + COUNT\n\
         RETURN RESULT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(84));
}

#[test]
fn break_advances_repeat_count_before_leaving_the_loop() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULT = 0\n\
         REPEAT 5\n\
             RESULT += 10\n\
             BREAK\n\
         REND\n\
         RESULT += COUNT\n\
         RETURN RESULT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(11));
}

#[test]
fn selectcase_loop_rejects_the_previous_string_tip() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS '= GET_TIP(0)\n\
         RESULT = RESULTS != \"zero\" && RESULTS != \"\"\n\
         RETURN RESULT\n\
         @GET_TIP(ARG)\n\
         #FUNCTIONS\n\
         #DIM INDEX\n\
         #DIMS TEXT\n\
         DO\n\
             SELECTCASE INDEX\n\
                 CASE 0\n\
                     TEXT = \"zero\"\n\
                 CASEELSE\n\
                     TEXT = \"one\"\n\
             ENDSELECT\n\
             INDEX += 1\n\
         LOOP TEXT == \"zero\"\n\
         RETURNF TEXT\n",
    );

    assert_eq!(run_compiled_result(&artifact), VmValue::Integer(1));
}

#[test]
fn goto_into_case_body_warns_and_treats_endselect_as_a_no_op() {
    let artifact = compile_source(
        "@ORACLE_GOTO_STRUCTURED\n\
         GOTO CHOICE\n\
         SELECTCASE 0\n\
             CASE 0\n\
                 $CHOICE\n\
                 RESULT:1 = 42\n\
         ENDSELECT\n\
         RETURN\n",
    );
    let (result, report) =
        run_compiled_entry_result_with_report(&artifact, "ORACLE_GOTO_STRUCTURED", 1);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, message, origin, notification, .. }
                if code == "vm.control_flow.goto_into_structured_block"
                    && message.contains("avoid jumping")
                    && origin.command == "Jump"
                    && *notification == VmDiagnosticNotification::LogOnly
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(42));
}

#[test]
fn goto_into_case_boundary_warns_and_skips_the_unselected_body() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         GOTO CASE_ENTRY\n\
         SELECTCASE 1\n\
             $CASE_ENTRY\n\
             CASE 1\n\
                 RESULT = 99\n\
         ENDSELECT\n\
         RESULT = 42\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, message, origin, .. }
                if code == "vm.control_flow.goto_into_structured_block"
                    && message.contains("avoid jumping")
                    && origin.command == "Jump"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(42));
}

#[test]
fn goto_into_caseelse_boundary_warns_and_skips_the_unselected_body() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         GOTO CASE_ENTRY\n\
         SELECTCASE 1\n\
             $CASE_ENTRY\n\
             CASEELSE\n\
                 RESULT = 99\n\
         ENDSELECT\n\
         RESULT = 42\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, .. }
                if code == "vm.control_flow.goto_into_structured_block"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(42));
}

#[test]
fn goto_into_for_body_warns_and_next_exits_the_inactive_loop() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM INDEX\n\
         GOTO BODY\n\
         FOR INDEX, 0, 3\n\
             $BODY\n\
             RESULT += 1\n\
         NEXT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, message, origin, .. }
                if code == "vm.control_flow.goto_into_structured_block"
                    && message.contains("avoid jumping")
                    && origin.command == "Jump"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(1));
}

#[test]
fn goto_into_for_body_warns_and_break_exits_without_a_counter() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM INDEX\n\
         GOTO BODY\n\
         FOR INDEX, 0, 3\n\
             $BODY\n\
             RESULT = 7\n\
             BREAK\n\
         NEXT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, message, origin, .. }
                if code == "vm.control_flow.goto_into_structured_block"
                    && message.contains("avoid jumping")
                    && origin.command == "Jump"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(7));
}

#[test]
fn goto_into_repeat_body_warns_and_rend_exits_the_inactive_loop() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         GOTO BODY\n\
         REPEAT 3\n\
             $BODY\n\
             RESULT += 1\n\
         REND\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, .. }
                if code == "vm.control_flow.goto_into_structured_block"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(1));
}

#[test]
fn goto_into_an_outer_loop_keeps_normally_entered_nested_loop_state_aligned() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM OUTER\n\
         #DIM INNER\n\
         GOTO OUTER_BODY\n\
         FOR OUTER, 0, 3\n\
             $OUTER_BODY\n\
             FOR INNER, 0, 2\n\
                 RESULT += 1\n\
             NEXT\n\
         NEXT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert_eq!(
        report
            .events
            .iter()
            .filter(|event| matches!(
                event,
                VmEvent::Diagnostic { code, .. }
                    if code == "vm.control_flow.goto_into_structured_block"
            ))
            .count(),
        1,
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(2));
}

#[test]
fn dynamic_goto_into_case_body_uses_the_same_warning_and_compatibility_path() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         TRYCGOTOFORM BODY\n\
         CATCH\n\
             RETURN 99\n\
         ENDCATCH\n\
         SELECTCASE 0\n\
             CASE 0\n\
                 $BODY\n\
                 RESULT = 42\n\
         ENDSELECT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, origin, .. }
                if code == "vm.control_flow.goto_into_structured_block"
                    && origin.command == "JumpDynamicLabel"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(42));
}

#[test]
fn goto_out_of_one_loop_cannot_supply_state_to_another_loop_body() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM FIRST\n\
         #DIM SECOND\n\
         FOR FIRST, 0, 3\n\
             GOTO OUTSIDE\n\
         NEXT\n\
         $OUTSIDE\n\
         GOTO SECOND_BODY\n\
         FOR SECOND, 0, 3\n\
             $SECOND_BODY\n\
             RESULT += 1\n\
         NEXT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, .. }
                if code == "vm.control_flow.goto_into_structured_block"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(1));
}

#[test]
fn goto_within_the_same_loop_preserves_state_without_warning() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM INDEX\n\
         FOR INDEX, 0, 3\n\
             GOTO SAME_BODY\n\
             $SAME_BODY\n\
             RESULT += 1\n\
         NEXT\n\
         RETURN RESULT\n",
    );
    let (result, report) = run_compiled_result_with_report(&artifact);

    assert!(
        !report.events.iter().any(|event| matches!(
            event,
            VmEvent::Diagnostic { code, .. }
                if code == "vm.control_flow.goto_into_structured_block"
        )),
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(3));
}

#[test]
fn regexpmatch_supports_positive_boundaries_without_consuming_adjacent_tokens() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n#DIM GROUP_COUNT\n#DIMS MATCHES, 4\nRESULT:0 = REGEXPMATCH(\"[$TOKEN:A][$TOKEN:B]\", \"(?<=\\\\[\\\\$TOKEN:).*?(?=\\\\])\", GROUP_COUNT, MATCHES)\nRESULT:1 = GROUP_COUNT\nRESULTS:10 '= MATCHES:0\nRESULTS:11 '= MATCHES:1\nRETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
        .key;
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
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
        vm.read_variable(result, &[0], None),
        Ok(VmValue::Integer(2))
    );
    assert_eq!(
        vm.read_variable(result, &[1], None),
        Ok(VmValue::Integer(1))
    );
    assert_eq!(
        (10..=11)
            .map(|index| vm.read_variable(results, &[index], None).unwrap())
            .collect::<Vec<_>>(),
        ["A", "B"]
            .map(|value| VmValue::String(value.into()))
            .to_vec()
    );
}

#[test]
fn one_dimensional_array_operations_accept_an_indexed_reference() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RELATION:0:0 = 4\n\
         RELATION:0:1 = 8\n\
         RELATION:0:2 = 12\n\
         RELATION:0:3 = 16\n\
         RESULT:0 = FINDELEMENT(RELATION:0:0, 12, 0, 4)\n\
         ARRAYREMOVE RELATION:0:0, 1, 1\n\
         RESULT:1 = RELATION:0:1\n\
         RETURN RESULT\n",
    );
    let entry = artifact.functions[0].key;
    let result = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULT")
        .unwrap()
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
        vm.read_variable(result, &[0], None).unwrap(),
        VmValue::Integer(2)
    );
    assert_eq!(
        vm.read_variable(result, &[1], None).unwrap(),
        VmValue::Integer(12)
    );
}

#[test]
fn conditional_form_trims_branch_edge_whitespace() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         RESULTS:0 = \\@ 0 ? unused # %\"魔力\"% \\@\n\
         RESULTS:1 = \\@ 1 ? \tkept\t # unused \\@\n\
         RETURN RESULT\n",
    );
    let results = artifact
        .globals
        .iter()
        .find(|global| global.name == "RESULTS")
        .unwrap()
        .key;
    let entry = artifact.functions[0].key;
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
        vm.read_variable(results, &[0], None).unwrap(),
        VmValue::String("魔力".into())
    );
    assert_eq!(
        vm.read_variable(results, &[1], None).unwrap(),
        VmValue::String("kept".into())
    );
}

pub(super) const CHARACTER_SHADOW_SOURCE: &str = "@SHADOW\n\
     #DIMS NAME\n\
     #DIMS CALLNAME\n\
     #DIMS NICKNAME\n\
     #DIMS MASTERNAME\n\
     #DIMS CSTR\n\
     #DIM NO\n\
     #DIM BASE\n\
     #DIM CFLAG\n\
     #DIM TARGET\n\
     RETURN\n\
     @SYSTEM_TITLE\n\
     ADDCHARA 1\n\
     ADDCHARA 2\n\
     TARGET = 2\n\
     RESULTS:0 '= NAME:1\n\
     RESULTS:1 '= CALLNAME:1\n\
     RESULTS:2 '= NICKNAME:1\n\
     RESULTS:3 '= MASTERNAME:1\n\
     RESULTS:4 '= CSTR:1:0\n\
     RESULTS:5 '= ANAME(1)\n\
     RESULTS:6 '= ANAME(2, 2)\n\
     RESULTS:7 '= CHARACTER_ROW(1)\n\
     RESULTS:8 '= CHARACTER_ROW(2)\n\
     RESULTS:9 '= NAME\n\
     RESULTS:10 '= CSTR:1:1\n\
     RESULT:11 = NO:1\n\
     RESULT:12 = BASE:1:0\n\
     RESULT:13 = CFLAG:1:0\n\
     RETURN RESULT\n\
     @ANAME(CHARA_ID = -999, CHARA_NUM = 1, ARG_SHOW_GUEST_JOB_TITLE = 1)\n\
     #FUNCTIONS\n\
     #DIM DYNAMIC CHARA_ID\n\
     #DIM DYNAMIC CHARA_NUM\n\
     #DIM DYNAMIC ARG_SHOW_GUEST_JOB_TITLE\n\
     IF CSTR:(CHARA_ID):0 != \"\"\n\
         RETURNF @\"%CSTR:(CHARA_ID):0%\\@CHARA_NUM > 1 ? 们 # \\@\"\n\
     ENDIF\n\
     RETURNF @\"%NAME:(CHARA_ID)%\\@CHARA_NUM > 1 ? 们 # \\@\"\n\
     @CHARACTER_ROW(CHARA_ID)\n\
     #FUNCTIONS\n\
     #DIM DYNAMIC CHARA_ID\n\
     RETURNF @\"◆%ANAME(CHARA_ID)%（女性）\"\n";
