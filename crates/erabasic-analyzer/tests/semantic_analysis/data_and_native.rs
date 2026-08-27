use super::*;

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
