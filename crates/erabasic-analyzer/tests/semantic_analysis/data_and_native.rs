use super::*;

fn graphics_diagnostics(
    profile: erabasic_compat::CompatibilityProfileId,
    statement: &str,
) -> Vec<AnalyzerDiagnostic> {
    analyze_project(
        AnalysisInput {
            project_data: empty_project(),
            sources: vec![source(
                "graphics-signature.erb",
                &format!("@SYSTEM_TITLE\n{statement}\nRETURN\n"),
            )],
        },
        &AnalyzerOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
            ..AnalyzerOptions::analysis_mode()
        },
        &ExtensionRegistry::default(),
    )
    .diagnostics
}

#[path = "data_and_native/call_contracts.rs"]
mod call_contracts;

#[path = "data_and_native/graphics_audio.rs"]
mod graphics_audio;

#[path = "data_and_native/indices_and_folding.rs"]
mod indices_and_folding;

#[path = "data_and_native/snake_data.rs"]
mod snake_data;

#[path = "data_and_native/structured_native.rs"]
mod structured_native;
