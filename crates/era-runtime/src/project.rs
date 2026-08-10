mod configuration;
mod extensions;
mod frontend;
#[cfg(test)]
mod tests;

use era_config::{ConfigStore, ReraConfigDocument};
use era_runtime_protocol::{
    CONFIG_BROWSER, CONFIG_RUNTIME, CONFIG_TAURI, CONFIG_TUI, ConfigurationApplication,
    ConfigurationClientProfile, ConfigurationValueKind, FileCategory, FileChange, FilePayload,
    ProjectAnalysisReport, ProjectAnalysisRequest, ProjectConfigurationEntry,
    ProjectConfigurationSnapshot, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic,
    ReloadProject, RuntimeLogLevel, SourceLocation, SubmittedFile, validate_relative_path,
};
use erabasic_analyzer::{
    AnalysisInput, AnalysisProgressStage, AnalyzerDiagnosticSeverity, AnalyzerOptions,
    analyze_project, analyze_project_with_progress, compare_reference_file_paths,
};
use erabasic_bytecode::BytecodeArtifact;
use erabasic_compiler::{
    CompilerOptions, IncrementalState, compile_validated_project_with_artifact,
    compile_validated_project_with_artifact_and_progress,
};
use erabasic_csv::{CsvDiagnosticSeverity, CsvLoadOptions, ProjectFiles, load_project};
use erabasic_data::LegacyEncoding;
use erabasic_validator::ValidatedArtifact;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::resource::ResourceGraph;
use crate::{ProjectProgress, ProjectProgressReporter, ProjectProgressStage};

use self::configuration::{apply_replace_configuration, parse_configuration};
use self::extensions::{category_relative_path, is_deferred_index_source, prepare_extensions};
use self::frontend::{
    analyzer_source, csv_file, manifest_source_texts, payload_hash, project_diagnostic,
    project_source_location,
};

pub(crate) fn is_root_configuration_file(file: &SubmittedFile) -> bool {
    file.category == FileCategory::Configuration
        && file
            .relative_path
            .replace('\\', "/")
            .eq_ignore_ascii_case("reraconfig.toml")
}

pub(crate) struct ProjectBuild {
    pub(crate) artifact: Option<ValidatedArtifact>,
    pub(crate) incremental: IncrementalState,
    pub(crate) report: ProjectLoadReport,
    pub(crate) snapshot: Option<NormalizedProjectSnapshot>,
}

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct NormalizedProjectSnapshot {
    pub(crate) manifest: Arc<ProjectManifest>,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resources: Vec<NormalizedResourceIdentity>,
    pub(crate) resource_graph: ResourceGraph,
    pub(crate) sort_with_filename: bool,
    pub(crate) auto_save: bool,
    pub(crate) ctrl_z_enabled: bool,
    pub(crate) allow_long_input_by_activation: bool,
    pub(crate) save_in_binary: bool,
    pub(crate) compress_save: bool,
    pub(crate) save_slot_count: u32,
    pub(crate) money_label: String,
    pub(crate) money_first: bool,
    pub(crate) maximum_shop_items: u32,
    pub(crate) viewport_width: u32,
    pub(crate) viewport_height: u32,
    pub(crate) font_size: u32,
    pub(crate) line_height: u32,
    pub(crate) print_c_per_line: u32,
    pub(crate) print_c_length: u32,
    pub(crate) configuration_profile: ConfigurationClientProfile,
    /// Complete query-visible configuration, including client-only compatibility values.
    pub(crate) configuration: ConfigStore,
    /// Complete editable TOML values; the protocol applies each client's UI whitelist.
    pub(crate) editable_configuration: ConfigStore,
    pub(crate) configuration_document: ReraConfigDocument,
    pub(crate) generated_configuration_source: Option<String>,
    pub(crate) extensions:
        std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
}

