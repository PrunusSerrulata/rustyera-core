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

#[test]
fn bitmap_cache_enable_requires_the_reference_argument() {
    for (call, accepted) in [
        ("BITMAP_CACHE_ENABLE()", false),
        ("BITMAP_CACHE_ENABLE(1)", true),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "bitmap-cache.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {call}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        let has_argument_error = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount);
        assert_eq!(
            has_argument_error, !accepted,
            "{call}: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn csv_name_tables_resolve_identifier_indices() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    source_path: None,
                    relative_path: "ABL.csv".into(),
                    payload: CsvFilePayload::Utf8("2,later\n".into()),
                },
                FrontendFile {
                    source_path: None,
                    relative_path: "BASE.csv".into(),
                    payload: CsvFilePayload::Utf8("0,health\n".into()),
                },
            ],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .unwrap();
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![source(
                "named-index.erb",
                "@SYSTEM_TITLE\nRESULT = ABL:later\nRESULT = MAXBASE:health\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let function = &report.project.unwrap().program.functions[0];
    let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
        panic!("expected assignment");
    };
    let erabasic_hir::HirExprKind::Variable { place } = &value.kind else {
        panic!("expected indexed variable");
    };
    assert_eq!(
        place.indices[0].constant,
        Some(erabasic_hir::ConstantValue::Integer(2))
    );
}

#[test]
fn getnum_folds_only_pure_builtin_name_table_lookups() {
    let mut project_data = empty_project();
    project_data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cflag)
        .unwrap()
        .lookup
        .insert("known".into(), 17);
    project_data
        .static_data
        .name_tables
        .get_mut(&erabasic_data::NameTableKind::Cdflag2)
        .unwrap()
        .lookup
        .insert("second".into(), 23);
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![
                source(
                    "constants.erh",
                    "#DIMS CONST LOOKUP_KEY = \"known\"\n#DIM CONST ONE = 1\n",
                ),
                source(
                    "getnum.erb",
                    "@SYSTEM_TITLE\n\
                     RESULT = GETNUM(CFLAG, LOOKUP_KEY + \"\")\n\
                     RESULT = GETNUM(CFLAG, \"known\", 0)\n\
                     RESULT = GETNUM(CFLAG, \"known\", 1)\n\
                     RESULT = GETNUM(CFLAG, \"missing\")\n\
                     RESULT = GETNUM(CDFLAG, \"second\", +(ONE + (1 ? 1 # 0)))\n\
                     RESULT = GETNUM(CFLAG, \"known\", -ONE)\n\
                     RESULT = GETNUM(CFLAG, RESULTS)\n\
                     RESULT = GETNUM(CFLAG:0, \"known\")\n\
                     #DIM DYNAMIC LOCAL_TABLE, 1\n\
                     RESULT = GETNUM(LOCAL_TABLE, \"known\")\n\
                     RETURN\n",
                ),
            ],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.expect("GETNUM source should analyze");
    let values = project.program.functions[0]
        .lines
        .iter()
        .filter_map(|line| match &line.kind {
            HirStatementKind::Assignment { value, .. } => Some(value),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 9);
    for (index, expected) in [(0, 17), (1, 17), (2, 17), (3, -1), (4, 23), (5, -1)] {
        assert!(
            matches!(
                &values[index].kind,
                erabasic_hir::HirExprKind::Integer { value } if *value == expected
            ),
            "unexpected folded value at {index}: {:#?}",
            values[index]
        );
    }
    for value in values.iter().skip(6) {
        assert!(
            matches!(&value.kind, erabasic_hir::HirExprKind::Call { .. }),
            "dynamic, indexed, and local lookups must remain calls: {value:#?}"
        );
    }
}

#[test]
fn user_getnum_method_is_not_folded_as_the_builtin() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "shadow.erb",
                "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"known\")\nRETURN\n\
                 @GETNUM(ARG, ARGS)\n#FUNCTION\nRETURNF 99\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.expect("shadowing method should analyze");
    let HirStatementKind::Assignment { value, .. } = &project.program.functions[0].lines[0].kind
    else {
        panic!("expected assignment");
    };
    assert!(matches!(
        &value.kind,
        erabasic_hir::HirExprKind::Call {
            target: erabasic_hir::CallTarget::User { .. },
            ..
        }
    ));
}

