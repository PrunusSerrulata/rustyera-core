//! Run the real loader/analyzer/compiler without converting project failures into tool failures.

use erabasic_analyzer::{AnalyzerDiagnosticSeverity, AnalyzerOptions};
use erabasic_ast::Span;
use erabasic_compiler::{CompilerDiagnosticSeverity, default_host_registry};
use erabasic_csv::{CsvDiagnosticSeverity, CsvLoadOptions, ProjectFiles};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
pub(super) struct LocatedDiagnostic {
    pub stage: String,
    pub path: Option<String>,
    pub span: Option<Span>,
    pub code: String,
    pub error: bool,
    pub details: Value,
}

#[derive(Debug, Serialize)]
pub(super) struct Pipeline {
    pub csv: String,
    pub analyzer: String,
    pub compiler: String,
    pub artifact_produced: bool,
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub analyzer_options: AnalyzerOptions,
    pub csv_options: CsvLoadOptions,
}

pub(super) fn analyze(
    sources: &[(String, String)],
    files: &ProjectFiles,
    analyzer_options: AnalyzerOptions,
    csv_options: CsvLoadOptions,
    inputs_complete: bool,
) -> Pipeline {
    let mut result = Pipeline {
        csv: "not_run".into(),
        analyzer: "blocked_by_csv".into(),
        compiler: "blocked_by_analyzer".into(),
        artifact_produced: false,
        diagnostics: Vec::new(),
        analyzer_options,
        csv_options,
    };
    crate::watchdog::publish_or_exit(
        json!({"phase": "csv_load", "case": sources.first().map(|(path, _)| path), "pending": "load_project", "csv_files": files.csv.len(), "erb_data_files": files.erb.len(), "diagnostics": [], "lastFullResponse": null}),
    );
    let csv = erabasic_csv::load_project(files, &result.csv_options);
    let csv_failed = csv.diagnostics.iter().any(|item| {
        matches!(
            item.severity,
            CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal
        )
    });
    result.csv = if csv_failed {
        "diagnostic_errors"
    } else {
        "accepted"
    }
    .into();
    result
        .diagnostics
        .extend(csv.diagnostics.into_iter().map(|item| {
            LocatedDiagnostic {
                stage: "csv".into(),
                path: item
                    .source
                    .as_ref()
                    .map(|source| source.relative_path.clone()),
                span: item
                    .source
                    .as_ref()
                    .map(|source| Span::new(source.byte_start, source.byte_end)),
                code: format!("{:?}", item.code),
                error: matches!(
                    item.severity,
                    CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal
                ),
                details: json!(item),
            }
        }));
    let Some(project_data) = csv.data else {
        return result;
    };
    crate::watchdog::publish_or_exit(
        json!({"phase": "analysis", "pending": "analyze_project", "source_count": sources.len(), "diagnostics": result.diagnostics, "lastFullResponse": null}),
    );
    let progress = |progress: erabasic_analyzer::AnalysisProgress| {
        crate::watchdog::publish_or_exit(
            json!({"phase": "analysis", "pending": "analyze_project", "projectProgress": {"stage": format!("{:?}", progress.stage), "completed": progress.completed, "total": progress.total}, "diagnostics": result.diagnostics, "lastFullResponse": null}),
        );
    };
    let report = erabasic_analyzer::analyze_project_with_progress(
        erabasic_analyzer::AnalysisInput {
            project_data,
            sources: sources
                .iter()
                .map(|(path, text)| erabasic_analyzer::ProjectSource {
                    relative_path: path.clone(),
                    payload: erabasic_analyzer::SourcePayload::Utf8(text.clone()),
                })
                .collect(),
        },
        &result.analyzer_options,
        &Default::default(),
        &progress,
    );
    let analyzer_failed = report.diagnostics.iter().any(|item| {
        matches!(
            item.severity,
            AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal
        )
    });
    result.analyzer = if analyzer_failed {
        "diagnostic_errors"
    } else {
        "accepted"
    }
    .into();
    result
        .diagnostics
        .extend(report.diagnostics.into_iter().map(|item| {
            LocatedDiagnostic {
                stage: "analyzer".into(),
                path: item
                    .source
                    .as_ref()
                    .map(|source| source.relative_path.clone()),
                span: item
                    .source
                    .as_ref()
                    .map(|source| Span::new(source.byte_start, source.byte_end)),
                code: format!("{:?}", item.code),
                error: matches!(
                    item.severity,
                    AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal
                ),
                details: json!(item),
            }
        }));
    if csv_failed || analyzer_failed || !inputs_complete {
        result.compiler = if !inputs_complete {
            "blocked_by_incomplete_input"
        } else {
            "blocked_by_load_diagnostics"
        }
        .into();
        return result;
    }
    let Some(project) = report.project else {
        return result;
    };
    crate::watchdog::publish_or_exit(
        json!({"phase": "compile", "pending": "compile_project", "diagnostics": result.diagnostics, "lastFullResponse": null}),
    );
    let progress = |progress: erabasic_compiler::CompileProgress| {
        crate::watchdog::publish_or_exit(
            json!({"phase": "compile", "pending": "compile_project", "projectProgress": {"stage": format!("{:?}", progress.stage), "completed": progress.completed, "total": progress.total}, "diagnostics": result.diagnostics, "lastFullResponse": null}),
        );
    };
    let compiled = erabasic_compiler::compile_project_with_artifact_and_progress(
        &project,
        &Default::default(),
        &default_host_registry(),
        None,
        None,
        &progress,
    );
    result.artifact_produced = compiled.artifact.is_some();
    result.compiler = if compiled
        .diagnostics
        .iter()
        .any(|item| item.severity == CompilerDiagnosticSeverity::Error)
    {
        "diagnostic_errors"
    } else if result.artifact_produced {
        "accepted"
    } else {
        "no_artifact"
    }
    .into();
    result
        .diagnostics
        .extend(compiled.diagnostics.into_iter().map(|item| {
            let path = item.location.and_then(|location| {
                project
                    .program
                    .sources
                    .iter()
                    .find(|source| source.id == location.source)
                    .map(|source| source.relative_path.clone())
            });
            LocatedDiagnostic {
                stage: "compiler".into(),
                path,
                span: item.location.map(|location| location.span),
                code: format!("{:?}", item.code),
                error: item.severity == CompilerDiagnosticSeverity::Error,
                details: json!(item),
            }
        }));
    result
}

