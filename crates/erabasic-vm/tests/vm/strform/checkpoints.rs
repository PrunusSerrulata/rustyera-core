use super::*;
fn run_checked_variable_domain_case(local_setup: &str, failing_statement: &str) {
    let source = format!(
        r#"@SYSTEM_TITLE
#DIM DYNAMIC VALUES, 2
VALUES:0 = 10
FLAG:4 = 17
RESULT:10 = 73
RESULT:10 = STRFORMCHECK("{{VARIABLE_FAILURE(VALUES)}}")
RESULT:11 = RECOVERED_VARIABLE_METHOD(VALUES)
RESULT:12 = CHECK_LOCKED
FLAG:9 = VALUES:0
RETURN
@VARIABLE_FAILURE(ITEMS)
#FUNCTION
#DIM REF ITEMS
{local_setup}
ITEMS:0 += 1
{failing_statement}
FLAG:8 = 1
RETURNF 99
@RECOVERED_VARIABLE_METHOD(ITEMS)
#FUNCTION
#DIM REF ITEMS
ITEMS:0 += 2
RETURNF ITEMS:0
"#
    );
    let artifact = compile_with_header(
        "#DIM CONST CHECK_LOCKED = 7\n",
        &source,
        &method_options(true),
    );
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{failing_statement}: {report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{failing_statement}: {report:?}"
    );
    for (index, value) in [(8, 0), (10, 0), (11, 13), (12, 7)] {
        assert_method_watch(&vm, &artifact, "RESULT", index, VmValue::Integer(value));
    }
    for (index, value) in [(4, 17), (8, 0), (9, 13)] {
        assert_method_watch(&vm, &artifact, "FLAG", index, VmValue::Integer(value));
    }
}

#[test]
fn checked_user_method_catches_dynamic_variable_read_and_named_index_domains() {
    for (setup, statement) in [
        (
            "#DIMS REFERENCE\nREFERENCE '= \"MISSING_CHECK_VARIABLE\"",
            "RESULT:8 = GETVAR(REFERENCE)",
        ),
        (
            "#DIMS KEY\nKEY '= \"MISSING_CHECK_FLAG_NAME\"",
            "RESULT:8 = FLAG:KEY",
        ),
        (
            "#DIMS REFERENCE\nREFERENCE '= \"FLAG:9999999\"",
            "RESULT:8 = GETVAR(REFERENCE)",
        ),
    ] {
        run_checked_variable_domain_case(setup, statement);
    }
}

#[test]
fn checked_user_method_catches_read_only_negative_and_out_of_range_setvar_domains() {
    for reference in ["CHECK_LOCKED", "FLAG:-1", "FLAG:9999999"] {
        let setup = format!("#DIMS REFERENCE\nREFERENCE '= \"{reference}\"");
        run_checked_variable_domain_case(&setup, "RESULT:8 = SETVAR(REFERENCE, 99)");
    }
}

// Exercise real script scopes. Candidate oracle captures are separate from these
// Rust contract assertions and remain pending until the authorized matrix runs.
fn run_nested_checkpoint_contract(
    source: &str,
    expected_integers: &[(&str, u64, i64)],
    expected_string: Option<(&str, u64, &str)>,
) {
    let artifact = compile_source_with_options(source, &method_options(true));
    let (vm, report) = run_entry(&artifact, VmConfig::default());
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberFaulted { .. })),
        "{report:?}"
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, VmEvent::FiberCompleted { .. })),
        "{report:?}"
    );
    for &(name, index, value) in expected_integers {
        assert_method_watch(&vm, &artifact, name, index, VmValue::Integer(value));
    }
    if let Some((name, index, value)) = expected_string {
        assert_method_watch(&vm, &artifact, name, index, VmValue::String(value.into()));
    }
}