#[test]
fn getnum_fold_does_not_bypass_argument_diagnostics() {
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "invalid-getnum.erb",
                "@SYSTEM_TITLE\nRESULT = GETNUM(CFLAG, \"missing\", 0, UNKNOWN_VALUE)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount })
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::UnknownIdentifier })
    );
}

#[test]
fn str_indices_resolve_through_strname_instead_of_initial_values() {
    let project_data = load_project(
        &ProjectFiles {
            csv: vec![
                FrontendFile {
                    source_path: None,
                    relative_path: "STR.csv".into(),
                    payload: CsvFilePayload::Utf8("0,initial text\n".into()),
                },
                FrontendFile {
                    source_path: None,
                    relative_path: "STRNAME.csv".into(),
                    payload: CsvFilePayload::Utf8("7,named_slot\n".into()),
                },
            ],
            erb: Vec::new(),
        },
        &CsvLoadOptions::default(),
    )
    .data
    .unwrap();
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![source(
                "strname.erb",
                "@SYSTEM_TITLE\nSTR:named_slot = \"updated\"\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let function = &report.project.unwrap().program.functions[0];
    let HirStatementKind::Assignment { target, .. } = &function.lines[0].kind else {
        panic!("expected STR assignment");
    };
    assert_eq!(
        target.indices[0].constant,
        Some(erabasic_hir::ConstantValue::Integer(7))
    );
}

#[test]
fn user_character_data_requires_binary_save_configuration() {
    let input = AnalysisInput {
        project_data: empty_project(),
        sources: vec![source(
            "character.erh",
            "#DIM CHARADATA CUSTOM_CHARACTER, 10\n",
        )],
    };
    let text = analyze_project(
        input.clone(),
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(text.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == AnalyzerDiagnosticCode::InvalidDeclaration
            && diagnostic.message.contains("require binary saves")
    }));

    let mut binary_options = AnalyzerOptions::analysis_mode();
    binary_options.system_save_in_binary = true;
    let binary = analyze_project(input, &binary_options, &ExtensionRegistry::default());
    assert!(
        !binary
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        binary.diagnostics
    );
}

#[test]
fn structured_native_signatures_require_mutable_array_outputs() {
    let valid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "structured.erb",
                "@SYSTEM_TITLE\n#DIMS KEYS, 4\nRESULTS '= MAP_GETKEYS(\"m\", KEYS, 1)\nRESULT = DT_COLUMN_NAMES(\"t\", KEYS)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !valid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        valid.diagnostics
    );

    let invalid = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "structured-invalid.erb",
                "@SYSTEM_TITLE\nRESULTS '= MAP_GETKEYS(\"m\", \"not a place\", 1)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidAssignment }),
        "{:#?}",
        invalid.diagnostics
    );
}

