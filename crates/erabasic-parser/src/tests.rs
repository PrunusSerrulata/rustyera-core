use crate::*;
use erabasic_ast::{
    Alignment, Argument, AssignOp, BinaryOp, DiagnosticCode, ExprKind, FormPart, StatementKind,
};

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
fn bare_assignment_rhs_is_an_empty_string() {
    let output = parse_line("RESULTS =", &DefaultParserContext::default());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(matches!(
        output.value.unwrap().kind,
        StatementKind::Assignment {
            value: erabasic_ast::Expr {
                kind: ExprKind::String(ref value),
                ..
            },
            ..
        } if value.is_empty()
    ));
}

#[test]
fn string_assignment_recovers_unquoted_form_text() {
    let output = parse_line(
        "LOCALS = HP(%CALLNAME:MASTER%)",
        &DefaultParserContext::default(),
    );
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(matches!(
        output.value.unwrap().kind,
        StatementKind::Assignment {
            value: erabasic_ast::Expr {
                kind: ExprKind::Formatted(_),
                ..
            },
            ..
        }
    ));
}

#[test]
fn apostrophe_equals_uses_string_expression_assignment() {
    let output = parse_line(
        "RESULTS '= REPLACE(ARGS, \"x\", \"y\")",
        &DefaultParserContext::default(),
    );
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(matches!(
        output.value.unwrap().kind,
        StatementKind::Assignment {
            op: AssignOp::StringAssign,
            value: erabasic_ast::Expr {
                kind: ExprKind::Call { .. },
                ..
            },
            ..
        }
    ));
}

#[test]
fn times_parses_real_literal_as_an_exact_ratio() {
    let output = parse_line("TIMES LOCAL:1, 1.25", &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected TIMES instruction");
    };
    assert_eq!(arguments.len(), 3);
    assert!(matches!(
        arguments.get(1),
        Some(Argument::Expression(erabasic_ast::Expr {
            kind: ExprKind::Integer(5),
            ..
        }))
    ));
    assert!(matches!(
        arguments.get(2),
        Some(Argument::Expression(erabasic_ast::Expr {
            kind: ExprKind::Integer(4),
            ..
        }))
    ));
}

#[test]
fn printv_apostrophe_operands_are_raw_strings() {
    let output = parse_line(
        "PRINTV 'LV,ABL:親密,'(,ABL:親密,')",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected PRINTV");
    };
    assert!(matches!(
        arguments.first(),
        Some(Argument::Expression(erabasic_ast::Expr {
            kind: ExprKind::String(value),
            ..
        })) if value == "LV"
    ));
}

#[test]
fn plain_assignment_defers_form_text_lexing_to_semantic_analysis() {
    let output = parse_line("LOCALS = 東方　カード", &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Assignment { raw_value, .. } = output.value.unwrap().kind else {
        panic!("expected assignment");
    };
    assert_eq!(raw_value, "東方　カード");
}

#[test]
fn case_preserves_comparison_and_range_selector_grammar() {
    for (source, expected) in [
        ("CASE IS < 20", "IS < 20"),
        ("CASE 20 TO 60", "20 TO 60"),
        ("CASE 8, 9", "8, 9"),
    ] {
        let output = parse_line(source, &DefaultParserContext::default());
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
            panic!("CASE instruction expected");
        };
        assert_eq!(arguments, vec![Argument::Raw(expected.into())]);
    }
}

#[test]
fn numeric_assignment_list_is_deferred_until_the_target_type_is_known() {
    let output = parse_line("LOCAL = 5, 6, 7", &DefaultParserContext::default());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let StatementKind::Assignment {
        additional_values,
        raw_value,
        ..
    } = output.value.unwrap().kind
    else {
        panic!("assignment expected");
    };
    assert!(additional_values.is_empty());
    assert_eq!(raw_value, "5, 6, 7");
}