pub(super) fn overlapping<'a>(
    pipeline: &'a Pipeline,
    path: &str,
    span: Span,
    stage: &str,
) -> Vec<&'a LocatedDiagnostic> {
    pipeline
        .diagnostics
        .iter()
        .filter(|item| {
            item.stage == stage
                && item.path.as_deref() == Some(path)
                && item.span.is_some_and(|location| {
                    location.start < span.end && span.start < location.end
                        || location.start == span.start
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_instruction_preserves_analysis_diagnostics_and_blocks_compile() {
        let sources = vec![(
            "ERB/main.erb".into(),
            "@SYSTEM_TITLE\nUNKNOWN_BATCH0_API 1\nRETURN\n".into(),
        )];
        let report = analyze(
            &sources,
            &ProjectFiles::default(),
            AnalyzerOptions::analysis_mode(),
            CsvLoadOptions::default(),
            true,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| item.stage == "analyzer" && item.code == "UnknownInstruction")
        );
        assert!(!report.artifact_produced);
        assert!(report.compiler.starts_with("blocked"));
    }

    #[test]
    fn missing_input_never_turns_a_partial_analysis_into_a_compiler_pass() {
        let sources = vec![("ERB/main.erb".into(), "@SYSTEM_TITLE\nRETURN\n".into())];
        let report = analyze(
            &sources,
            &ProjectFiles::default(),
            AnalyzerOptions::analysis_mode(),
            CsvLoadOptions::default(),
            false,
        );
        assert_eq!(report.compiler, "blocked_by_incomplete_input");
        assert!(!report.artifact_produced);
    }
}