impl NormalizedProjectSnapshot {
    pub(crate) fn configuration_snapshot(&self) -> ProjectConfigurationSnapshot {
        let mut configuration_files = self
            .manifest
            .files
            .iter()
            .filter(|file| is_root_configuration_file(file));
        let source = configuration_files.next();
        let generated_from_legacy =
            self.generated_configuration_source.is_some() && configuration_files.next().is_none();
        let source_digest = if generated_from_legacy {
            era_protocol::ProtocolBytes::new(Vec::new())
        } else {
            source
                .and_then(|file| match &file.payload {
                    FilePayload::Utf8(text) => Some(era_protocol::ProtocolBytes::new(
                        blake3::hash(era_config::normalize_line_endings(text).as_bytes())
                            .as_bytes()
                            .to_vec(),
                    )),
                    _ => None,
                })
                .unwrap_or_else(|| era_protocol::ProtocolBytes::new(Vec::new()))
        };
        let entries = era_config::catalog()
            .into_iter()
            .filter(|spec| {
                era_config::is_regular_code(spec.code)
                    || matches!(
                        spec.code,
                        "AudioVolume" | "ReplaceFullWidthSpaces" | "CharacterWidthMode"
                    )
            })
            .filter_map(|spec| {
                let value = self.editable_configuration.get_code(spec.code)?;
                let effective = self.configuration.get_code(spec.code)?;
                let applicability = protocol_applicability(spec.clients);
                (applicability != 0).then(|| ProjectConfigurationEntry {
                    code: spec.code.into(),
                    japanese: spec.japanese.into(),
                    english: spec.english.into(),
                    value: value.config_text(),
                    kind: configuration_value_kind(value),
                    allowed: match value {
                        era_config::ConfigValue::Enum { allowed, .. } => allowed.clone(),
                        _ => Vec::new(),
                    },
                    fixed: self.editable_configuration.is_fixed(spec.code),
                    applicability,
                    default_value: profile_default(&spec, self.configuration_profile).config_text(),
                    effective_value: effective.config_text(),
                    application: profile_application(spec.code, self.configuration_profile),
                })
            })
            .collect::<Vec<_>>();
        let restart_pending = entries
            .iter()
            .any(|entry| entry.value != entry.effective_value);
        ProjectConfigurationSnapshot {
            project_revision: self.manifest.project_revision,
            source_digest,
            entries,
            restart_pending,
            generated_source: self
                .generated_configuration_source
                .clone()
                .map(String::into_boxed_str),
        }
    }
}

fn profile_default(
    spec: &era_config::ConfigSpec,
    profile: ConfigurationClientProfile,
) -> era_config::ConfigValue {
    let override_value = match profile {
        ConfigurationClientProfile::Tui => era_config::tui_default(spec.code),
        ConfigurationClientProfile::Browser | ConfigurationClientProfile::Tauri => {
            era_config::web_default(spec.code)
        }
        ConfigurationClientProfile::Reference => None,
    };
    if let Some(value) = override_value {
        value
    } else {
        spec.default.clone()
    }
}

pub(crate) fn profile_application(
    code: &str,
    profile: ConfigurationClientProfile,
) -> ConfigurationApplication {
    let application = match profile {
        ConfigurationClientProfile::Tui => era_config::tui_application(code),
        ConfigurationClientProfile::Browser => era_config::browser_application(code),
        ConfigurationClientProfile::Tauri => era_config::tauri_application(code),
        ConfigurationClientProfile::Reference => None,
    };
    if application == Some(era_config::ConfigApplication::Hot) {
        ConfigurationApplication::Hot
    } else {
        ConfigurationApplication::Restart
    }
}

pub(crate) fn profile_applicability(profile: ConfigurationClientProfile) -> Option<u32> {
    match profile {
        ConfigurationClientProfile::Reference => None,
        ConfigurationClientProfile::Tui => Some(CONFIG_TUI),
        ConfigurationClientProfile::Browser => Some(CONFIG_BROWSER),
        ConfigurationClientProfile::Tauri => Some(CONFIG_TAURI),
    }
}

