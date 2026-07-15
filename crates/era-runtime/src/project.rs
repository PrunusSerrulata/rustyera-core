use era_runtime_protocol::{
    DiagnosticSeverity, FileCategory, FilePayload, ProjectLoadReport, ProjectManifest,
    ProtocolDiagnostic, SourceLocation, validate_relative_path,
};
use erabasic_analyzer::{
    AnalysisInput, AnalyzerDiagnosticSeverity, AnalyzerOptions, ExtensionRegistry, ProjectSource,
    SourceIoError, SourceIoErrorKind, SourcePayload, analyze_project,
};
use erabasic_compiler::{
    CompilerOptions, IncrementalState, compile_project, default_host_registry,
};
use erabasic_csv::{
    CsvDiagnosticSeverity, CsvLoadOptions, FilePayload as CsvFilePayload,
    FrontendFile as CsvFrontendFile, FrontendIoError as CsvIoError,
    FrontendIoErrorKind as CsvIoErrorKind, ProjectFiles, load_project,
};
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};

pub(crate) struct ProjectBuild {
    pub(crate) artifact: Option<ValidatedArtifact>,
    pub(crate) incremental: IncrementalState,
    pub(crate) report: ProjectLoadReport,
    pub(crate) snapshot: Option<NormalizedProjectSnapshot>,
}

#[derive(Clone)]
pub(crate) struct NormalizedProjectSnapshot {
    pub(crate) manifest: ProjectManifest,
    pub(crate) sort_with_filename: bool,
    pub(crate) use_new_random_ignored: bool,
}

#[derive(Clone, Debug, Default)]
struct SemanticConfig {
    csv: CsvLoadOptions,
    analyzer: AnalyzerOptions,
    use_new_random: bool,
}

