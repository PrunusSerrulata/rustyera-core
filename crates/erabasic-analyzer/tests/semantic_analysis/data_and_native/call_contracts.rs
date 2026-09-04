use super::*;

#[test]
fn snake_static_user_calls_report_excess_arguments_without_dropping_hir() {
    let text = "@SYSTEM_TITLE\nCALL TAKE, 1, SIDE()\nRESULT = METH(2, SIDE(), 9223372036854775807 + 1)\nRETURN\n@TAKE(ARG)\nRETURN\n@METH(ARG)\n#FUNCTION\nRETURNF ARG\n@SIDE\n#FUNCTION\nFLAG:0 += 1\nRETURNF 9\n";
    for strict in [false, true] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source("user-arity.erb", text)],
            },
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                    erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                ),
                strict_user_call_arguments: strict,
                ..AnalyzerOptions::analysis_mode()
            },
            &ExtensionRegistry::default(),
        );
        let arity = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::ExcessUserArguments)
            .collect::<Vec<_>>();
        assert_eq!(arity.len(), 2, "{:?}", report.diagnostics);
        assert!(arity.iter().all(|diagnostic| {
            diagnostic.reference_level == (if strict { 2 } else { 1 })
                && diagnostic.source.as_ref().is_some_and(|source| {
                    source.relative_path == "user-arity.erb" && source.byte_end > source.byte_start
                })
        }));
        assert!(!report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code,
                AnalyzerDiagnosticCode::IntegerOverflow
                    | AnalyzerDiagnosticCode::IntegerDivideByZero
            )
        }));
        let project = report.project.unwrap();
        let title = project
            .program
            .functions
            .iter()
            .find(|function| function.name == "SYSTEM_TITLE")
            .unwrap();
        let HirStatementKind::Instruction { arguments, .. } = &title.lines[0].kind else {
            panic!("expected CALL");
        };
        assert_eq!(
            arguments.len(),
            3,
            "target plus both actual ASTs must remain"
        );
        let HirStatementKind::Assignment { value, .. } = &title.lines[1].kind else {
            panic!("expected method assignment");
        };
        let erabasic_hir::HirExprKind::Call { arguments, .. } = &value.kind else {
            panic!("expected direct method");
        };
        assert_eq!(arguments.len(), 3);
    }
}

#[test]
fn ignored_user_call_actuals_still_check_names_and_builtin_arity() {
    for (actual, expected) in [
        (
            "MISSING_VARIABLE",
            AnalyzerDiagnosticCode::UnknownIdentifier,
        ),
        ("MISSING_METHOD()", AnalyzerDiagnosticCode::UnknownFunction),
        ("ABS(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        ("SIGN(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        ("SQRT(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        ("CBRT(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        ("LOG(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        ("LOG10(1, 2)", AnalyzerDiagnosticCode::InvalidArgumentCount),
        (
            "EXPONENT(1, 2)",
            AnalyzerDiagnosticCode::InvalidArgumentCount,
        ),
    ] {
        for statement in [
            format!("CALL TAKE, 1, {actual}"),
            format!("RESULT = METH(1, {actual})"),
        ] {
            let report = analyze_project(
                AnalysisInput {
                    project_data: empty_project(),
                    sources: vec![source(
                        "checked-tail.erb",
                        &format!(
                            "@SYSTEM_TITLE\n{statement}\nRETURN\n@TAKE(ARG)\nRETURN\n@METH(ARG)\n#FUNCTION\nRETURNF ARG\n"
                        ),
                    )],
                },
                &AnalyzerOptions {
                    compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                    ),
                    ..AnalyzerOptions::analysis_mode()
                },
                &ExtensionRegistry::default(),
            );
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == expected),
                "{statement}: {:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn unchecked_functions_require_exact_integer_signatures() {
    for (call, accepted) in [
        ("UNCHECKED_ADD(1, 2)", true),
        ("UNCHECKED_SUB(1, 2)", true),
        ("UNCHECKED_MUL(1, 2)", true),
        ("UNCHECKED_NEG(1)", true),
        ("UNCHECKED_ADD(1)", false),
        ("UNCHECKED_SUB(1, 2, 3)", false),
        ("UNCHECKED_MUL(\"1\", 2)", false),
        ("UNCHECKED_NEG()", false),
        ("UNCHECKED_NEG(1, 2)", false),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "unchecked.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {call}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        assert_eq!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            accepted,
            "{call}: {:?}",
            report.diagnostics,
        );
    }
}

#[test]
fn column_options_treat_default_as_syntax_and_require_typed_header_and_values() {
    for (tail, accepted) in [
        ("\"t\", \"c\", DEFAULT, 12, DEFAULT, \"value\"", true),
        ("1, \"c\", DEFAULT, 12", false),
        ("\"t\", 1, DEFAULT, 12", false),
        ("\"t\", \"c\", DEFAULT,", false),
        ("\"t\", \"c\", , 12", false),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "column-options.erb",
                    &format!("@SYSTEM_TITLE\nDT_COLUMN_OPTIONS {tail}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        assert_eq!(
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            accepted,
            "{tail}: {:?}",
            report.diagnostics
        );
        if accepted {
            let project = report.project.unwrap();
            let HirStatementKind::Instruction { arguments, .. } =
                &project.program.functions[0].lines[0].kind
            else {
                panic!("expected instruction")
            };
            assert!(
                matches!(&arguments[2], erabasic_hir::HirArgument::Raw(value) if value == "DEFAULT")
            );
        }
    }
}