#[test]
fn plain_assignment_with_commas_stays_raw_until_target_type_is_known() {
    let output = parse_line("LOCALS = a,　b", &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Assignment {
        additional_values,
        raw_value,
        ..
    } = output.value.unwrap().kind
    else {
        panic!("expected assignment");
    };
    assert!(additional_values.is_empty());
    assert_eq!(raw_value, "a,　b");
}

#[test]
fn parses_rename_symbol_as_one_expression_term() {
    let output = parse_line("RESULT = [[霊夢]]", &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let expressions = parse_expression_list_at("[[霊夢]]", 0, &DefaultParserContext::default());
    assert!(!expressions.has_errors(), "{:#?}", expressions.diagnostics);
    assert!(matches!(
        expressions.value.unwrap().as_slice(),
        [erabasic_ast::Expr {
            kind: ExprKind::Identifier(name),
            ..
        }] if name == "[[霊夢]]"
    ));
}

#[test]
fn string_variable_can_be_named_minus() {
    let mut context = DefaultParserContext::default();
    assert!(context.register_variable("MINUS"));
    let output = parse_line("MINUS = -", &context);
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    assert!(matches!(
        output.value.unwrap().kind,
        StatementKind::Assignment { .. }
    ));
}

#[test]
fn parses_each_character_variable_index_at_the_outer_level() {
    let output = parse_line(
        "CFLAG:TARGET:現在位置 '= TCVAR:MASTER:(LOCAL:1)",
        &DefaultParserContext::default(),
    );
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let StatementKind::Assignment { target, value, .. } = output.value.unwrap().kind else {
        panic!("assignment expected");
    };
    assert_eq!(target.indices.len(), 2);
    let ExprKind::Variable { indices, .. } = value.kind else {
        panic!("variable value expected");
    };
    assert_eq!(indices.len(), 2);
    assert!(matches!(indices[1].kind, ExprKind::Group(_)));
}

#[test]
fn lowers_standalone_increment_and_decrement_to_assignments() {
    let increment = parse_line("LOCAL:1 ++", &DefaultParserContext::default());
    assert!(
        increment.diagnostics.is_empty(),
        "{:?}",
        increment.diagnostics
    );
    let StatementKind::Assignment {
        target,
        op: AssignOp::Add,
        value,
        ..
    } = increment.value.unwrap().kind
    else {
        panic!("increment assignment expected");
    };
    assert_eq!(target.indices.len(), 1);
    assert!(matches!(value.kind, ExprKind::Integer(1)));

    let decrement = parse_line("--CNT_CHARA", &DefaultParserContext::default());
    assert!(
        decrement.diagnostics.is_empty(),
        "{:?}",
        decrement.diagnostics
    );
    assert!(matches!(
        decrement.value.unwrap().kind,
        StatementKind::Assignment {
            op: AssignOp::Subtract,
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
fn printform_preserves_width_alignment_and_triple_parts() {
    let output = parse_line(
        "PRINTFORM |{LOCAL:0, 6, LEFT}|%LOCALS:0, 8, RIGHT%|***",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected instruction")
    };
    let [Argument::Formatted(formatted)] = arguments.as_slice() else {
        panic!("expected formatted argument")
    };
    assert!(matches!(
        formatted.parts.as_slice(),
        [
            FormPart::Text(_),
            FormPart::IntegerInterpolation {
                alignment: Some(Alignment::Left),
                ..
            },
            FormPart::Text(_),
            FormPart::StringInterpolation {
                alignment: Some(Alignment::Right),
                ..
            },
            FormPart::Text(_),
            FormPart::Triple { symbol: '*', .. }
        ]
    ));
}

#[test]
fn plain_print_preserves_format_metacharacters_as_raw_text() {
    let output = parse_line(
        "PRINTDL ascii :{ 50% }; comment-like text",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected instruction");
    };
    assert!(matches!(arguments.as_slice(), [Argument::Raw(value)] if value == "ascii :{ 50% }"));
}

#[test]
fn plain_print_ascii_art_is_not_misparsed_as_assignment() {
    let output = parse_line(
        "PRINTDL 　　　　　　　　　-=ﾆ====-　　ﾆ=-",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction {
        name, arguments, ..
    } = output.value.unwrap().kind
    else {
        panic!("expected instruction");
    };
    assert_eq!(name, "PRINTDL");
    assert!(matches!(arguments.as_slice(), [Argument::Raw(_)]));
}

#[test]
fn joins_braced_physical_lines_and_consumes_utf8_bom() {
    let source = "\u{feff}; header\n{\n#DIMS CONST SUITS, 2 = \"A\",\n  \"B\"\n}\n";
    let output = parse_erh(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let script = output.value.unwrap();
    assert_eq!(script.declarations.len(), 1);
    assert_eq!(
        script.declarations[0].raw_arguments,
        "CONST SUITS, 2 = \"A\",   \"B\" "
    );
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
