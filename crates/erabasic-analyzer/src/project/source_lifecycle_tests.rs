use super::*;
use crate::{ProjectSource, SourcePayload};
use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
use erabasic_hir::SourceId;
use std::sync::Mutex;

#[test]
fn large_workload_progress_reports_first_percent() {
    let events = Mutex::new(Vec::new());
    let callback = |event| events.lock().unwrap().push(event);
    let progress = ProgressCounter::new(
        AnalysisProgressStage::DeclaringLocals,
        100_000,
        Some(&callback),
    );
    for _ in 0..1_000 {
        progress.advance();
    }
    let events = events.into_inner().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].completed, 1_000);
    assert_eq!(events[1].total, 100_000);
}

fn input(sources: &[(&str, &str)]) -> AnalysisInput {
    AnalysisInput {
        project_data: load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .expect("default project data"),
        sources: sources
            .iter()
            .map(|(relative_path, text)| ProjectSource {
                relative_path: (*relative_path).into(),
                payload: SourcePayload::Utf8((*text).into()),
            })
            .collect(),
    }
}

fn assert_progressive_owned_analysis_matches_parallel(input: AnalysisInput) {
    let options = AnalyzerOptions::analysis_mode();
    let extensions = ExtensionRegistry::default();
    let parallel = analyze_project_inner_with_source_lifecycle(
        input.clone(),
        &options,
        &extensions,
        None,
        false,
    );
    let progressive =
        analyze_project_inner_with_source_lifecycle(input, &options, &extensions, None, true);
    assert_eq!(progressive, parallel);
}

fn parsed_sources(project_data: &ProjectData) -> Vec<ParsedProjectSource> {
    let catalog = Catalog::build(&ExtensionRegistry::default());
    let options = AnalyzerOptions::analysis_mode();
    let mut context =
        AnalysisParserContext::new(&project_data.schema, &catalog, std::iter::empty(), &options);
    let header_text = "#DIM CONST SIZE = 3\n";
    let header = parse_erh(header_text, &mut context)
        .value
        .expect("header AST");
    let script_text = "PRINT top level\n@SYSTEM_TITLE\nRESULT = SIZE + CLIENTWIDTH()\nRETURN\n";
    let script = parse_erb(script_text, &mut context)
        .value
        .expect("script AST");
    vec![
        ParsedProjectSource {
            source: source_file(SourceId(0), "vars.erh".into(), SourceKind::Erh, header_text),
            text: header_text.into(),
            script: header,
        },
        ParsedProjectSource {
            source: source_file(SourceId(1), "main.erb".into(), SourceKind::Erb, script_text),
            text: script_text.into(),
            script,
        },
    ]
}

#[test]
fn borrowed_and_owned_preserved_ast_paths_return_identical_reports() {
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .expect("default project data");
    let sources = parsed_sources(&project_data);
    let options = AnalyzerOptions::analysis_mode();
    let extensions = ExtensionRegistry::default();
    let borrowed = analyze_parsed_project(project_data.clone(), &sources, &options, &extensions);
    let catalog = Catalog::build(&extensions);
    let mut context = AnalysisParserContext::new(
        &project_data.schema,
        &catalog,
        sources
            .iter()
            .flat_map(|source| source.script.functions.iter())
            .map(|function| function.name.clone()),
        &options,
    );
    for source in sources
        .iter()
        .filter(|source| source.source.kind == SourceKind::Erh)
    {
        let _ = parse_erh(&source.text, &mut context);
    }
    let owned = analyze_with_context(
        project_data,
        Cow::Owned(sources),
        &options,
        &catalog,
        &context,
        Vec::new(),
        None,
        false,
    );

    assert_eq!(owned, borrowed);
}

#[test]
fn progressive_owned_analysis_preserves_hir_and_diagnostic_order() {
    assert_progressive_owned_analysis_matches_parallel(input(&[
        ("変数.erh", "#DIM CONST SIZE = 3\n"),
        (
            "main.erb",
            "PRINT top level\n@SYSTEM_TITLE\nRESULT = SIZE + CLIENTWIDTH()\nRETURN\n",
        ),
        ("unused.erb", "@UNUSED\nPRINT あいう\nRETURN\n"),
    ]));
}

#[test]
fn progressive_owned_analysis_handles_sparse_duplicate_definitions() {
    assert_progressive_owned_analysis_matches_parallel(input(&[
        ("first.erb", "@SYSTEM_TITLE\nRETURN\n@SAME\nRETURN\n"),
        ("second.erb", "@SAME\nPRINT duplicate\nRETURN\n"),
    ]));
}