fn protocol_applicability(clients: &[era_config::ConfigClient]) -> u32 {
    use era_config::ConfigClient;
    let mut flags = 0;
    for client in clients {
        flags |= match client {
            ConfigClient::Runtime => CONFIG_RUNTIME,
            ConfigClient::Tui => CONFIG_TUI,
            ConfigClient::Browser => CONFIG_BROWSER,
            ConfigClient::Tauri => CONFIG_TAURI,
        };
    }
    flags
}

fn configuration_value_kind(value: &era_config::ConfigValue) -> ConfigurationValueKind {
    use era_config::ConfigValue;
    match value {
        ConfigValue::Boolean(_) => ConfigurationValueKind::Boolean,
        ConfigValue::Integer(_) => ConfigurationValueKind::Integer,
        ConfigValue::String(_) => ConfigurationValueKind::String,
        ConfigValue::Enum { .. } => ConfigurationValueKind::Enum,
        ConfigValue::Color(_) => ConfigurationValueKind::Color,
        ConfigValue::Character(_) => ConfigurationValueKind::Character,
        ConfigValue::IntegerList(_) => ConfigurationValueKind::IntegerList,
        ConfigValue::StringList(_) => ConfigurationValueKind::StringList,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct NormalizedResourceIdentity {
    pub(crate) relative_path: String,
    pub(crate) category: FileCategory,
    pub(crate) payload_digest: [u8; 32],
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct SemanticConfig {
    values: ConfigStore,
    csv: CsvLoadOptions,
    analyzer: AnalyzerOptions,
    auto_save: bool,
    ctrl_z_enabled: bool,
    allow_long_input_by_activation: bool,
    save_in_binary: bool,
    compress_save: bool,
    save_slot_count: u32,
    money_label: String,
    money_first: bool,
    maximum_shop_items: u32,
    viewport_width: u32,
    viewport_height: u32,
    font_size: u32,
    line_height: u32,
    print_c_per_line: u32,
    print_c_length: u32,
    legacy_encoding: LegacyEncoding,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            values: ConfigStore::default(),
            csv: CsvLoadOptions::default(),
            analyzer: AnalyzerOptions::default(),
            auto_save: true,
            ctrl_z_enabled: false,
            allow_long_input_by_activation: false,
            save_in_binary: false,
            compress_save: false,
            save_slot_count: 20,
            money_label: "$".into(),
            money_first: true,
            maximum_shop_items: 100,
            viewport_width: 760,
            viewport_height: 480,
            font_size: 18,
            line_height: 19,
            print_c_per_line: 3,
            print_c_length: 25,
            legacy_encoding: LegacyEncoding::Japanese,
        }
    }
}

// Keeping the pipeline in one function makes the atomic artifact/report outcome visible;
// conversion details live in the helpers below.
#[cfg(test)]
pub(crate) fn build_project(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    build_project_inner(manifest, previous, None)
}

#[cfg(test)]
pub(crate) fn build_project_with_extensions(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
) -> ProjectBuild {
    build_project_with_extensions_and_progress(
        manifest,
        previous,
        previous_artifact,
        extensions,
        ConfigurationClientProfile::Reference,
        None,
    )
}

pub(crate) fn build_project_with_extensions_and_progress(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    build_project_inner_with_extensions(
        manifest,
        previous,
        previous_artifact,
        None,
        false,
        extensions,
        configuration_profile,
        progress,
    )
}

pub(crate) fn analyze_submitted_project_with_extensions(
    request: &ProjectAnalysisRequest,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
) -> ProjectAnalysisReport {
    let selected = request
        .selected_erb_paths
        .iter()
        .filter_map(|path| validate_relative_path(path).ok())
        .map(|path| path.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    let build = build_project_inner_with_extensions(
        &request.manifest,
        None,
        None,
        Some(&selected),
        request.debug_mode,
        extensions,
        configuration_profile,
        None,
    );
    let mut analyzed_erb_paths = request
        .manifest
        .files
        .iter()
        .filter(|file| file.category == FileCategory::Erb)
        .filter_map(|file| validate_relative_path(&file.relative_path).ok())
        .filter(|path| selected.is_empty() || selected.contains(&path.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    analyzed_erb_paths.sort();
    ProjectAnalysisReport {
        project_revision: request.manifest.project_revision,
        success: build.report.success,
        diagnostics: build.report.diagnostics,
        analyzed_erb_paths,
    }
}

#[cfg(test)]
fn build_project_inner(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
) -> ProjectBuild {
    build_project_inner_with_extensions(
        manifest,
        previous,
        None,
        analysis_selection,
        false,
        &[],
        ConfigurationClientProfile::Reference,
        None,
    )
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the atomic build pipeline keeps all identity-affecting inputs explicit"
)]
fn build_project_inner_with_extensions(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
    analysis_debug_mode: bool,
    extension_declarations: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    let mut diagnostics = Vec::new();
    report_progress(
        progress,
        ProjectProgressStage::Normalizing,
        0,
        manifest.files.len(),
    );
    let source_texts = manifest_source_texts(manifest);
    let (extensions, host_registry, extension_map) =
        prepare_extensions(extension_declarations, &mut diagnostics);
    let mut files = manifest.files.clone();
    let parsed_configuration = parse_configuration(&files, &mut diagnostics);
    let mut config = parsed_configuration.semantic;
    let configuration_document = parsed_configuration.document;
    let generated_configuration_source = parsed_configuration.generated_source;
    if config.csv.sort_with_filename {
        files.sort_by(|left, right| {
            (!path_has_priority_directory(&left.relative_path))
                .cmp(&(!path_has_priority_directory(&right.relative_path)))
                .then_with(|| {
                    compare_reference_file_paths(&left.relative_path, &right.relative_path)
                })
                .then_with(|| (left.category as u8).cmp(&(right.category as u8)))
        });
    }
    let mut csv_files = ProjectFiles::default();
    let mut sources = Vec::new();
    let mut resources = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let file_count = files.len();
    for (file_index, mut file) in files.into_iter().enumerate() {
        let path = match validate_relative_path(&file.relative_path) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_path",
                    RuntimeLogLevel::Error,
                    error.message,
                    Some(SourceLocation {
                        relative_path: file.relative_path,
                        byte_start: 0,
                        byte_end: 0,
                        line: None,
                        byte_column: None,
                    }),
                ));
                report_fraction(
                    progress,
                    ProjectProgressStage::Normalizing,
                    file_index + 1,
                    file_count,
                );
                continue;
            }
        };
        file.relative_path.clone_from(&path);
        if !seen.insert((file.category as u8, path.to_ascii_lowercase())) {
            diagnostics.push(project_diagnostic(
                "runtime.duplicate_path",
                RuntimeLogLevel::Error,
                "duplicate normalized project path",
                Some(SourceLocation {
                    relative_path: path,
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            report_fraction(
                progress,
                ProjectProgressStage::Normalizing,
                file_index + 1,
                file_count,
            );
            continue;
        }
        let payload_digest = payload_hash(&file.payload);
        if let (Some(expected), Some(actual)) =
            (file.content_hash.as_ref(), payload_digest.as_ref())
            && expected.as_slice() != actual.as_bytes()
        {
            diagnostics.push(project_diagnostic(
                "runtime.content_hash_mismatch",
                RuntimeLogLevel::Error,
                "submitted content hash does not match the payload",
                Some(SourceLocation {
                    relative_path: path.clone(),
                    byte_start: 0,
                    byte_end: 0,
                    line: None,
                    byte_column: None,
                }),
            ));
            report_fraction(
                progress,
                ProjectProgressStage::Normalizing,
                file_index + 1,
                file_count,
            );
            continue;
        }
        match file.category {
            FileCategory::Csv => csv_files
                .csv
                .push(csv_file(category_relative_path(&path, "CSV"), file.payload)),
            FileCategory::Erh | FileCategory::Erb => {
                // The CSV loader consults the ERB root only for ERD deferred-index files.
                // Copying every ordinary script here doubled the resident source payload for
                // large projects before the analyzer had even started.
                if is_deferred_index_source(&path) {
                    csv_files.erb.push(csv_file(
                        category_relative_path(&path, "ERB"),
                        file.payload.clone(),
                    ));
                }
                if file.category == FileCategory::Erh
                    || analysis_selection.is_none_or(|selection| {
                        selection.is_empty() || selection.contains(&path.to_ascii_lowercase())
                    })
                {
                    sources.push(analyzer_source(path, file.payload));
                }
            }
            FileCategory::Configuration => {}
            FileCategory::ResourceManifest | FileCategory::Resource => {
                if let Some(identity) = normalize_resource(
                    &mut diagnostics,
                    path,
                    file.category,
                    &file.payload,
                    payload_digest,
                ) {
                    resources.push(identity);
                }
            }
        }
        report_fraction(
            progress,
            ProjectProgressStage::Normalizing,
            file_index + 1,
            file_count,
        );
    }

    report_progress(progress, ProjectProgressStage::LoadingData, 0, 1);
    let csv = load_project(&csv_files, &config.csv);
    report_progress(progress, ProjectProgressStage::LoadingData, 1, 1);
    diagnostics.extend(csv.diagnostics.iter().map(|diagnostic| ProtocolDiagnostic {
        code: format!("csv.{:?}", diagnostic.code).to_ascii_lowercase(),
        level: match diagnostic.severity {
            CsvDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
            CsvDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
            CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal => RuntimeLogLevel::Error,
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
    let Some(mut data) = csv.data else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    data.static_data.legacy_encoding = config.legacy_encoding;
    let editable_configuration = config.values.clone();
    apply_replace_configuration(&config.values, &mut data.static_data.replace);
    config
        .money_label
        .clone_from(&data.static_data.replace.money_label);
    config.money_first = data.static_data.replace.money_first;
    config.maximum_shop_items = u32::try_from(data.static_data.replace.max_shop_item).unwrap_or(0);
    let mut analyzer_options = config.analyzer.clone();
    if analysis_selection.is_some() {
        analyzer_options.analysis_mode = true;
        analyzer_options.debug_mode = analysis_debug_mode;
        analyzer_options.ignore_uncalled_functions = false;
    }
    let analysis_input = AnalysisInput {
        project_data: data,
        sources,
    };
    let analysis_progress = |event: erabasic_analyzer::AnalysisProgress| {
        report_progress(
            progress,
            match event.stage {
                AnalysisProgressStage::Parsing => ProjectProgressStage::Parsing,
                AnalysisProgressStage::Analyzing => ProjectProgressStage::Analyzing,
            },
            event.completed,
            event.total,
        );
    };
    let analysis = if progress.is_some() {
        analyze_project_with_progress(
            analysis_input,
            &analyzer_options,
            &extensions,
            &analysis_progress,
        )
    } else {
        analyze_project(analysis_input, &analyzer_options, &extensions)
    };
    diagnostics.extend(analysis.diagnostics.iter().map(|diagnostic| {
        let source = diagnostic.source.as_ref().map(|source| {
            let text = source_texts
                .get(&source.relative_path.to_ascii_lowercase())
                .copied();
            project_source_location(
                source.relative_path.clone(),
                source.byte_start,
                source.byte_end,
                Some(u64::from(source.physical_line)),
                text,
            )
        });
        ProtocolDiagnostic {
            code: format!("analyzer.{:?}", diagnostic.code).to_ascii_lowercase(),
            level: match diagnostic.severity {
                AnalyzerDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
                AnalyzerDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
                AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal => {
                    RuntimeLogLevel::Error
                }
            },
            message: diagnostic.message.clone(),
            source,
        }
    }));
    let Some(project) = analysis.project else {
        return failed(manifest.project_revision, diagnostics, previous);
    };
    if analysis_selection.is_some() {
        let success = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error);
        return ProjectBuild {
            artifact: None,
            incremental: IncrementalState::default(),
            report: ProjectLoadReport {
                project_revision: manifest.project_revision,
                success,
                diagnostics,
                payload_required: false,
                configuration: None,
            },
            snapshot: None,
        };
    }
    let compile_progress = |event: erabasic_compiler::CompileProgress| {
        report_progress(
            progress,
            match event.stage {
                erabasic_compiler::CompileProgressStage::Compiling => {
                    ProjectProgressStage::Compiling
                }
                erabasic_compiler::CompileProgressStage::Finalizing => {
                    ProjectProgressStage::Finalizing
                }
            },
            event.completed,
            event.total,
        );
    };
    let compile = if progress.is_some() {
        compile_validated_project_with_artifact_and_progress(
            &project,
            &CompilerOptions::default(),
            &host_registry,
            previous,
            previous_artifact,
            &compile_progress,
        )
    } else {
        compile_validated_project_with_artifact(
            &project,
            &CompilerOptions::default(),
            &host_registry,
            previous,
            previous_artifact,
        )
    };
    diagnostics.extend(compile.diagnostics.iter().map(|diagnostic| {
        let source = diagnostic.location.map(|location| {
            let relative_path = project
                .program
                .sources
                .iter()
                .find(|source| source.id == location.source)
                .map_or_else(String::new, |source| source.relative_path.clone());
            let text = source_texts
                .get(&relative_path.to_ascii_lowercase())
                .copied();
            project_source_location(
                relative_path,
                location.span.start,
                location.span.end,
                None,
                text,
            )
        });
        ProtocolDiagnostic {
            code: format!("compiler.{:?}", diagnostic.code).to_ascii_lowercase(),
            level: match diagnostic.severity {
                erabasic_compiler::CompilerDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
                erabasic_compiler::CompilerDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
                erabasic_compiler::CompilerDiagnosticSeverity::Error => RuntimeLogLevel::Error,
            },
            message: diagnostic.message.clone(),
            source,
        }
    }));
    let incremental = compile.incremental_state;
    let Some(artifact) = compile.artifact else {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    };
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error);
    if !success {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    }
    // The compiler structurally validated this exact in-process value before
    // refreshing its identities. Avoid repeating that full artifact walk at the
    // runtime boundary. Decoded or external bytecode still uses `validate_bytecode`.
    report_progress(progress, ProjectProgressStage::Validating, 0, 1);
    report_progress(progress, ProjectProgressStage::Validating, 1, 1);

    let preparing_total = ResourceGraph::work_item_count(manifest).saturating_add(2);
    report_progress(
        progress,
        ProjectProgressStage::Preparing,
        0,
        preparing_total,
    );
    resources.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| (left.category as u8).cmp(&(right.category as u8)))
    });
    report_fraction(
        progress,
        ProjectProgressStage::Preparing,
        1,
        preparing_total,
    );
    let (resource_graph, resource_diagnostics) =
        ResourceGraph::from_manifest_with_progress(manifest, |completed, _| {
            report_fraction(
                progress,
                ProjectProgressStage::Preparing,
                completed.saturating_add(1),
                preparing_total,
            );
        });
    diagnostics.extend(resource_diagnostics.into_iter().map(|diagnostic| {
        project_diagnostic(
            diagnostic.code,
            if diagnostic.error {
                RuntimeLogLevel::Error
            } else {
                RuntimeLogLevel::Warning
            },
            diagnostic.message,
            Some(SourceLocation {
                relative_path: diagnostic.path,
                byte_start: 0,
                byte_end: 0,
                line: diagnostic.line,
                byte_column: None,
            }),
        )
    }));
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error);
    if !success {
        return failed_with_incremental(manifest.project_revision, diagnostics, incremental);
    }
    let project_identity = project_identity(&artifact, &config, &resources, &extension_map);
    report_progress(
        progress,
        ProjectProgressStage::Preparing,
        preparing_total,
        preparing_total,
    );
    let mut normalized_manifest = manifest.clone();
    if let Some(contents) = &generated_configuration_source {
        let digest =
            era_protocol::ProtocolBytes::new(blake3::hash(contents.as_bytes()).as_bytes().to_vec());
        normalized_manifest.files.push(SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(contents.clone()),
            content_hash: Some(digest),
        });
    }
    ProjectBuild {
        artifact: Some(artifact),
        incremental,
        report: ProjectLoadReport {
            project_revision: manifest.project_revision,
            success,
            diagnostics,
            payload_required: false,
            configuration: None,
        },
        snapshot: Some(NormalizedProjectSnapshot {
            manifest: Arc::new(normalized_manifest),
            project_identity,
            resources,
            resource_graph,
            sort_with_filename: config.csv.sort_with_filename,
            auto_save: config.auto_save,
            ctrl_z_enabled: config.ctrl_z_enabled,
            allow_long_input_by_activation: config.allow_long_input_by_activation,
            save_in_binary: config.save_in_binary,
            compress_save: config.compress_save,
            save_slot_count: config.save_slot_count,
            money_label: config.money_label,
            money_first: config.money_first,
            maximum_shop_items: config.maximum_shop_items,
            viewport_width: config.viewport_width,
            viewport_height: config.viewport_height,
            font_size: config.font_size,
            line_height: config.line_height,
            print_c_per_line: config.print_c_per_line,
            print_c_length: config.print_c_length,
            configuration_profile,
            configuration: config.values,
            editable_configuration,
            configuration_document,
            generated_configuration_source,
            extensions: extension_map,
        }),
    }
}

