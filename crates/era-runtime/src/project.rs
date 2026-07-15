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
    files.sort_by_key(|file| {
        (
            file.category as u8,
            file.relative_path.to_ascii_lowercase(),
            file.relative_path.clone(),
        )
    });
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
            FileCategory::Configuration
            | FileCategory::ResourceManifest
            | FileCategory::Resource => {}
        }
    }

    let csv = load_project(&csv_files, &CsvLoadOptions::default());
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
        &AnalyzerOptions::default(),
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