// Keeping the pipeline in one function makes the atomic artifact/report outcome visible;
// conversion details live in the helpers below.
#[allow(clippy::too_many_lines)]
pub(crate) fn build_project(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    let mut diagnostics = Vec::new();
    let mut files = manifest.files.clone();
    let config = parse_configuration(&files, &mut diagnostics);
    if config.csv.sort_with_filename {
        files.sort_by_key(|file| {
            (
                !path_has_priority_directory(&file.relative_path),
                file.relative_path.to_ascii_lowercase(),
                file.relative_path.clone(),
                file.category as u8,
            )
        });
    }
    let mut csv_files = ProjectFiles::default();
    let mut sources = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for mut file in files {
        let path = match validate_relative_path(&file.relative_path) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_path",
                    DiagnosticSeverity::Error,
                    error.message,
                    Some(SourceLocation {
                        relative_path: file.relative_path,
                        byte_start: 0,
                        byte_end: 0,
                        line: None,
                        byte_column: None,
                    }),
                ));
                continue;
            }
        };
        file.relative_path.clone_from(&path);
        if !seen.insert((file.category as u8, path.to_ascii_lowercase())) {
            diagnostics.push(project_diagnostic(
                "runtime.duplicate_path",
                DiagnosticSeverity::Error,
                "duplicate normalized project path",
                Some(SourceLocation {
                    relative_path: path,
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            continue;
        }
        if let (Some(expected), Some(actual)) =
            (file.content_hash.as_ref(), payload_hash(&file.payload))
            && expected.as_slice() != actual.as_bytes()
        {
            diagnostics.push(project_diagnostic(
                "runtime.content_hash_mismatch",
                DiagnosticSeverity::Error,
                "submitted content hash does not match the payload",
                Some(SourceLocation {
                    relative_path: path.clone(),
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            continue;
        }
        match file.category {
            FileCategory::Csv => csv_files.csv.push(csv_file(path, file.payload)),
            FileCategory::Erh | FileCategory::Erb => {
                csv_files
                    .erb
                    .push(csv_file(path.clone(), file.payload.clone()));
                sources.push(analyzer_source(path, file.payload));
            }
            FileCategory::Configuration => {}
            FileCategory::ResourceManifest => inspect_deferred_file(
                &mut diagnostics,
                &path,
                &file.payload,
                true,
                "runtime.resource_manifest_deferred",
                "resource manifest input is validated but is not applied by this runtime stage",
            ),
            FileCategory::Resource => inspect_deferred_file(
                &mut diagnostics,
                &path,
                &file.payload,
                false,
                "runtime.resource_deferred",
                "resource bytes are retained by the frontend until resource services are enabled",
            ),
        }
    }

    let csv = load_project(&csv_files, &config.csv);
    diagnostics.extend(csv.diagnostics.iter().map(|diagnostic| ProtocolDiagnostic {
        code: format!("csv.{:?}", diagnostic.code).to_ascii_lowercase(),
        severity: match diagnostic.severity {
            CsvDiagnosticSeverity::Notice => DiagnosticSeverity::Information,
            CsvDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
            CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal => {
                DiagnosticSeverity::Error
            }
        },
        message: diagnostic.message.clone(),
        source: diagnostic.source.as_ref().map(|source| SourceLocation {
            relative_path: source.relative_path.clone(),
            byte_start: source.byte_start as u64,
            byte_end: source.byte_end as u64,
            line: Some(u64::from(source.physical_line)),
            byte_column: None,
        }),
    }));
    let Some(data) = csv.data else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    let analysis = analyze_project(
        AnalysisInput {
            project_data: data,
            sources,
        },
        &config.analyzer,
        &ExtensionRegistry::default(),
    );
    diagnostics.extend(
        analysis
            .diagnostics
            .iter()
            .map(|diagnostic| ProtocolDiagnostic {
                code: format!("analyzer.{:?}", diagnostic.code).to_ascii_lowercase(),
                severity: match diagnostic.severity {
                    AnalyzerDiagnosticSeverity::Notice => DiagnosticSeverity::Information,
                    AnalyzerDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                    AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal => {
                        DiagnosticSeverity::Error
                    }
                },
                message: diagnostic.message.clone(),
                source: diagnostic.source.as_ref().map(|source| SourceLocation {
                    relative_path: source.relative_path.clone(),
                    byte_start: source.byte_start as u64,
                    byte_end: source.byte_end as u64,
                    line: Some(u64::from(source.physical_line)),
                    byte_column: None,
                }),
            }),
    );
    let Some(project) = analysis.project else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    let compile = compile_project(
        &project,
        &CompilerOptions::default(),
        &default_host_registry(),
        previous,
    );
    diagnostics.extend(
        compile
            .diagnostics
            .iter()
            .map(|diagnostic| ProtocolDiagnostic {
                code: format!("compiler.{:?}", diagnostic.code).to_ascii_lowercase(),
                severity: DiagnosticSeverity::Error,
                message: diagnostic.message.clone(),
                source: diagnostic.location.map(|location| SourceLocation {
                    relative_path: project
                        .program
                        .sources
                        .iter()
                        .find(|source| source.id == location.source)
                        .map_or_else(String::new, |source| source.relative_path.clone()),
                    byte_start: location.span.start as u64,
                    byte_end: location.span.end as u64,
                    line: None,
                    byte_column: None,
                }),
            }),
    );
    let incremental = compile.incremental_state;
    let Some(artifact) = compile.artifact else {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    };
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    diagnostics.extend(validation.diagnostics.iter().map(|diagnostic| {
        project_diagnostic(
            &format!("validator.{:?}", diagnostic.code).to_ascii_lowercase(),
            DiagnosticSeverity::Error,
            diagnostic.message.clone(),
            None,
        )
    }));
    let artifact = validation.value;
    let success = artifact.is_some()
        && !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);
    ProjectBuild {
        artifact: success.then_some(artifact).flatten(),
        incremental,
        report: ProjectLoadReport {
            project_revision: manifest.project_revision,
            success,
            diagnostics,
        },
        snapshot: Some(NormalizedProjectSnapshot {
            manifest: manifest.clone(),
            sort_with_filename: config.csv.sort_with_filename,
            use_new_random_ignored: config.use_new_random,
        }),
    }
}

fn inspect_deferred_file(
    diagnostics: &mut Vec<ProtocolDiagnostic>,
    path: &str,
    payload: &FilePayload,
    require_utf8: bool,
    code: &str,
    message: &str,
) {
    let location = || SourceLocation {
        relative_path: path.into(),
        byte_start: 0,
        byte_end: 0,
        line: None,
        byte_column: None,
    };
    match payload {
        FilePayload::IoError(error) => diagnostics.push(project_diagnostic(
            "runtime.frontend_io_error",
            DiagnosticSeverity::Error,
            error.message.clone(),
            Some(location()),
        )),
        FilePayload::Bytes(_) if require_utf8 => diagnostics.push(project_diagnostic(
            "runtime.expected_utf8",
            DiagnosticSeverity::Error,
            "configuration and resource manifests must be submitted as UTF-8",
            Some(location()),
        )),
        FilePayload::Utf8(_) | FilePayload::Bytes(_) => diagnostics.push(project_diagnostic(
            code,
            DiagnosticSeverity::Warning,
            message,
            Some(location()),
        )),
    }
}

fn failed(
    revision: u64,
    diagnostics: Vec<ProtocolDiagnostic>,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    failed_with_incremental(revision, diagnostics, previous.cloned().unwrap_or_default())
}

fn failed_with_incremental(
    revision: u64,
    diagnostics: Vec<ProtocolDiagnostic>,
    incremental: IncrementalState,
) -> ProjectBuild {
    ProjectBuild {
        artifact: None,
        incremental,
        report: ProjectLoadReport {
            project_revision: revision,
            success: false,
            diagnostics,
        },
        snapshot: None,
    }
}

fn path_has_priority_directory(path: &str) -> bool {
    path.replace('\\', "/")
        .split('/')
        .rev()
        .skip(1)
        .any(|component| component.contains('#'))
}

fn parse_configuration(
    files: &[era_runtime_protocol::SubmittedFile],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> SemanticConfig {
    let mut config = SemanticConfig::default();
    for file in files
        .iter()
        .filter(|file| file.category == FileCategory::Configuration)
    {
        let FilePayload::Utf8(text) = &file.payload else {
            inspect_deferred_file(
                diagnostics,
                &file.relative_path,
                &file.payload,
                true,
                "runtime.configuration_ignored",
                "configuration payload was not UTF-8",
            );
            continue;
        };
        if parse_json_configuration(text, &file.relative_path, &mut config, diagnostics) {
            continue;
        }
        for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_configuration",
                    DiagnosticSeverity::Warning,
                    "configuration line has no ':' separator",
                    Some(SourceLocation {
                        relative_path: file.relative_path.clone(),
                        byte_start: 0,
                        byte_end: 0,
                        line: Some(line_index as u64 + 1),
                        byte_column: None,
                    }),
                ));
                continue;
            };
            let Some(boolean) = parse_bool(value.trim()) else {
                continue;
            };
            match name.trim() {
                "大文字小文字の違いを無視する" | "Ignore case" => {
                    config.csv.ignore_case = boolean;
                    config.analyzer.ignore_case = boolean;
                }
                "_Rename.csvを利用する" | "Use _Rename.csv file" => {
                    config.csv.use_rename_file = boolean;
                }
                "_Replace.csvを利用する" | "Use _Replace.csv file" => {
                    config.csv.use_replace_file = boolean;
                }
                "サブディレクトリを検索する" | "Search subfolders" => {
                    config.csv.search_subdirectories = boolean;
                }
                "読み込み順をファイル名順にソートする" | "Sort filenames" => {
                    config.csv.sort_with_filename = boolean;
                    config.analyzer.sort_with_filename = boolean;
                }
                "全角スペースをホワイトスペースに含める"
                | "Whitespace includes full-width space" => {
                    config.csv.allow_full_width_space = boolean;
                    config.analyzer.allow_full_width_space = boolean;
                }
                "SPキャラを使用する" | "Allow SP characters" => {
                    config.csv.compatible_sp_character = boolean;
                }
                "ERD機能を利用する" | "Use ERD" => {
                    config.csv.use_erd = boolean;
                    config.analyzer.use_erd = boolean;
                }
                "UseNewRandom" | "新しい高速な乱数アルゴリズムを使う" => {
                    config.use_new_random = boolean;
                }
                _ => {}
            }
        }
    }
    if config.use_new_random {
        diagnostics.push(project_diagnostic(
            "runtime.use_new_random_ignored",
            DiagnosticSeverity::Warning,
            "UseNewRandom=true is ignored; the pinned SFMT implementation is always used",
            None,
        ));
    }
    config
}

fn parse_json_configuration(
    text: &str,
    path: &str,
    config: &mut SemanticConfig,
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> bool {
    let text = text.trim_start_matches('\u{feff}');
    if !text.trim_start().starts_with('{') {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            if let Some(boolean) = value
                .get("UseNewRandom")
                .and_then(serde_json::Value::as_bool)
            {
                config.use_new_random = boolean;
            }
        }
        Err(error) => diagnostics.push(project_diagnostic(
            "runtime.invalid_json_configuration",
            DiagnosticSeverity::Warning,
            error.to_string(),
            Some(SourceLocation {
                relative_path: path.into(),
                byte_start: 0,
                byte_end: u64::try_from(text.len()).unwrap_or(u64::MAX),
                line: None,
                byte_column: None,
            }),
        )),
    }
    true
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_uppercase().as_str() {
        "YES" | "TRUE" | "1" => Some(true),
        "NO" | "FALSE" | "0" => Some(false),
        _ => None,
    }
}

fn csv_file(path: String, payload: FilePayload) -> CsvFrontendFile {
    CsvFrontendFile {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => CsvFilePayload::Utf8(value),
            FilePayload::Bytes(_) => CsvFilePayload::IoError(CsvIoError {
                kind: CsvIoErrorKind::InvalidData,
                message: "CSV and EraBasic sources must be submitted as UTF-8".into(),
            }),
            FilePayload::IoError(error) => CsvFilePayload::IoError(CsvIoError {
                kind: csv_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

fn analyzer_source(path: String, payload: FilePayload) -> ProjectSource {
    ProjectSource {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => SourcePayload::Utf8(value),
            FilePayload::Bytes(_) => SourcePayload::IoError(SourceIoError {
                kind: SourceIoErrorKind::InvalidData,
                message: "EraBasic sources must be submitted as UTF-8".into(),
            }),
            FilePayload::IoError(error) => SourcePayload::IoError(SourceIoError {
                kind: analyzer_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

fn csv_error_kind(kind: era_runtime_protocol::FrontendIoErrorKind) -> CsvIoErrorKind {
    match kind {
        era_runtime_protocol::FrontendIoErrorKind::NotFound => CsvIoErrorKind::NotFound,
        era_runtime_protocol::FrontendIoErrorKind::PermissionDenied => {
            CsvIoErrorKind::PermissionDenied
        }
        era_runtime_protocol::FrontendIoErrorKind::InvalidData => CsvIoErrorKind::InvalidData,
        era_runtime_protocol::FrontendIoErrorKind::Interrupted => CsvIoErrorKind::Interrupted,
        era_runtime_protocol::FrontendIoErrorKind::ReadOnly
        | era_runtime_protocol::FrontendIoErrorKind::AlreadyExists
        | era_runtime_protocol::FrontendIoErrorKind::Other => CsvIoErrorKind::Other,
    }
}

fn analyzer_error_kind(kind: era_runtime_protocol::FrontendIoErrorKind) -> SourceIoErrorKind {
    match kind {
        era_runtime_protocol::FrontendIoErrorKind::NotFound => SourceIoErrorKind::NotFound,
        era_runtime_protocol::FrontendIoErrorKind::PermissionDenied => {
            SourceIoErrorKind::PermissionDenied
        }
        era_runtime_protocol::FrontendIoErrorKind::InvalidData => SourceIoErrorKind::InvalidData,
        era_runtime_protocol::FrontendIoErrorKind::Interrupted => SourceIoErrorKind::Interrupted,
        era_runtime_protocol::FrontendIoErrorKind::ReadOnly
        | era_runtime_protocol::FrontendIoErrorKind::AlreadyExists
        | era_runtime_protocol::FrontendIoErrorKind::Other => SourceIoErrorKind::Other,
    }
}

fn payload_hash(payload: &FilePayload) -> Option<blake3::Hash> {
    match payload {
        FilePayload::Utf8(value) => Some(blake3::hash(value.as_bytes())),
        FilePayload::Bytes(value) => Some(blake3::hash(value.as_slice())),
        FilePayload::IoError(_) => None,
    }
}

fn project_diagnostic(
    code: &str,
    severity: DiagnosticSeverity,
    message: impl Into<String>,
    source: Option<SourceLocation>,
) -> ProtocolDiagnostic {
    ProtocolDiagnostic {
        code: code.into(),
        severity,
        message: message.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};

    use super::*;

    fn configuration(text: &str) -> SubmittedFile {
        SubmittedFile {
            relative_path: "emuera.config".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(text.into()),
            content_hash: None,
        }
    }

    #[test]
    fn semantic_configuration_is_applied_and_new_random_warns_once() {
        let mut diagnostics = Vec::new();
        let config = parse_configuration(
            &[configuration(
                "\u{feff}Sort filenames:YES\nIgnore case:NO\nUseNewRandom:TRUE\nフォント名:Test\n",
            )],
            &mut diagnostics,
        );
        assert!(config.csv.sort_with_filename);
        assert!(!config.csv.ignore_case);
        assert!(!config.analyzer.ignore_case);
        assert!(config.use_new_random);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "runtime.use_new_random_ignored")
                .count(),
            1
        );
    }

    #[test]
    fn json_configuration_applies_runtime_semantics_without_device_settings() {
        let mut diagnostics = Vec::new();
        let config = parse_configuration(
            &[configuration(
                r#"{"UseNewRandom":true,"UseMouse":false,"WindowWidth":1200}"#,
            )],
            &mut diagnostics,
        );
        assert!(config.use_new_random);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "runtime.use_new_random_ignored")
        );
    }

    #[test]
    fn only_directory_components_with_hash_receive_priority() {
        assert!(path_has_priority_directory("ERB/#boot/first.erb"));
        assert!(path_has_priority_directory("ERB/a#early/first.erb"));
        assert!(!path_has_priority_directory("ERB/ordinary/#function.erb"));
        assert!(!path_has_priority_directory("root.erb"));
    }
}