#[test]
fn dynamic_method_signatures_validate_name_fallback_and_existence_arity() {
    for (expression, expected) in [
        (
            "GETMETH()",
            Some(AnalyzerDiagnosticCode::InvalidArgumentCount),
        ),
        (
            "GETMETH(, 1)",
            Some(AnalyzerDiagnosticCode::InvalidArgument),
        ),
        ("GETMETH(1)", Some(AnalyzerDiagnosticCode::TypeMismatch)),
        (
            "GETMETH(\"M\", \"wrong\")",
            Some(AnalyzerDiagnosticCode::TypeMismatch),
        ),
        (
            "GETMETHS(\"M\", 1)",
            Some(AnalyzerDiagnosticCode::TypeMismatch),
        ),
        (
            "EXISTMETH()",
            Some(AnalyzerDiagnosticCode::InvalidArgumentCount),
        ),
        (
            "EXISTMETH(\"M\", 1)",
            Some(AnalyzerDiagnosticCode::InvalidArgumentCount),
        ),
        ("EXISTMETH(1)", Some(AnalyzerDiagnosticCode::TypeMismatch)),
        ("GETMETH(\"M\")", None),
        ("GETMETH(\"M\",, FLAG:1,, 3)", None),
        ("EXISTMETH(\"M\")", None),
    ] {
        for statement in [
            format!("RESULT = {expression}"),
            expression
                .replacen('(', " ", 1)
                .strip_suffix(')')
                .unwrap()
                .to_owned(),
        ] {
            let report = analyze_project(
                AnalysisInput {
                    project_data: empty_project(),
                    sources: vec![source(
                        "methods.erb",
                        &format!("@SYSTEM_TITLE\n{statement}\nRETURN\n"),
                    )],
                },
                &AnalyzerOptions::analysis_mode(),
                &ExtensionRegistry::default(),
            );
            if let Some(expected) = expected {
                assert!(
                    report
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == expected),
                    "{expression}: {:#?}",
                    report.diagnostics
                );
            } else {
                assert!(
                    !report
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.reference_level >= 2),
                    "{expression}: {:#?}",
                    report.diagnostics
                );
            }
        }
    }
}

#[test]
fn dynamic_methods_preserve_omitted_slots_and_variables_without_constant_folding() {
    use erabasic_hir::{HirCallArgument, HirExprKind};
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "methods.erb",
                "@SYSTEM_TITLE\nRESULT = GETMETH(\"M\",, FLAG:1,, -9223372036854775807 - 1)\nRESULTS '= GETMETHS(\"S\", \"fallback\", STR:2)\nRETURN\n",
            )],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2),
        "{:#?}",
        report.diagnostics
    );
    let project = report.project.expect("valid dynamic method expressions");
    let function = &project.program.functions[0];
    let HirStatementKind::Assignment { value, .. } = &function.lines[0].kind else {
        panic!("assignment");
    };
    let HirExprKind::Call { arguments, .. } = &value.kind else {
        panic!("method call");
    };
    assert!(value.constant.is_none());
    assert_eq!(arguments.len(), 5);
    assert!(matches!(arguments[1], HirCallArgument::Omitted));
    let HirCallArgument::Place(place) = &arguments[2] else {
        panic!("retained variable");
    };
    assert_eq!(place.indices.len(), 1);
    assert!(matches!(arguments[3], HirCallArgument::Omitted));
    let HirCallArgument::Value(minimum) = &arguments[4] else {
        panic!("present integer");
    };
    assert_eq!(
        minimum.constant,
        Some(erabasic_hir::ConstantValue::Integer(i64::MIN))
    );
    let HirStatementKind::Assignment { value, .. } = &function.lines[1].kind else {
        panic!("string assignment");
    };
    let HirExprKind::Call { arguments, .. } = &value.kind else {
        panic!("string method call");
    };
    assert_eq!(value.value_type, SemanticType::String);
    assert!(matches!(arguments[1], HirCallArgument::Value(_)));
    assert!(matches!(arguments[2], HirCallArgument::Place(_)));
}

