use era_runtime_protocol::{
    FileCategory, FilePayload, ProjectLoadReport, ProtocolDiagnostic, RuntimeLogLevel,
    SourceLocation,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::ValidatedArtifact;

use super::{NormalizedResourceIdentity, ProjectBuild, SemanticConfig};
use crate::project::frontend::project_diagnostic;

pub(super) fn normalize_resource(
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

pub(super) fn project_identity(
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

pub(super) fn inspect_deferred_file(
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

pub(super) fn failed(
    revision: u64,
    diagnostics: Vec<ProtocolDiagnostic>,
    previous: Option<&IncrementalState>,
) -> ProjectBuild {
    failed_with_incremental(revision, diagnostics, previous.cloned().unwrap_or_default())
}

pub(super) fn failed_with_incremental(
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

pub(super) fn path_has_priority_directory(path: &str) -> bool {
    path.replace('\\', "/")
        .split('/')
        .rev()
        .skip(1)
        .any(|component| component.contains('#'))
}
