use crate::*;
use erabasic_ast::{Argument, AssignOp, BinaryOp, DiagnosticCode, ExprKind, StatementKind};

#[test]
fn precedence_matches_expected_shape() {
    let output = parse_expression("1 + 2 * 3", &DefaultParserContext::default());
    let ExprKind::Binary {
        op: BinaryOp::Add,
        right,
        ..
    } = output.value.unwrap().kind
    else {
        panic!("expected add")
    };
    assert!(matches!(
        right.kind,
        ExprKind::Binary {
            op: BinaryOp::Multiply,
            ..
        }
    ));
}

#[test]
fn parses_assignment_line() {
    let output = parse_line("LOCAL:0 += 2", &DefaultParserContext::default());
    assert!(matches!(
        output.value.unwrap().kind,
        StatementKind::Assignment {
            op: AssignOp::Add,
            ..
        }
    ));
}

#[test]
fn parses_erb_function_and_checks_blocks() {
    let source = "@TEST, ARG=1\nIF ARG\nPRINTFORM value={ARG}\nENDIF\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    assert_eq!(output.value.unwrap().functions.len(), 1);
}

#[test]
fn erh_define_is_visible_to_following_input() {
    let mut context = DefaultParserContext::default();
    let _ = parse_erh("#DEFINE TEN 10\n", &mut context);
    let output = parse_expression("TEN + 1", &context);
    assert!(!output.has_errors());
}

#[test]
fn ternary_uses_era_sharp_separator() {
    let output = parse_expression("FLAG ? 10 # 20", &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    assert!(matches!(
        output.value.unwrap().kind,
        ExprKind::Ternary { .. }
    ));
}

#[test]
fn printform_argument_becomes_formatted_ast() {
    let output = parse_line(
        "PRINTFORM value={LOCAL:0}",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected instruction")
    };
    assert!(matches!(arguments.first(), Some(Argument::Formatted(_))));
}

#[test]
fn media_arguments_preserve_mixed_number_units() {
    let output = parse_line(
        "PRINT_RECT 10px, 20, 30px, 40",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected instruction");
    };
    assert!(matches!(
        arguments.as_slice(),
        [
            Argument::MixedExpression { is_px: true, .. },
            Argument::MixedExpression { is_px: false, .. },
            Argument::MixedExpression { is_px: true, .. },
            Argument::MixedExpression { is_px: false, .. }
        ]
    ));

    let utf8 = parse_line(
        "PRINT_IMG \"画像\", \"選択\", \"マスク\", 100, 20px, 0",
        &DefaultParserContext::default(),
    );
    assert!(!utf8.has_errors(), "{:#?}", utf8.diagnostics);
}

#[test]
fn string_assignment_uses_percent_form_interpolation() {
    let output = parse_line(
        "RESULTS:0 = %MAP_GET(\"m\", \"k\")%",
        &DefaultParserContext::default(),
    );
    assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
    let StatementKind::Assignment { value, .. } = output.value.unwrap().kind else {
        panic!("expected assignment");
    };
    assert!(matches!(value.kind, ExprKind::Formatted(_)));
}

#[test]
fn preprocessor_omits_inactive_branch() {
    let source = "[IF 0]\n@OMITTED\n[ELSE]\n@KEPT\n[ENDIF]\nRETURN\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let script = output.value.unwrap();
    assert_eq!(script.functions.len(), 1);
    assert_eq!(script.functions[0].name, "KEPT");
}

#[test]
fn reports_unclosed_control_structure() {
    let output = parse_erb("@TEST\nIF 1\n", &mut DefaultParserContext::default());
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::UnmatchedBlock)
    );
}

#[test]
fn printdata_suffixes_and_nested_datalist_have_reference_block_shape() {
    let source = "@TEST\nPRINTDATADW\nDATALIST\nDATAFORM one\nDATAFORM two\nENDLIST\nENDDATA\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
}

#[test]
fn try_function_lists_close_with_endfunc_not_endlist() {
    let source = "@TEST\nTRYCALLLIST\nFUNC FIRST, 1\nFUNC SECOND, 2\nENDFUNC\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);

    let invalid = parse_erb(
        "@TEST\nTRYCALLLIST\nFUNC FIRST\nENDLIST\n",
        &mut DefaultParserContext::default(),
    );
    assert!(invalid.has_errors());
}

#[test]
fn dynamic_call_separates_formatted_target_from_lazy_arguments() {
    let output = parse_line(
        "CALLFORM CHARAMOVE_{ARG}(4, LOCAL)",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected instruction");
    };
    assert!(matches!(arguments.first(), Some(Argument::Formatted(_))));
    assert_eq!(arguments.len(), 3);

    let comma = parse_line(
        "FUNC CHARAMOVE_{ARG}, 4, LOCAL",
        &DefaultParserContext::default(),
    );
    assert!(!comma.has_errors(), "{:#?}", comma.diagnostics);
    let StatementKind::Instruction { arguments, .. } = comma.value.unwrap().kind else {
        panic!("expected instruction");
    };
    assert_eq!(arguments.len(), 3);
}