#[test]
fn xml_replace_stored_key_overload_preserves_inline_mutability_rules() {
    for (operands, accepted) in [
        ("\"doc\", \"<root/>\"", true),
        ("1, \"<root/>\"", true),
        ("XML_SOURCE, \"<root/>\"", true),
        ("\"<root/>\", \"/root\", \"<next/>\"", false),
        ("XML_SOURCE, \"/root\", \"<next/>\"", true),
    ] {
        for statement in [true, false] {
            let call = if statement {
                format!("XML_REPLACE {operands}")
            } else {
                format!("RESULT = XML_REPLACE({operands})")
            };
            let report = analyze_project(
                AnalysisInput {
                    project_data: empty_project(),
                    sources: vec![source(
                        "xml-replace.erb",
                        &format!("@SYSTEM_TITLE\n#DIMS XML_SOURCE\n{call}\nRETURN\n"),
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
                report.diagnostics
            );
            if accepted && operands.starts_with("XML_SOURCE,") && !operands.contains("/root") {
                let project = report.project.unwrap();
                let line = &project.program.functions[0].lines[0].kind;
                let kept_value = match line {
                    HirStatementKind::Instruction { arguments, .. } => {
                        matches!(arguments[0], erabasic_hir::HirArgument::Expression(_))
                    }
                    HirStatementKind::Assignment { value, .. } => {
                        matches!(&value.kind, erabasic_hir::HirExprKind::Call { arguments, .. } if matches!(arguments[0], erabasic_hir::HirCallArgument::Value(_)))
                    }
                    _ => false,
                };
                assert!(
                    kept_value,
                    "stored key is a value, not an inline XML writeback place"
                );
            }
        }
    }
}

#[test]
fn html_queries_require_reference_types_and_argument_counts() {
    for (call, string_result, accepted) in [
        ("HTML_STRINGLEN(\"x\")", false, true),
        ("HTML_STRINGLEN(\"x\", 1)", false, true),
        ("HTML_STRINGLEN()", false, false),
        ("HTML_STRINGLEN(1)", false, false),
        ("HTML_STRINGLEN(\"x\", \"pixels\")", false, false),
        ("HTML_STRINGLEN(\"x\", 1, 2)", false, false),
        ("HTML_STRINGLINES(\"x\", 2)", false, true),
        ("HTML_STRINGLINES(\"x\")", false, false),
        ("HTML_STRINGLINES(1, 2)", false, false),
        ("HTML_SUBSTRING(\"x\", 2)", true, true),
        ("HTML_SUBSTRING(\"x\")", true, false),
        ("HTML_SUBSTRING(\"x\", \"2\")", true, false),
    ] {
        let assignment = if string_result {
            "RESULTS '= "
        } else {
            "RESULT = "
        };
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "html-query.erb",
                    &format!("@SYSTEM_TITLE\n{assignment}{call}\nRETURN\n"),
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
            report.diagnostics
        );
    }
}

#[test]
fn getcsvno_methods_are_snake_only_and_keep_exact_builtin_arity() {
    for name in [
        "GETCSVNOBYNAME",
        "GETCSVNOBYCALLNAME",
        "GETCSVNOBYNICKNAME",
        "GETCSVNOBYMASTERNAME",
    ] {
        for (arguments, expected) in [
            ("\"name\"", None),
            ("", Some(AnalyzerDiagnosticCode::InvalidArgumentCount)),
            (
                "\"name\", \"extra\"",
                Some(AnalyzerDiagnosticCode::InvalidArgumentCount),
            ),
            ("42", Some(AnalyzerDiagnosticCode::TypeMismatch)),
        ] {
            for profile in [
                erabasic_compat::CompatibilityProfileId::EmueraEm,
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ] {
                let report = analyze_project(
                    AnalysisInput {
                        project_data: empty_project(),
                        sources: vec![source(
                            "getcsvno.erb",
                            &format!("@SYSTEM_TITLE\nRESULT = {name}({arguments})\nRETURN\n"),
                        )],
                    },
                    &AnalyzerOptions {
                        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                        ..AnalyzerOptions::analysis_mode()
                    },
                    &ExtensionRegistry::default(),
                );
                let expected = if profile == erabasic_compat::CompatibilityProfileId::EmueraEm {
                    Some(AnalyzerDiagnosticCode::UnknownFunction)
                } else {
                    expected
                };
                if let Some(expected) = expected {
                    assert!(
                        report
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.code == expected),
                        "{profile} {name}({arguments}): {:?}",
                        report.diagnostics
                    );
                } else {
                    assert!(
                        !report
                            .diagnostics
                            .iter()
                            .any(|diagnostic| diagnostic.reference_level >= 2),
                        "{name}: {:?}",
                        report.diagnostics
                    );
                }
            }
        }
    }
}

#[test]
fn snake_graphics_functions_keep_profile_specific_names_and_overloads() {
    let valid_snake = [
        "SPRITECREATE(\"S\", 1)",
        "SPRITECREATE(\"S\", 1, 0, 0, 2, 1)",
        "SPRITECREATE(\"S\", 1, 0, 0, 2, 1, -3, 4)",
        "SPRITECREATE(\"S\", 1, 0, 0, 2, 1, -3, 4, -7, -9)",
        "SPRITECREATEFROMFILE(\"S\", \"image.png\")",
        "SPRITECREATEFROMFILE(\"S\", \"image.png\", 1)",
        "G_POLYGON_POINT_ADD(1, 2, 3)",
        "G_POLYGON_DRAW(1)",
        "G_POLYGON_FILL(1)",
        "G_POLYGON_POINT_CLEAR(1)",
    ];
    for call in valid_snake {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "graphics.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {call}\nRETURN\n"),
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
            !report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.reference_level >= 2),
            "{call}: {:?}",
            report.diagnostics,
        );
    }

    for (profile, call, expected) in [
        (
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            "SPRITECREATE(\"S\", 1, 2)",
            AnalyzerDiagnosticCode::InvalidArgumentCount,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraEm,
            "SPRITECREATE(\"S\", 1, 0, 0, 2, 1, 3, 4)",
            AnalyzerDiagnosticCode::InvalidArgumentCount,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraEm,
            "SPRITECREATEFROMFILE(\"S\", \"image.png\")",
            AnalyzerDiagnosticCode::UnknownFunction,
        ),
        (
            erabasic_compat::CompatibilityProfileId::EmueraEm,
            "G_POLYGON_DRAW(1)",
            AnalyzerDiagnosticCode::UnknownFunction,
        ),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "graphics-invalid.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {call}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                ..AnalyzerOptions::analysis_mode()
            },
            &ExtensionRegistry::default(),
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == expected),
            "{call}: {:?}",
            report.diagnostics,
        );
    }
}

