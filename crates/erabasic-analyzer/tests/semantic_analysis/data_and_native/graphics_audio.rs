use super::*;

#[test]
fn scene_graphics_omissions_and_cbg_sprite_arity_follow_the_selected_profile() {
    use erabasic_compat::CompatibilityProfileId::{EmueraEm, EmueraSkiaSnake};

    let original_short = graphics_diagnostics(EmueraEm, "RESULT = CBGSETSPRITE(\"A\")");
    assert!(
        original_short
            .iter()
            .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgumentCount })
    );
    let original_exact = graphics_diagnostics(EmueraEm, "RESULT = CBGSETSPRITE(\"A\", 0, 0, 1)");
    assert!(!original_exact.iter().any(|diagnostic| matches!(
        diagnostic.code,
        AnalyzerDiagnosticCode::InvalidArgumentCount | AnalyzerDiagnosticCode::InvalidArgument
    )));
    let snake_short = graphics_diagnostics(EmueraSkiaSnake, "RESULT = CBGSETSPRITE(\"A\")");
    assert!(!snake_short.iter().any(|diagnostic| matches!(
        diagnostic.code,
        AnalyzerDiagnosticCode::InvalidArgumentCount | AnalyzerDiagnosticCode::InvalidArgument
    )));
    for statement in [
        "SETIMAGELAYER , 1",
        "SETIMAGELAYERL \"A\",",
        "RESULT = CBGSETBUTTONSPRITE(1, , \"H\", 0, 0, 1)",
    ] {
        let diagnostics = graphics_diagnostics(EmueraSkiaSnake, statement);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgument }),
            "{statement}: {diagnostics:#?}"
        );
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
fn playsound_accepts_one_or_two_values_but_not_an_omitted_resource() {
    let options = AnalyzerOptions {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        ..AnalyzerOptions::analysis_mode()
    };
    for (statement, invalid) in [
        ("PLAYSOUND \"tone.wav\"", false),
        ("PLAYSOUND \"tone.wav\", 3", false),
        ("PLAYSOUND , 3", true),
    ] {
        let report = analyze_project(
            AnalysisInput {
                project_data: empty_project(),
                sources: vec![source(
                    "playsound-signature.erb",
                    &format!("@SYSTEM_TITLE\n{statement}\nRETURN\n"),
                )],
            },
            &options,
            &ExtensionRegistry::default(),
        );
        assert_eq!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == AnalyzerDiagnosticCode::InvalidArgument }),
            invalid,
            "{statement}: {:#?}",
            report.diagnostics
        );
    }
}
