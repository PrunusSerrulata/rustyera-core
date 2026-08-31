mod configuration;
mod extensions;
mod frontend;
mod model;
mod support;
#[cfg(test)]
mod tests;

use era_runtime_protocol::{
    ConfigurationClientProfile, DiagnosticNotification, FileCategory, FileChange, FilePayload,
    ProjectAnalysisReport, ProjectAnalysisRequest, ProjectGameInformation, ProjectLoadReport,
    ProjectManifest, ProtocolDiagnostic, ReloadProject, RuntimeLogLevel, SourceLocation,
    SubmittedFile, validate_relative_path,
};
use erabasic_analyzer::{
    AnalysisInput, AnalysisProgressStage, AnalyzerDiagnosticSeverity, analyze_project,
    analyze_project_with_progress, compare_reference_file_paths,
};
use erabasic_bytecode::BytecodeArtifact;
use erabasic_compiler::{
    CompilerOptions, IncrementalState, compile_owned_validated_project_with_artifact,
    compile_owned_validated_project_with_artifact_and_progress,
};
use erabasic_csv::{CsvDiagnosticSeverity, CsvLoadReport, ProjectFiles, load_project_owned};
use erabasic_validator::ValidatedArtifact;
use std::sync::Arc;

use crate::resource::ResourceGraph;
use crate::{ProjectProgress, ProjectProgressReporter, ProjectProgressStage};

use self::configuration::{apply_replace_configuration, parse_configuration};
use self::extensions::{category_relative_path, prepare_extensions};
pub(crate) use self::frontend::project_diagnostic;
#[cfg(test)]
use self::frontend::project_source_location;
use self::frontend::{
    analyzer_source, csv_file, index_input_error, indexed_project_source_location,
    indexed_source_record_location, payload_hash,
};
use self::model::SemanticConfig;
pub(crate) use self::model::{
    NormalizedProjectSnapshot, NormalizedResourceIdentity, profile_applicability,
    profile_application, profile_preference_eligible,
};
use self::support::{
    failed, failed_with_incremental, inspect_deferred_file, normalize_resource,
    path_has_priority_directory, project_identity,
};

pub(crate) const fn project_diagnostic_notification(
    level: RuntimeLogLevel,
) -> DiagnosticNotification {
    if matches!(level, RuntimeLogLevel::Warning) {
        DiagnosticNotification::LogOnly
    } else {
        DiagnosticNotification::Default
    }
}

pub(crate) fn is_root_configuration_file(file: &SubmittedFile) -> bool {
    file.category == FileCategory::Configuration
        && validate_relative_path(&file.relative_path)
            .is_ok_and(|path| path.eq_ignore_ascii_case("reraconfig.toml"))
}

pub(crate) fn project_configuration_values(files: &[SubmittedFile]) -> era_config::ConfigStore {
    let mut diagnostics = Vec::new();
    configuration::parse_configuration(files, &mut diagnostics)
        .semantic
        .values
}

pub(crate) fn project_configuration_source_digest(
    files: &[SubmittedFile],
) -> era_protocol::ProtocolBytes {
    files
        .iter()
        .find(|file| is_root_configuration_file(file))
        .and_then(|file| match &file.payload {
            FilePayload::Utf8(text) => Some(era_protocol::ProtocolBytes::new(
                blake3::hash(era_config::normalize_line_endings(text).as_bytes())
                    .as_bytes()
                    .to_vec(),
            )),
            _ => None,
        })
        .unwrap_or_else(|| era_protocol::ProtocolBytes::new(Vec::new()))
}

fn release_manifest_payloads(
    manifest: &mut ProjectManifest,
    release: impl Fn(FileCategory) -> bool,
) {
    for file in &mut manifest.files {
        if !release(file.category) || is_root_configuration_file(file) {
            continue;
        }
        ensure_manifest_hash(file);
        match &mut file.payload {
            FilePayload::Utf8(value) => {
                *value = String::new();
            }
            FilePayload::Bytes(value) => {
                *value = era_protocol::ProtocolBytes::new(Vec::new());
            }
            FilePayload::IoError(_) | FilePayload::ExternalResource(_) => {}
        }
    }
}