#[test]
fn file_sprite_only_allows_its_trailing_argument_to_be_omitted() {
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    for (body, invalid) in [
        (
            "RESULT = SPRITECREATEFROMFILE(\"S\", \"image.png\", )",
            false,
        ),
        ("SPRITECREATEFROMFILE \"S\", \"image.png\",", false),
        ("RESULT = SPRITECREATEFROMFILE(, \"image.png\", 1)", true),
        ("RESULT = SPRITECREATEFROMFILE(\"S\", , 1)", true),
        ("SPRITECREATEFROMFILE , \"image.png\", 1", true),
        ("SPRITECREATEFROMFILE \"S\", , 1", true),
    ] {
        let text = format!("@SYSTEM_TITLE\n{body}\nRETURN\n");
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source("graphics-omitted.erb", &text)],
            },
            &options,
            &ExtensionRegistry::default(),
        );
        let has_invalid_argument = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == AnalyzerDiagnosticCode::InvalidArgument);
        let has_error = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.reference_level >= 2);
        assert_eq!(
            has_invalid_argument, invalid,
            "{body}: {:?}",
            report.diagnostics
        );
        assert_eq!(has_error, invalid, "{body}: {:?}", report.diagnostics);
    }
}

#[test]
fn bit_apis_require_snake_mutable_integer_rank_one_tokens_in_both_call_forms() {
    for statement in [false, true] {
        for (declaration, token, accepted) in [
            ("#DIM WORDS, 2", "WORDS", true),
            ("#DIM WORDS, 2, 2", "WORDS", false),
            ("#DIMS WORDS, 2", "WORDS", false),
            ("#DIM CONST WORDS, 2 = 1, 2", "WORDS", false),
            ("#DIM WORDS, 2", "1 + 2", false),
        ] {
            let call = if statement {
                format!("BITGET {token}, 0")
            } else {
                format!("RESULT = BITGET({token}, 0)")
            };
            let text = format!("@SYSTEM_TITLE\n{declaration}\n{call}\nRETURN\n");
            let options = AnalyzerOptions {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                    erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
                ),
                ..AnalyzerOptions::analysis_mode()
            };
            let report = analyze_project(
                AnalysisInput {
                    project_data: empty_project(),
                    sources: vec![source("bit-shape.erb", &text)],
                },
                &options,
                &ExtensionRegistry::default(),
            );
            let invalid = report.diagnostics.iter().any(|d| {
                matches!(
                    d.code,
                    AnalyzerDiagnosticCode::InvalidArgument | AnalyzerDiagnosticCode::TypeMismatch
                )
            });
            assert_eq!(invalid, !accepted, "{text}: {:?}", report.diagnostics);
        }
    }
    let text = "@SYSTEM_TITLE\nRESULT = BITSET(FLAG, 0)\nRETURN\n";
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source("bit-original.erb", text)],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == AnalyzerDiagnosticCode::UnknownFunction)
    );
}

