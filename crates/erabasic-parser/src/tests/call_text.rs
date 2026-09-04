use crate::{
    CallTextParseStage, DefaultParserContext, ParserContext, parse_call_text_at, parse_erb,
    parse_line,
};
use erabasic_ast::{Argument, BinaryOp, DiagnosticCode, ExprKind, FormPart, Span, StatementKind};

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
                .any(|item| item.code == DiagnosticCode::UnmatchedBlock)
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
    assert_eq!(statement.span, Span::new(0, 63));
}
