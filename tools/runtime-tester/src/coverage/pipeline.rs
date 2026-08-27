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
    pub symbols: Value,
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
        symbols: json!({"phase": "unavailable", "status": "no_project_data"}),
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
    result.symbols = json!({"phase": "csv_load", "status": "declarations_not_resolved", "data": super::symbols::data(&project_data, "csv_load")});
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
    // A diagnostic-bearing analyzer result can still contain resolved symbols.
    // Capture them before compile gating; never serialize its HIR bodies.
    if let Some(project) = &report.project {
        result.symbols = super::symbols::analyzed(project, sources);
    }
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

/// Stable IDs refer to the single authoritative diagnostics array in the report.
/// Index by both path and stage so millions of appearances do not rescan every
/// project's diagnostic. Unlocated diagnostics remain in the project evidence.
pub(super) struct DiagnosticIndex<'a> {
    by_path: std::collections::BTreeMap<&'a str, std::collections::BTreeMap<&'a str, Vec<usize>>>,
    diagnostics: &'a [LocatedDiagnostic],
}

impl<'a> DiagnosticIndex<'a> {
    pub fn new(diagnostics: &'a [LocatedDiagnostic]) -> Self {
        let mut result = Self {
            by_path: Default::default(),
            diagnostics,
        };
        for (id, diagnostic) in diagnostics.iter().enumerate() {
            if let Some(path) = diagnostic.path.as_deref() {
                result
                    .by_path
                    .entry(path)
                    .or_default()
                    .entry(&diagnostic.stage)
                    .or_default()
                    .push(id);
            }
        }
        result
    }

    pub fn overlapping(&self, path: &str, span: Span, stage: &str) -> Vec<usize> {
        self.by_path
            .get(path)
            .and_then(|stages| stages.get(stage))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&id| {
                self.diagnostics[id].span.is_some_and(|location| {
                    location.start < span.end && span.start < location.end
                        || location.start == span.start
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_index_preserves_stage_path_span_and_stable_ids() {
        let diagnostics = [
            ("a.erb", "analyzer", 0, 4),
            ("b.erb", "analyzer", 0, 4),
            ("a.erb", "compiler", 2, 5),
            ("a.erb", "analyzer", 4, 4),
        ]
        .into_iter()
        .map(|(path, stage, start, end)| LocatedDiagnostic {
            stage: stage.into(),
            path: Some(path.into()),
            span: Some(Span::new(start, end)),
            code: "test".into(),
            error: true,
            details: json!({}),
        })
        .collect::<Vec<_>>();
        let index = DiagnosticIndex::new(&diagnostics);
        assert_eq!(index.overlapping("a.erb", Span::new(1, 3), "analyzer"), [0]);
        assert_eq!(index.overlapping("a.erb", Span::new(1, 3), "compiler"), [2]);
        assert_eq!(index.overlapping("a.erb", Span::new(4, 4), "analyzer"), [3]);
        assert!(
            index
                .overlapping("missing.erb", Span::new(0, 5), "analyzer")
                .is_empty()
        );
    }

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

    #[test]
    fn symbols_survive_compile_blocking_diagnostics_with_signed_aliases_and_reverse_names() {
        use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
        use erabasic_csv::{FilePayload, FrontendFile};
        let file = |path: &str, text: &str| FrontendFile {
            relative_path: path.into(),
            source_path: Some(path.into()),
            payload: FilePayload::Utf8(text.into()),
        };
        let files = ProjectFiles {
            csv: vec![
                file("BUFF.csv", "10,z_primary\n"),
                file("BUFF.als", "10,a_alias\n11,eleven\n300,far\n-1,negative\n"),
            ],
            erb: vec![
                file("COLUMNDIV@2.ERD", "10,column\n"),
                file("COLUMNDIV@2.als", "11,column_alias\n"),
                file("SEMEN_MATRIX@2.ERD", "11,matrix\n"),
                file("SEMEN_MATRIX@2.als", "300,matrix_alias\n"),
            ],
        };
        let sources = vec![("ERH/data.erh".into(), "#DIM BUFF, 400\n#DIM CHARADATA COLUMNDIV, 2, 12\n#DIM SEMEN_MATRIX, 2, 12\n".into()),
            ("ERB/main.erb".into(), "@SYSTEM_TITLE\nUNKNOWN_COVERAGE_GATE 1\nRETURN\n@CAN_MOVE_A\n#FUNCTION\nRETURNF 1\n".into())];
        let identity = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        let mut options = AnalyzerOptions::analysis_mode();
        options.compatibility = identity.clone();
        options.system_save_in_binary = true;
        let report = analyze(
            &sources,
            &files,
            options,
            CsvLoadOptions {
                compatibility: identity,
                use_erd: true,
                ..CsvLoadOptions::default()
            },
            true,
        );
        assert!(report.compiler.starts_with("blocked"));
        assert_eq!(report.symbols["phase"], "analyzer_project");
        let indices = report.symbols["data"]["resolved_user_indices"]
            .as_array()
            .unwrap();
        let buff = indices
            .iter()
            .find(|table| table["stem"] == "BUFF")
            .unwrap();
        assert_eq!(buff["variable_name"], "BUFF");
        assert_eq!(buff["data_dimension_length"], 400);
        for (name, index) in [
            ("a_alias", 10),
            ("eleven", 11),
            ("far", 300),
            ("negative", -1),
        ] {
            assert_eq!(buff["signed_name_lookup"][name], index);
        }
        assert_eq!(
            buff["reverse_names_in_primary_then_insertion_precedence"]["10"],
            "z_primary"
        );
        let columns = indices
            .iter()
            .find(|table| table["stem"] == "COLUMNDIV@2")
            .unwrap();
        assert_eq!(columns["data_dimension"], 2);
        assert_eq!(columns["variable_name"], "COLUMNDIV");
        assert_eq!(columns["data_dimension_length"], 12);
        let matrix = indices
            .iter()
            .find(|table| table["stem"] == "SEMEN_MATRIX@2")
            .unwrap();
        assert_eq!(matrix["variable_name"], "SEMEN_MATRIX");
        assert_eq!(matrix["data_dimension_length"], 12);
        assert_eq!(matrix["signed_name_lookup"]["matrix_alias"], 300);
        let method = report.symbols["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|method| method["name"] == "CAN_MOVE_A")
            .unwrap();
        assert_eq!(method["kind"], "method");
        assert_eq!(method["return_type"], "integer");
        assert!(
            method.get("lines").is_none(),
            "coverage must not retain HIR bodies"
        );
    }
}
