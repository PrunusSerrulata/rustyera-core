use crate::*;
use erabasic_ast::{
    Alignment, Argument, AssignOp, BinaryOp, DiagnosticCode, ExprKind, FormPart, Span,
    StatementKind,
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
fn trailing_destination_comma_preserves_the_form_comma_after_assignment() {
    let output = parse_line("VALUE ,= ,", &DefaultParserContext::default());
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let StatementKind::Assignment {
        target,
        op,
        raw_value,
        value,
        ..
    } = output.value.unwrap().kind
    else {
        panic!("expected assignment");
    };
    assert_eq!(target.name, "VALUE");
    assert_eq!(op, AssignOp::Assign);
    assert_eq!(raw_value, ",");
    let ExprKind::Formatted(formatted) = value.kind else {
        panic!("expected formatted assignment value");
    };
    assert!(matches!(
        formatted.parts.as_slice(),
        [erabasic_ast::FormPart::Text(value)] if value == ","
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
fn column_options_parse_keywords_and_preserve_unicode_expression_spans() {
    let text = "DT_COLUMN_OPTIONS \"表\", \"列\", default, F(1, 2), DEFAULT, \"值\"";
    let output = parse_line(text, &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected column options");
    };
    assert_eq!(arguments.len(), 6);
    for index in [2, 4] {
        assert!(matches!(&arguments[index], Argument::Raw(value) if value == "DEFAULT"));
    }
    let Argument::Expression(value) = &arguments[3] else {
        panic!("expected value expression")
    };
    assert_eq!(&text[value.span.start..value.span.end], "F(1, 2)");
    assert!(matches!(value.kind, ExprKind::Call { .. }));
}

#[test]
fn column_options_reject_unknown_or_missing_keywords_and_values() {
    for tail in [
        "\"t\", \"c\"",
        "\"t\", \"c\", DEFAULT",
        "\"t\", \"c\", NULLABLE, 1",
        "\"t\", \"c\", \"DEFAULT\", 1",
        "\"t\", \"c\", DEFAULT(), 1",
    ] {
        let output = parse_line(
            &format!("DT_COLUMN_OPTIONS {tail}"),
            &DefaultParserContext::default(),
        );
        assert!(output.has_errors(), "{tail}");
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("DT_COLUMN_OPTIONS"))
        );
    }
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
fn plain_assignment_trims_decoded_ascii_edges_and_preserves_utf8_span() {
    let source = "RESULTS:0 = \t\\s\\tvalue inner　\\S\\s \t";
    let output = parse_line(source, &DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let statement = output.value.unwrap();
    let StatementKind::Assignment {
        raw_value, value, ..
    } = statement.kind
    else {
        panic!("expected assignment");
    };
    assert_eq!(raw_value, "\\s\\tvalue inner　\\S\\s");
    let ExprKind::Formatted(form) = value.kind else {
        panic!("expected FORM assignment");
    };
    assert_eq!(form.parts, vec![FormPart::Text("value inner　　".into())]);
    let expected_start = source.find("\\s").unwrap();
    assert_eq!(
        value.span,
        Span::new(expected_start, expected_start + raw_value.len())
    );
    assert_eq!(statement.span, Span::new(0, source.len()));
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
fn string_input_parses_one_form_default_then_expression_flags() {
    let output = parse_line(
        "ONEINPUTS value=%RESULTS%, 1, 0",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
        panic!("expected ONEINPUTS instruction");
    };
    assert!(matches!(
        arguments.as_slice(),
        [
            Argument::Formatted(_),
            Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::Integer(1),
                ..
            }),
            Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::Integer(0),
                ..
            })
        ]
    ));
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
fn plain_print_consumes_only_its_first_separator() {
    for (source, expected) in [("PRINT  text", " text"), ("PRINT \u{3000}", "\u{3000}")] {
        let output = parse_line(source, &DefaultParserContext::default());
        assert!(!output.has_errors(), "{:#?}", output.diagnostics);
        let StatementKind::Instruction {
            arguments,
            raw_arguments,
            ..
        } = output.value.unwrap().kind
        else {
            panic!("expected instruction");
        };
        assert_eq!(raw_arguments, expected);
        assert_eq!(arguments, vec![Argument::Raw(expected.into())]);
    }
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
fn continuation_delimiters_allow_trailing_horizontal_whitespace() {
    let source = "@TEST\n{\t\nPRINTFORM value={LOCAL}\n}\t \nRETURN\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    assert_eq!(output.value.unwrap().functions[0].body.len(), 2);
}

#[test]
fn continuation_spans_map_back_to_physical_utf8_offsets() {
    struct TwoByteSeparatorContext(DefaultParserContext);

    impl ParserContext for TwoByteSeparatorContext {
        fn lexer_config(&self) -> &erabasic_lexer::LexerConfig {
            self.0.lexer_config()
        }

        fn macros(&self) -> &erabasic_lexer::MacroTable {
            self.0.macros()
        }

        fn macros_mut(&mut self) -> &mut erabasic_lexer::MacroTable {
            self.0.macros_mut()
        }

        fn instruction(&self, name: &str) -> Option<InstructionSpec> {
            self.0.instruction(name)
        }

        fn continuation_separator(&self) -> &'static str {
            " \t"
        }
    }

    let source = "@TEST\n{\nRESULT += 1\n    + 2\n    + 3\n}\n";
    let output = parse_erb(
        source,
        &mut TwoByteSeparatorContext(DefaultParserContext::default()),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let statement = &output.value.unwrap().functions[0].body[0];
    let StatementKind::Assignment { target, value, .. } = &statement.kind else {
        panic!("expected assignment");
    };
    assert_eq!(statement.span.start, source.find("RESULT").unwrap());
    assert_eq!(statement.span.end, source.find("}\n").unwrap());
    assert_eq!(target.span.start, source.find("RESULT").unwrap());
    assert_eq!(value.span.start, source.find('1').unwrap());
    assert_eq!(value.span.end, source.rfind('3').unwrap() + 1);
    assert!(statement.span.end <= source.len());
}

#[test]
fn instruction_spec_is_reused_for_argument_parsing() {
    struct CountingContext {
        inner: DefaultParserContext,
        instruction_calls: std::cell::Cell<usize>,
    }

    impl ParserContext for CountingContext {
        fn lexer_config(&self) -> &erabasic_lexer::LexerConfig {
            self.inner.lexer_config()
        }

        fn macros(&self) -> &erabasic_lexer::MacroTable {
            self.inner.macros()
        }

        fn macros_mut(&mut self) -> &mut erabasic_lexer::MacroTable {
            self.inner.macros_mut()
        }

        fn instruction(&self, name: &str) -> Option<InstructionSpec> {
            self.instruction_calls.set(self.instruction_calls.get() + 1);
            self.inner.instruction(name)
        }
    }

    let context = CountingContext {
        inner: DefaultParserContext::default(),
        instruction_calls: std::cell::Cell::new(0),
    };
    let output = parse_line("PRINTFORMW text", &context);

    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    assert_eq!(context.instruction_calls.get(), 1);
}

#[test]
fn standalone_carriage_return_starts_a_new_physical_line() {
    let source = "@SYSTEM_TITLE\nIF 1\nELSE\r      IF 1\nPRINTL nested\nENDIF\nENDIF\nRETURN\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    assert_eq!(output.value.unwrap().functions[0].body.len(), 7);
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
fn if_ndebug_keeps_the_release_branch() {
    let source = "[IF_NDEBUG]\n@RELEASE\nRETURN\n[ELSE]\n@DEBUG\nRETURN\n[ENDIF]\n";
    let output = parse_erb(source, &mut DefaultParserContext::default());
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let script = output.value.unwrap();
    assert_eq!(script.functions.len(), 1);
    assert_eq!(script.functions[0].name, "RELEASE");
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
fn callstr_family_uses_one_expression_not_formatted_target() {
    for name in [
        "CALLSTR",
        "JUMPSTR",
        "TRYCALLSTR",
        "TRYJUMPSTR",
        "TRYCCALLSTR",
        "TRYCJUMPSTR",
    ] {
        let output = parse_line(
            &format!(r#"{name} "TARGET(1)" + ARGS:0"#),
            &DefaultParserContext::default(),
        );
        assert!(!output.has_errors(), "{name}: {:#?}", output.diagnostics);
        let StatementKind::Instruction { arguments, .. } = output.value.unwrap().kind else {
            panic!("expected instruction");
        };
        assert!(matches!(
            arguments.as_slice(),
            [Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::Binary {
                    op: BinaryOp::Add,
                    ..
                },
                ..
            })]
        ));
    }
}

#[test]
fn callstr_family_rejects_missing_or_separated_outer_arguments() {
    for name in [
        "CALLSTR",
        "JUMPSTR",
        "TRYCALLSTR",
        "TRYJUMPSTR",
        "TRYCCALLSTR",
        "TRYCJUMPSTR",
    ] {
        for tail in ["", r#""TARGET", 1"#] {
            let output = parse_line(&format!("{name} {tail}"), &DefaultParserContext::default());
            assert!(output.has_errors(), "{name} {tail}");
        }
    }
}

#[test]
fn callstr_try_catch_variants_open_script_blocks() {
    for name in ["TRYCCALLSTR", "TRYCJUMPSTR"] {
        let output = parse_erb(
            &format!("@TEST\n{name} \"TARGET()\"\nCATCH\nPRINTL caught\nENDCATCH\nRETURN\n"),
            &mut DefaultParserContext::default(),
        );
        assert!(!output.has_errors(), "{name}: {:#?}", output.diagnostics);
        let unterminated = parse_erb(
            &format!("@TEST\n{name} \"TARGET()\"\nCATCH\nRETURN\n"),
            &mut DefaultParserContext::default(),
        );
        assert!(
            unterminated
                .diagnostics
                .iter()
                .any(|item| { item.code == DiagnosticCode::UnmatchedBlock })
        );
    }
}

#[test]
fn call_text_accepts_both_syntaxes_and_preserves_nested_arguments() {
    for source in [
        r#"TARGET(OTHER(1, 2), "a,(b)", VALUES:INDEX(3))"#,
        r#"TARGET, OTHER(1, 2), "a,(b)", VALUES:INDEX(3)"#,
    ] {
        let output = parse_call_text_at(source, 0, &DefaultParserContext::default()).unwrap();
        assert!(output.diagnostics.is_empty());
        let call = output.call.unwrap();
        assert_eq!(call.target, "TARGET");
        assert_eq!(call.arguments.len(), 3);
        assert!(matches!(
            &call.arguments[0],
            Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::Call { name, args },
                ..
            }) if name == "OTHER" && args.len() == 2
        ));
        assert!(matches!(
            &call.arguments[1],
            Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::String(value),
                ..
            }) if value == "a,(b)"
        ));
        assert!(matches!(
            &call.arguments[2],
            Argument::Expression(erabasic_ast::Expr {
                kind: ExprKind::Variable { name, .. },
                ..
            }) if name == "VALUES"
        ));
    }
}