#[test]
fn bit_discarded_token_indices_still_receive_source_name_validation() {
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    let report = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "bit-index.erb",
                "@SYSTEM_TITLE\nRESULT = BITGET(FLAG:MISSING(), 0)\nRETURN\n",
            )],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == AnalyzerDiagnosticCode::UnknownFunction)
    );
}

#[test]
fn matchallex_checks_source_literal_shape_before_constant_folding() {
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    for (first, accepted) in [
        ("\"FLAG\"", true),
        ("(\"FLAG\")", true),
        ("\"FL\" + \"AG\"", false),
        ("S", false),
    ] {
        for statement in [false, true] {
            let call = if statement {
                format!("MATCHALLEX {first}, 0")
            } else {
                format!("RESULT = MATCHALLEX({first}, 0)")
            };
            let text = format!("@SYSTEM_TITLE\n#DIMS CONST S = \"FLAG\"\n{call}\nRETURN\n");
            let report = analyze_project(
                AnalysisInput {
                    project_data: empty_project(),
                    sources: vec![source("match-shape.erb", &text)],
                },
                &options,
                &ExtensionRegistry::default(),
            );
            let invalid = report
                .diagnostics
                .iter()
                .any(|d| d.code == AnalyzerDiagnosticCode::InvalidArgument);
            assert_eq!(
                invalid, !accepted,
                "{first}; statement={statement}: {:?}",
                report.diagnostics
            );
        }
    }
}

#[test]
fn match_apis_are_snake_only_and_do_not_relax_builtin_arity_or_token_names() {
    for expression in ["MATCHALL(FLAG, 0)", "MATCHALLEX(\"FLAG\", 0)"] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "match-original.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {expression}\nRETURN\n"),
                )],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == AnalyzerDiagnosticCode::UnknownFunction)
        );
    }
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    let omitted = analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "match-omitted.erb",
                "@SYSTEM_TITLE\nRESULT = MATCHALL(FLAG, 0, , , )\nRETURN\n",
            )],
        },
        &options,
        &ExtensionRegistry::default(),
    );
    assert!(omitted.diagnostics.is_empty(), "{:?}", omitted.diagnostics);
    for expression in [
        "MATCHALL(FLAG:MISSING(), 0)",
        "MATCHALLEX(\"FLAG\", 0, 0, 1, FLAG, 7)",
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "match-invalid.erb",
                    &format!("@SYSTEM_TITLE\nRESULT = {expression}\nRETURN\n"),
                )],
            },
            &options,
            &ExtensionRegistry::default(),
        );
        assert!(!report.diagnostics.is_empty(), "{expression}");
    }
}
