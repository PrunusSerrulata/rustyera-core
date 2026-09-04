use super::*;
use crate::default_host_registry;
use erabasic_analyzer::{
    AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
    analyze_project,
};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

fn analyzed(source: &str) -> AnalyzedProject {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data");
    let report = analyze_project(
        AnalysisInput {
            project_data,
            sources: vec![ProjectSource {
                relative_path: "main.erb".into(),
                payload: SourcePayload::Utf8(source.into()),
            }],
        },
        &AnalyzerOptions::analysis_mode(),
        &ExtensionRegistry::default(),
    );
    assert!(report.project.is_some(), "{:#?}", report.diagnostics);
    report.project.expect("analyzed project")
}

#[test]
fn consumed_owned_hir_matches_the_borrowed_compile() {
    let source = "@SYSTEM_TITLE\nCALL HELPER\nRETURN\n\
                  @HELPER(ARG = 2)\nRESULT = ARG\nRETURN\n";
    let borrowed_project = analyzed(source);
    let expected = compile_validated_project_with_artifact(
        &borrowed_project,
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
    );
    let consumed = compile_project_inner(
        ProjectInput::Owned(Box::new(analyzed(source))),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: true,
        },
        None,
    );

    assert_eq!(consumed.report, expected);
    assert_eq!(consumed.source_ids, [SourceId(0)]);
    assert!(consumed.diagnostic_sources.is_empty());
}

#[test]
fn compilation_preparation_reports_intermediate_work() {
    let events = std::sync::Mutex::new(Vec::new());
    let callback = |progress| events.lock().unwrap().push(progress);
    let project = analyzed("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRETURN\n");
    let expected_work = project
        .program
        .functions
        .len()
        .saturating_mul(5)
        .saturating_add(project.program.variables.len().saturating_mul(2))
        .saturating_add(project.program.sources.len());
    let report = compile_project_inner(
        ProjectInput::Owned(Box::new(project)),
        &CompilerOptions::default(),
        &default_host_registry(),
        None,
        None,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: true,
        },
        Some(&callback),
    );

    assert!(
        report.report.artifact.is_some(),
        "{:#?}",
        report.report.diagnostics
    );
    let events = events.into_inner().unwrap();
    let compiling = events
        .iter()
        .filter(|progress| progress.stage == CompileProgressStage::Compiling)
        .collect::<Vec<_>>();
    assert_eq!(compiling.first().unwrap().completed, 0);
    assert_eq!(compiling.last().unwrap().total, expected_work);
    assert_eq!(
        compiling.last().unwrap().completed,
        compiling.last().unwrap().total
    );
    assert!(
        compiling
            .iter()
            .any(|progress| progress.completed > 0 && progress.completed < progress.total),
        "compilation preparation did not expose intermediate progress: {compiling:?}"
    );
}