fn report_progress(
    reporter: Option<&ProjectProgressReporter>,
    stage: ProjectProgressStage,
    completed: usize,
    total: usize,
) {
    if let Some(reporter) = reporter {
        reporter.report(ProjectProgress {
            stage,
            completed: u64::try_from(completed).unwrap_or(u64::MAX),
            total: u64::try_from(total).unwrap_or(u64::MAX),
        });
    }
}

fn report_fraction(
    reporter: Option<&ProjectProgressReporter>,
    stage: ProjectProgressStage,
    completed: usize,
    total: usize,
) {
    let percent = completed.saturating_mul(100).checked_div(total);
    let previous_percent = completed
        .saturating_sub(1)
        .saturating_mul(100)
        .checked_div(total);
    if total == 0 || completed == total || percent > previous_percent {
        report_progress(reporter, stage, completed, total);
    }
}

pub(crate) fn apply_project_delta(
    current: &ProjectManifest,
    reload: &ReloadProject,
) -> Result<ProjectManifest, String> {
    if reload.base_revision != current.project_revision {
        return Err("reload base revision differs from the loaded project".into());
    }
    if reload.target_revision <= reload.base_revision {
        return Err("reload target revision must increase monotonically".into());
    }
    let mut files = std::collections::BTreeMap::new();
    for file in &current.files {
        let path = validate_relative_path(&file.relative_path).map_err(|error| error.message)?;
        files.insert(
            (file.category as u8, path.to_ascii_lowercase()),
            file.clone(),
        );
    }
    let mut changed = std::collections::BTreeSet::new();
    for change in &reload.changes {
        let (category, path) = match change {
            FileChange::Upsert { file } => (file.category, file.relative_path.as_str()),
            FileChange::Remove {
                category,
                relative_path,
            } => (*category, relative_path.as_str()),
        };
        let path = validate_relative_path(path).map_err(|error| error.message)?;
        let identity = (category as u8, path.to_ascii_lowercase());
        if !changed.insert(identity.clone()) {
            return Err("reload contains duplicate changes for one normalized path".into());
        }
        match change {
            FileChange::Upsert { file } => {
                let mut file = file.clone();
                file.relative_path = path;
                files.insert(identity, file);
            }
            FileChange::Remove { .. } => {
                files.remove(&identity);
            }
        }
    }
    Ok(ProjectManifest {
        project_revision: reload.target_revision,
        files: files.into_values().collect(),
    })
}