#[test]
fn call_text_noop_is_only_whitespace() {
    let context = DefaultParserContext::default();
    for source in ["", " \t\r\n", "　"] {
        assert!(
            parse_call_text_at(source, 0, &context)
                .unwrap()
                .call
                .is_none()
        );
    }
    for source in ["TARGET", "TARGET()", "TARGET,", "TARGET ; comment"] {
        let call = parse_call_text_at(source, 0, &context)
            .unwrap()
            .call
            .unwrap();
        assert_eq!(call.target, "TARGET");
        assert!(call.arguments.is_empty());
    }
    let comment = parse_call_text_at("; comment", 0, &context)
        .unwrap()
        .call
        .unwrap();
    assert_eq!(comment.target, "");
    // Empty-target lookup belongs to execution; it must not become a no-op here.
}

#[test]
fn call_text_preserves_omissions_without_trailing_extra_slot() {
    for source in ["TARGET(, 2,,)", "TARGET, , 2,,"] {
        let call = parse_call_text_at(source, 0, &DefaultParserContext::default())
            .unwrap()
            .call
            .unwrap();
        assert!(matches!(
            call.arguments.as_slice(),
            [
                Argument::Omitted(_),
                Argument::Expression(erabasic_ast::Expr {
                    kind: ExprKind::Integer(2),
                    ..
                }),
                Argument::Omitted(_)
            ]
        ));
    }
}