pub(crate) fn release_snapshot_manifest_payloads(snapshot: &mut NormalizedProjectSnapshot) {
    let manifest = Arc::get_mut(&mut snapshot.manifest)
        .expect("a newly built project snapshot must uniquely own its manifest");
    release_manifest_payloads(manifest, compiled_cache_omits_payload);
}

fn compiled_cache_omits_payload(category: FileCategory) -> bool {
    !matches!(
        category,
        FileCategory::Configuration | FileCategory::ResourceManifest
    )
}

fn ensure_manifest_hash(file: &mut SubmittedFile) {
    match &file.payload {
        FilePayload::Utf8(value) => {
            file.content_hash.get_or_insert_with(|| {
                era_protocol::ProtocolBytes::new(blake3::hash(value.as_bytes()).as_bytes().to_vec())
            });
        }
        FilePayload::Bytes(value) => {
            file.content_hash.get_or_insert_with(|| {
                era_protocol::ProtocolBytes::new(blake3::hash(value.as_slice()).as_bytes().to_vec())
            });
        }
        FilePayload::IoError(_) | FilePayload::ExternalResource(_) => {}
    }
}

fn take_manifest_payload(file: &mut SubmittedFile) -> FilePayload {
    ensure_manifest_hash(file);
    match &file.payload {
        FilePayload::Utf8(_) => {
            std::mem::replace(&mut file.payload, FilePayload::Utf8(String::new()))
        }
        FilePayload::Bytes(_) => std::mem::replace(
            &mut file.payload,
            FilePayload::Bytes(era_protocol::ProtocolBytes::new(Vec::new())),
        ),
        FilePayload::IoError(_) | FilePayload::ExternalResource(_) => file.payload.clone(),
    }
}

pub(crate) struct ProjectBuild {
    pub(crate) artifact: Option<ValidatedArtifact>,
    pub(crate) incremental: IncrementalState,
    pub(crate) report: ProjectLoadReport,
    pub(crate) snapshot: Option<NormalizedProjectSnapshot>,
}

pub(crate) fn project_game_information(artifact: &ValidatedArtifact) -> ProjectGameInformation {
    let game_base = &artifact.artifact().project_data.static_data.game_base;
    let present = |value: &str| (!value.trim().is_empty()).then(|| value.to_owned());
    ProjectGameInformation {
        title: present(&game_base.title),
        author: present(&game_base.author),
        // An explicitly declared zero is still project metadata; an absent version is not.
        version: game_base
            .version_defined
            .then(|| game_base.script_version_text()),
        year: present(&game_base.year),
        information: present(&game_base.info),
    }
}

pub(crate) fn refresh_project_identity(
    snapshot: &mut NormalizedProjectSnapshot,
    artifact: &ValidatedArtifact,
) {
    snapshot.project_identity = project_identity_for_configuration(
        snapshot,
        artifact,
        snapshot.editable_configuration.clone(),
    );
}

