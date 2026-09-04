
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
pub(crate) use self::frontend::{payload_hash, project_diagnostic};
#[cfg(test)]
use self::frontend::project_source_location;
use self::frontend::{
    analyzer_source, csv_file, index_input_error, indexed_project_source_location,
    indexed_source_record_location,
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
    if file.content_hash.is_none()
        && let Some(hash) = payload_hash(&file.payload)
    {
        file.content_hash = Some(era_protocol::ProtocolBytes::new(hash.as_bytes().to_vec()));
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