fn normalize_resource(
    diagnostics: &mut Vec<ProtocolDiagnostic>,
    relative_path: String,
    category: FileCategory,
    payload: &FilePayload,
    payload_digest: Option<blake3::Hash>,
) -> Option<NormalizedResourceIdentity> {
    let location = Some(SourceLocation {
        relative_path: relative_path.clone(),
        byte_start: 0,
        byte_end: 0,
        line: None,
        byte_column: None,
    });
    match payload {
        FilePayload::Bytes(_) if category == FileCategory::ResourceManifest => {
            diagnostics.push(project_diagnostic(
                "runtime.expected_utf8",
                RuntimeLogLevel::Error,
                "resource manifests must be submitted as UTF-8",
                location,
            ));
            return None;
        }
        FilePayload::Utf8(_) | FilePayload::Bytes(_) => {}
        FilePayload::IoError(error) => {
            diagnostics.push(project_diagnostic(
                "runtime.frontend_io_error",
                RuntimeLogLevel::Error,
                format!("frontend resource read failed: {:?}", error.kind),
                location,
            ));
            return None;
        }
    }
    Some(NormalizedResourceIdentity {
        relative_path,
        category,
        payload_digest: *payload_digest?.as_bytes(),
    })
}

fn project_identity(
    artifact: &ValidatedArtifact,
    config: &SemanticConfig,
    resources: &[NormalizedResourceIdentity],
    extensions: &std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.runtime.project.v2");
    hasher.update(&artifact.artifact().manifest.artifact_id.bytes());
    hasher.update(&[
        u8::from(config.auto_save),
        u8::from(config.save_in_binary),
        u8::from(config.compress_save),
        u8::from(config.money_first),
        u8::from(config.ctrl_z_enabled),
        u8::from(config.allow_long_input_by_activation),
    ]);
    hasher.update(&config.save_slot_count.to_le_bytes());
    hasher.update(&config.maximum_shop_items.to_le_bytes());
    hasher.update(&config.viewport_width.to_le_bytes());
    hasher.update(&config.viewport_height.to_le_bytes());
    hasher.update(&config.font_size.to_le_bytes());
    hasher.update(&config.line_height.to_le_bytes());
    hasher.update(&config.print_c_per_line.to_le_bytes());
    hasher.update(&config.print_c_length.to_le_bytes());
    hasher.update(&(config.money_label.len() as u64).to_le_bytes());
    hasher.update(config.money_label.as_bytes());
    // GETCONFIG exposes the complete catalog, including frontend-only preferences.
    // Hash every canonical entry so two projects with observably different query
    // results can never share a snapshot or hot-reload identity.
    for (code, value) in config.values.iter() {
        hasher.update(&(code.len() as u64).to_le_bytes());
        hasher.update(code.as_bytes());
        let encoded = serde_json::to_vec(value).expect("configuration value serializes");
        hasher.update(&(encoded.len() as u64).to_le_bytes());
        hasher.update(&encoded);
    }
    for resource in resources {
        hasher.update(&(resource.relative_path.len() as u64).to_le_bytes());
        hasher.update(resource.relative_path.as_bytes());
        hasher.update(&[resource.category as u8]);
        hasher.update(&resource.payload_digest);
    }
    for (operation, declaration) in extensions {
        hasher.update(&(operation.len() as u64).to_le_bytes());
        hasher.update(operation.as_bytes());
        let encoded = serde_json::to_vec(declaration).expect("extension declaration serializes");
        hasher.update(&(encoded.len() as u64).to_le_bytes());
        hasher.update(&encoded);
    }
    *hasher.finalize().as_bytes()
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
            RuntimeLogLevel::Error,
            error.message.clone(),
            Some(location()),
        )),
        FilePayload::Bytes(_) if require_utf8 => diagnostics.push(project_diagnostic(
            "runtime.expected_utf8",
            RuntimeLogLevel::Error,
            "configuration and resource manifests must be submitted as UTF-8",
            Some(location()),
        )),
        FilePayload::Utf8(_) | FilePayload::Bytes(_) => diagnostics.push(project_diagnostic(
            code,
            RuntimeLogLevel::Warning,
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
            payload_required: false,
            configuration: None,
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