#[test]
fn call_text_lexical_errors_precede_argument_reduction() {
    let context = DefaultParserContext::default();
    for source in [r#"TARGET(1 +, "unterminated)"#, "TARGET(1 +", "TARGET\\"] {
        let error = parse_call_text_at(source, 0, &context).unwrap_err();
        assert_eq!(error.stage, CallTextParseStage::Lexical, "{source}");
        assert!(!error.diagnostics.is_empty());
    }
    for source in ["TARGET(1 +)", "TARGET(+)", "TARGET, 1 2"] {
        let error = parse_call_text_at(source, 0, &context).unwrap_err();
        assert_eq!(error.stage, CallTextParseStage::Arguments, "{source}");
        assert!(!error.diagnostics.is_empty());
    }
}

#[test]
fn call_text_keeps_utf8_argument_and_omission_spans() {
    let source = " 函数(, \"文字\",, )";
    let base = 70;
    let call = parse_call_text_at(source, base, &DefaultParserContext::default())
        .unwrap()
        .call
        .unwrap();
    assert_eq!(call.target, "函数");
    assert_eq!(
        call.target_span,
        Span::new(base, base + source.find('(').unwrap())
    );
    let Argument::Expression(expression) = &call.arguments[1] else {
        panic!("expected string expression");
    };
    assert_eq!(
        &source[expression.span.start - base..expression.span.end - base],
        "\"文字\""
    );
    for argument in &call.arguments {
        let span = match argument {
            Argument::Expression(expression) => expression.span,
            Argument::Omitted(span) => *span,
            _ => panic!("runtime call text only yields expression/omitted slots"),
        };
        assert!(source.is_char_boundary(span.start - base));
        assert!(source.is_char_boundary(span.end - base));
    }
}