pub(crate) fn project_identity_for_configuration(
    snapshot: &NormalizedProjectSnapshot,
    artifact: &ValidatedArtifact,
    configuration: era_config::ConfigStore,
) -> [u8; 32] {
    let mut config = configuration::semantic_config(configuration);
    config.money_label.clone_from(&snapshot.money_label);
    config.money_first = snapshot.money_first;
    config.maximum_shop_items = snapshot.maximum_shop_items;
    project_identity(artifact, &config, &snapshot.resources, &snapshot.extensions)
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

#[cfg(test)]
pub(crate) fn build_project_with_extensions_and_progress(
    manifest: &ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    build_project_inner_with_extensions(
        manifest.clone(),
        previous,
        previous_artifact,
        None,
        false,
        extensions,
        configuration_profile,
        true,
        progress,
    )
}

pub(crate) fn build_owned_project_with_extensions_and_progress(
    manifest: ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    extensions: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    retain_project_source_payloads: bool,
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
        retain_project_source_payloads,
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
        request.manifest.clone(),
        None,
        None,
        Some(&selected),
        request.debug_mode,
        extensions,
        configuration_profile,
        true,
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
        compatibility: build.report.compatibility.clone(),
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
        manifest.clone(),
        previous,
        None,
        analysis_selection,
        false,
        &[],
        ConfigurationClientProfile::Reference,
        true,
        None,
    )
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the atomic build pipeline keeps all identity-affecting inputs explicit"
)]
fn build_project_inner_with_extensions(
    manifest: ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
    analysis_debug_mode: bool,
    extension_declarations: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    retain_project_source_payloads: bool,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    let compatibility = manifest.compatibility.clone();
    let validation = compatibility
        .validate()
        .map_err(|error| {
            crate::compatibility::configuration_error(
                "runtime.unsupported_compatibility_identity",
                error.to_string(),
                None,
            )
        })
        .and_then(|()| {
            crate::compatibility::resolve_manifest_compatibility(&manifest).and_then(
                |(resolved, _)| {
                    if resolved == compatibility {
                        Ok(())
                    } else {
                        Err(crate::compatibility::configuration_error(
                            "runtime.compatibility_identity_mismatch",
                            format!(
                                "manifest profile {} does not match root configuration profile {}",
                                compatibility.profile, resolved.profile
                            ),
                            None,
                        ))
                    }
                },
            )
        });
    let mut build = match validation {
        Err(diagnostic) => failed(manifest.project_revision, vec![*diagnostic], previous),
        Ok(()) => build_project_with_resolved_compatibility(
            manifest,
            previous,
            previous_artifact,
            analysis_selection,
            analysis_debug_mode,
            extension_declarations,
            configuration_profile,
            retain_project_source_payloads,
            progress,
        ),
    };
    build.report.compatibility = Some(compatibility.clone());
    for diagnostic in &mut build.report.diagnostics {
        crate::compatibility::attach_diagnostic_identity(diagnostic, &compatibility);
    }
    if compatibility.is_experimental() && build.report.success {
        build
            .report
            .diagnostics
            .push(crate::compatibility::experimental_profile_diagnostic(
                &compatibility,
            ));
    }
    build
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_project_with_resolved_compatibility(
    manifest: ProjectManifest,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    analysis_selection: Option<&std::collections::BTreeSet<String>>,
    analysis_debug_mode: bool,
    extension_declarations: &[era_runtime_protocol::ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
    retain_project_source_payloads: bool,
    progress: Option<&ProjectProgressReporter>,
) -> ProjectBuild {
    // Cold loads already own the decoded manifest. Keep that allocation as the authoritative
    // reload snapshot so the end of compilation does not clone every source payload at once.
    let mut normalized_manifest = manifest;
    let mut diagnostics = Vec::new();
    report_progress(
        progress,
        ProjectProgressStage::Normalizing,
        0,
        normalized_manifest.files.len(),
    );
    let (extensions, host_registry, extension_map) =
        prepare_extensions(extension_declarations, &mut diagnostics);
    let configuration_source_digest =
        project_configuration_source_digest(&normalized_manifest.files);
    let mut files = if retain_project_source_payloads {
        normalized_manifest.files.clone()
    } else {
        normalized_manifest
            .files
            .iter_mut()
            .map(|file| {
                let payload = if !is_root_configuration_file(file)
                    && matches!(
                        file.category,
                        FileCategory::Erb
                            | FileCategory::Erh
                            | FileCategory::Csv
                            | FileCategory::Als
                            | FileCategory::Erd
                    ) {
                    take_manifest_payload(file)
                } else {
                    file.payload.clone()
                };
                SubmittedFile {
                    relative_path: file.relative_path.clone(),
                    category: file.category,
                    payload,
                    content_hash: file.content_hash.clone(),
                }
            })
            .collect()
    };
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
    let erd_alias_paths = files
        .iter()
        .filter(|file| file.category == FileCategory::Erd)
        .filter_map(|file| validate_relative_path(&file.relative_path).ok())
        .filter_map(|path| {
            path.rsplit_once('.')
                .map(|(stem, _)| format!("{}.als", stem.to_ascii_lowercase()))
        })
        .collect::<std::collections::BTreeSet<_>>();
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
        if let Some(error) = index_input_error(&file) {
            diagnostics.push(error);
            report_fraction(
                progress,
                ProjectProgressStage::Normalizing,
                file_index + 1,
                file_count,
            );
            continue;
        }
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
        let payload_digest = match &file.payload {
            FilePayload::ExternalResource(_) => file.content_hash.as_ref().and_then(|digest| {
                <[u8; 32]>::try_from(digest.as_slice())
                    .ok()
                    .map(blake3::Hash::from_bytes)
            }),
            payload => payload_hash(payload),
        };
        if matches!(file.payload, FilePayload::ExternalResource(_)) && payload_digest.is_none() {
            diagnostics.push(project_diagnostic(
                "runtime.invalid_external_resource_digest",
                RuntimeLogLevel::Error,
                "external resources require a 32-byte content hash",
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
        let actual_hash = payload_hash(&file.payload);
        if let (Some(expected), Some(actual)) = (file.content_hash.as_ref(), actual_hash.as_ref())
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
            FileCategory::Csv => csv_files.csv.push(csv_file(
                category_relative_path(&path, "CSV"),
                path,
                file.payload,
            )),
            FileCategory::Erd => csv_files.erb.push(csv_file(
                category_relative_path(&path, "ERB"),
                path,
                file.payload,
            )),
            FileCategory::Als => {
                // Alias identity includes its data root, not only its file stem.
                let erb_root = path
                    .split('/')
                    .next()
                    .is_some_and(|root| root.eq_ignore_ascii_case("ERB"))
                    || erd_alias_paths.contains(&path.to_ascii_lowercase());
                let root = if erb_root { "ERB" } else { "CSV" };
                let file = csv_file(category_relative_path(&path, root), path, file.payload);
                if erb_root {
                    csv_files.erb.push(file);
                } else {
                    csv_files.csv.push(file);
                }
            }
            FileCategory::Erh | FileCategory::Erb => {
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
    let CsvLoadReport {
        data,
        diagnostics: csv_diagnostics,
    } = load_project_owned(csv_files, &config.csv);
    report_progress(progress, ProjectProgressStage::LoadingData, 1, 1);
    diagnostics.extend(csv_diagnostics.into_iter().map(|diagnostic| {
        let level = match diagnostic.severity {
            CsvDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
            CsvDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
            CsvDiagnosticSeverity::Error | CsvDiagnosticSeverity::Fatal => RuntimeLogLevel::Error,
        };
        ProtocolDiagnostic {
            context: None,
            code: format!("csv.{:?}", diagnostic.code).to_ascii_lowercase(),
            level,
            message: diagnostic.message,
            source: diagnostic.source.map(|source| SourceLocation {
                relative_path: source.relative_path,
                byte_start: source.byte_start as u64,
                byte_end: source.byte_end as u64,
                line: Some(u64::from(source.physical_line)),
                byte_column: None,
            }),
            notification: project_diagnostic_notification(level),
        }
    }));
    let Some(mut data) = data else {
        return failed(normalized_manifest.project_revision, diagnostics, previous);
    };
    data.static_data.legacy_encoding = config.legacy_encoding;
    let editable_configuration = config.values.clone();
    let client_configuration = config.values.clone();
    apply_replace_configuration(&config.values, &mut data.static_data.replace);
    config
        .money_label
        .clone_from(&data.static_data.replace.money_label);
    config.money_first = data.static_data.replace.money_first;
    config.maximum_shop_items = u32::try_from(data.static_data.replace.max_shop_item).unwrap_or(0);
    let mut analyzer_options = config.analyzer.clone();
    analyzer_options.compatibility = normalized_manifest.compatibility.clone();
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
                AnalysisProgressStage::DeclaringGlobals
                | AnalysisProgressStage::IndexingFunctions
                | AnalysisProgressStage::DeclaringLocals
                | AnalysisProgressStage::Analyzing => ProjectProgressStage::Analyzing,
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
    let erabasic_analyzer::AnalysisReport {
        project,
        diagnostics: analysis_diagnostics,
    } = analysis;
    diagnostics.extend(analysis_diagnostics.into_iter().map(|diagnostic| {
        let source = diagnostic.source.map(|source| {
            let indexed = project.as_ref().and_then(|project| {
                project.program.sources.iter().find(|candidate| {
                    candidate
                        .relative_path
                        .eq_ignore_ascii_case(&source.relative_path)
                })
            });
            indexed_project_source_location(
                source.relative_path,
                source.byte_start,
                source.byte_end,
                Some(u64::from(source.physical_line)),
                indexed,
            )
        });
        let level = match diagnostic.severity {
            AnalyzerDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
            AnalyzerDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
            AnalyzerDiagnosticSeverity::Error | AnalyzerDiagnosticSeverity::Fatal => {
                RuntimeLogLevel::Error
            }
        };
        let excess_arguments =
            diagnostic.code == erabasic_analyzer::AnalyzerDiagnosticCode::ExcessUserArguments;
        ProtocolDiagnostic {
            context: excess_arguments.then(|| {
                Box::new(era_runtime_protocol::CompatibilityDiagnosticContext {
                    artifact: None,
                    project_load_id: None,
                    runtime_epoch: None,
                    generation: None,
                    identity: Some(analyzer_options.compatibility.clone()),
                    stage: "compat".into(),
                    api: Some("user_call".into()),
                    required_capability: None,
                })
            }),
            code: if excess_arguments {
                "compat.call.excess_arguments".into()
            } else {
                format!("analyzer.{:?}", diagnostic.code).to_ascii_lowercase()
            },
            level,
            message: diagnostic.message,
            source,
            notification: project_diagnostic_notification(level),
        }
    }));
    let Some(project) = project else {
        return failed(normalized_manifest.project_revision, diagnostics, previous);
    };
    if analysis_selection.is_some() {
        let success = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error);
        return ProjectBuild {
            artifact: None,
            incremental: IncrementalState::default(),
            report: ProjectLoadReport {
                compatibility: None,
                project_revision: normalized_manifest.project_revision,
                success,
                diagnostics,
                payload_required: false,
                configuration: None,
                game_information: None,
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
    let erabasic_compiler::OwnedValidatedCompileReport {
        report: compile,
        source_ids,
        diagnostic_sources,
    } = if progress.is_some() {
        compile_owned_validated_project_with_artifact_and_progress(
            project,
            &CompilerOptions::default(),
            &host_registry,
            previous,
            previous_artifact,
            &compile_progress,
        )
    } else {
        compile_owned_validated_project_with_artifact(
            project,
            &CompilerOptions::default(),
            &host_registry,
            previous,
            previous_artifact,
        )
    };
    let erabasic_compiler::ValidatedCompileReport {
        artifact,
        incremental_state: incremental,
        diagnostics: compile_diagnostics,
        patch: _,
        stats: _,
    } = compile;
    let compile_sources = artifact
        .as_ref()
        .map_or(diagnostic_sources.as_slice(), |artifact| {
            artifact.artifact().source_map.sources.as_slice()
        });
    let compile_source_index = frontend::CompilerSourceIndex::new(&source_ids);
    diagnostics.extend(compile_diagnostics.into_iter().map(|diagnostic| {
        let source = diagnostic.location.map(|location| {
            let indexed = compile_source_index.get(compile_sources, location.source);
            let relative_path =
                indexed.map_or_else(String::new, |source| source.relative_path.clone());
            indexed_source_record_location(
                relative_path,
                location.span.start,
                location.span.end,
                indexed,
            )
        });
        let level = match diagnostic.severity {
            erabasic_compiler::CompilerDiagnosticSeverity::Notice => RuntimeLogLevel::Info,
            erabasic_compiler::CompilerDiagnosticSeverity::Warning => RuntimeLogLevel::Warning,
            erabasic_compiler::CompilerDiagnosticSeverity::Error => RuntimeLogLevel::Error,
        };
        ProtocolDiagnostic {
            context: None,
            code: format!("compiler.{:?}", diagnostic.code).to_ascii_lowercase(),
            level,
            message: diagnostic.message,
            source,
            notification: project_diagnostic_notification(level),
        }
    }));
    drop(compile_source_index);
    drop(source_ids);
    drop(diagnostic_sources);
    let Some(artifact) = artifact else {
        return failed_with_incremental(
            normalized_manifest.project_revision,
            diagnostics,
            incremental,
        );
    };
    let success = !diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == RuntimeLogLevel::Error);
    if !success {
        return failed_with_incremental(
            normalized_manifest.project_revision,
            diagnostics,
            incremental,
        );
    }
    if let Some(contents) = &generated_configuration_source {
        let digest =
            era_protocol::ProtocolBytes::new(blake3::hash(contents.as_bytes()).as_bytes().to_vec());
        let generated_file = SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(contents.clone()),
            content_hash: Some(digest),
        };
        if let Some(existing) = normalized_manifest
            .files
            .iter_mut()
            .find(|file| is_root_configuration_file(file))
        {
            *existing = generated_file;
        } else {
            normalized_manifest.files.push(generated_file);
        }
    }
    if !retain_project_source_payloads {
        release_manifest_payloads(&mut normalized_manifest, |category| {
            matches!(
                category,
                FileCategory::Erb
                    | FileCategory::Erh
                    | FileCategory::Csv
                    | FileCategory::Als
                    | FileCategory::Erd
            )
        });
    }
    // The compiler structurally validated this exact in-process value before
    // refreshing its identities. Avoid repeating that full artifact walk at the
    // runtime boundary. Decoded or external bytecode still uses `validate_bytecode`.
    report_progress(progress, ProjectProgressStage::Validating, 0, 1);
    report_progress(progress, ProjectProgressStage::Validating, 1, 1);

    let preparing_total = ResourceGraph::work_item_count(&normalized_manifest).saturating_add(2);
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
        ResourceGraph::from_manifest_with_progress(&normalized_manifest, |completed, _| {
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
        return failed_with_incremental(
            normalized_manifest.project_revision,
            diagnostics,
            incremental,
        );
    }
    let project_identity = project_identity(&artifact, &config, &resources, &extension_map);
    report_progress(
        progress,
        ProjectProgressStage::Preparing,
        preparing_total,
        preparing_total,
    );
    if !retain_project_source_payloads {
        release_manifest_payloads(&mut normalized_manifest, compiled_cache_omits_payload);
    }
    let game_information = project_game_information(&artifact);
    ProjectBuild {
        artifact: Some(artifact),
        incremental,
        report: ProjectLoadReport {
            compatibility: None,
            project_revision: normalized_manifest.project_revision,
            success,
            diagnostics,
            payload_required: false,
            configuration: None,
            game_information: Some(Box::new(game_information)),
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
            client_configuration,
            editable_configuration,
            configuration_document,
            configuration_source_digest,
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
        compatibility: current.compatibility.clone(),
        project_revision: reload.target_revision,
        files: files.into_values().collect(),
    })
}