#[test]
fn nearest_existvar_checkpoint_precedes_enclosing_strformcheck_in_bytecode_and_form() {
    // nearest-existvar-bytecode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{PROBE()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@PROBE
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), 1)
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // nearest-existvar-form
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{PROBE()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@PROBE
#FUNCTION
RESULTS:1 '= STRFORM("{EXISTVAR(NAME_SOURCE(), 1)}")
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        Some(("RESULTS", 1, "0")),
    );
    // nearest-existvar-same-frame
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = STRFORMCHECK("{EXISTVAR(NAME_SOURCE(), 1)}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:19 = FLAG:9999999
ENDIF
RETURNF "FLAG"
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[test]
fn nearest_strformcheck_checkpoint_precedes_enclosing_existvar_in_bytecode_and_form() {
    // nearest-strformcheck-bytecode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = 73
RESULT:20 = EXISTVAR(NAME_SOURCE(), 1)
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:10 = STRFORMCHECK("{BAD()}")
FLAG:1 += 1
ENDIF
RETURNF "FLAG"
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // nearest-strformcheck-form
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:20 = 73
RESULTS:1 '= STRFORM("{EXISTVAR(NAME_SOURCE(), 1)}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@NAME_SOURCE
#FUNCTIONS
FLAG:0 += 1
IF FLAG:0 == 2
RESULT:10 = STRFORMCHECK("{BAD()}")
FLAG:1 += 1
ENDIF
RETURNF "FLAG"
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 10, 0),
            ("RESULT", 20, 73),
            ("RESULT", 21, 1),
            ("FLAG", 0, 2),
            ("FLAG", 1, 1),
            ("FLAG", 2, 1),
            ("FLAG", 9, 1),
        ],
        Some(("RESULTS", 1, "1")),
    );
    // nearest-strformcheck-same-frame
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULTS:0 '= "{BAD()}"
RESULT:20 = EXISTVAR(STRFORM("{STRFORMCHECK(RESULTS:0)}"), 1)
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@BAD
#FUNCTION
FLAG:2 += 1
RETURNF FLAG:9999999
"#,
        &[
            ("RESULT", 20, 1),
            ("RESULT", 21, 1),
            ("FLAG", 2, 2),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[test]
#[allow(clippy::too_many_lines)] // Keep each fixture and its lifecycle assertions together.
fn outer_check_catches_failed_inner_parameters_in_order_without_entering_inner_scope() {
    // parameter-failure-existvar-first-source
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), MODE_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RESULT:19 = FLAG:9999999
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 1),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // parameter-failure-existvar-mode
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = EXISTVAR(NAME_SOURCE(), MODE_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RESULT:19 = FLAG:9999999
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 12),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
    // parameter-failure-strformcheck-source
    run_nested_checkpoint_contract(
        r#"@SYSTEM_TITLE
RESULT:10 = 73
RESULT:19 = 61
RESULT:20 = STRFORMCHECK("{CHILD()}{AFTER()}")
RESULT:21 = STRFORMCHECK("fresh")
FLAG:9 = 1
RETURN RESULT
@CHILD
#FUNCTION
RESULT:10 = STRFORMCHECK(NAME_SOURCE())
FLAG:1 += 1
RETURNF 7
@NAME_SOURCE
#FUNCTIONS
FLAG:0 = FLAG:0 * 10 + 1
RESULT:19 = FLAG:9999999
RETURNF "FLAG"
@MODE_SOURCE
#FUNCTION
FLAG:0 = FLAG:0 * 10 + 2
RETURNF 1
@AFTER
#FUNCTION
FLAG:2 += 1
RETURNF 8
"#,
        &[
            ("RESULT", 10, 73),
            ("RESULT", 19, 61),
            ("RESULT", 20, 0),
            ("RESULT", 21, 1),
            ("FLAG", 0, 1),
            ("FLAG", 1, 0),
            ("FLAG", 2, 0),
            ("FLAG", 9, 1),
        ],
        None,
    );
}

#[path = "checkpoints/restructuring.rs"]
pub(super) mod restructuring;