#[test]
fn call_text_uses_caller_lexer_configuration_and_macros() {
    let mut context = DefaultParserContext::default();
    let error = parse_call_text_at("TARGET(1　+ 2)", 0, &context).unwrap_err();
    assert_eq!(error.stage, CallTextParseStage::Lexical);
    context.set_lexer_compatibility(true, false, false);
    assert!(parse_call_text_at("TARGET(1　+ 2)", 0, &context).is_ok());
    let error = parse_call_text_at("TARGET(1;#;+2)", 0, &context).unwrap_err();
    assert_eq!(error.stage, CallTextParseStage::Lexical);
    context.set_lexer_compatibility(true, true, false);
    assert!(parse_call_text_at("TARGET(1;#;+2)", 0, &context).is_ok());
    context.macros_mut().insert(
        "VALUE".into(),
        erabasic_lexer::lex("3", &erabasic_lexer::LexerConfig::default()).tokens,
    );
    let call = parse_call_text_at("VALUE(VALUE)", 0, &context)
        .unwrap()
        .call
        .unwrap();
    assert_eq!(call.target, "VALUE");
    assert!(matches!(
        call.arguments.as_slice(),
        [Argument::Expression(erabasic_ast::Expr {
            kind: ExprKind::Integer(3),
            ..
        })]
    ));
}

#[test]
fn call_text_target_escapes_are_not_form_interpolation() {
    let context = DefaultParserContext::default();
    let call = parse_call_text_at(r"  F\sN\(X(1)", 0, &context)
        .unwrap()
        .call
        .unwrap();
    assert_eq!(call.target, "F N(X");
    let call = parse_call_text_at("F_{VALUE}(1)", 0, &context)
        .unwrap()
        .call
        .unwrap();
    assert_eq!(call.target, "F_{VALUE}");
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

#[test]
fn dynamic_call_target_modulo_does_not_swallow_parenthesized_arguments() {
    let output = parse_line(
        "TRYCCALLFORM IRAI_一般{IRAI_ID % 1000}(CHARA, IRAI_ID, SCENE)",
        &DefaultParserContext::default(),
    );
    assert!(!output.has_errors(), "{:#?}", output.diagnostics);
    let statement = output.value.expect("dynamic call statement");
    let StatementKind::Instruction {
        name, arguments, ..
    } = statement.kind
    else {
        panic!("expected instruction");
    };
    assert_eq!(name, "TRYCCALLFORM");
    assert_eq!(arguments.len(), 4);
    let Argument::Formatted(target) = &arguments[0] else {
        panic!("expected formatted target");
    };
    assert!(matches!(
        target.parts.as_slice(),
        [
            FormPart::Text(prefix),
            FormPart::IntegerInterpolation { expression, .. }
        ] if prefix == "IRAI_一般"
            && matches!(
                expression.kind,
                ExprKind::Binary {
                    op: BinaryOp::Modulo,
                    ..
                }
            )
    ));
    assert_eq!(statement.span, erabasic_ast::Span::new(0, 63));
}
