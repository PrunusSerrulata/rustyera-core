use super::*;

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
