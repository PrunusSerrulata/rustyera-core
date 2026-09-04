use super::*;
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
fn goto_between_nested_scopes_retains_the_outer_loop_and_enters_the_target_select() {
    let artifact = compile_source(
        "@SYSTEM_TITLE\n\
         #DIM INDEX\n\
         FOR INDEX, 0, 2\n\
             SELECTCASE 0\n\
                 CASE 0\n\
                     GOTO SECOND_BODY\n\
             ENDSELECT\n\
             SELECTCASE 1\n\
                 CASE 1\n\
                     $SECOND_BODY\n\
                     RESULT += 1\n\
             ENDSELECT\n\
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
        2,
        "{:#?}",
        report.events
    );
    assert_eq!(result, VmValue::Integer(2));
}
