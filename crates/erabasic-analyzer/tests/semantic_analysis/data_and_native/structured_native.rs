use super::*;

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
