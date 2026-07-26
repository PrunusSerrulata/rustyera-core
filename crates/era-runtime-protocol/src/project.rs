use era_protocol::{ProtocolBytes, ProtocolError, ProtocolErrorCode};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::RuntimeLogLevel;

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum FrontendIoErrorKind {
    #[n(0)]
    NotFound,
    #[n(1)]
    PermissionDenied,
    #[n(2)]
    InvalidData,
    #[n(3)]
    Interrupted,
    #[n(4)]
    ReadOnly,
    #[n(5)]
    AlreadyExists,
    #[n(6)]
    Other,
    /// A `Missing` or `Revision` storage precondition did not hold.
    #[n(7)]
    Conflict,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct FrontendIoError {
    #[n(0)]
    pub kind: FrontendIoErrorKind,
    #[n(1)]
    pub message: String,
    #[n(2)]
    pub platform_code: Option<i64>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum FileCategory {
    #[n(0)]
    Csv,
    #[n(1)]
    Erh,
    #[n(2)]
    Erb,
    #[n(3)]
    ResourceManifest,
    #[n(4)]
    Resource,
    #[n(5)]
    Configuration,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum FilePayload {
    #[n(0)]
    Utf8(#[n(0)] String),
    #[n(1)]
    Bytes(#[n(0)] ProtocolBytes),
    #[n(2)]
    IoError(#[n(0)] FrontendIoError),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SubmittedFile {
    #[n(0)]
    pub relative_path: String,
    #[n(1)]
    pub category: FileCategory,
    #[n(2)]
    pub payload: FilePayload,
    #[n(3)]
    pub content_hash: Option<ProtocolBytes>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectManifest {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub files: Vec<SubmittedFile>,
}

/// Lightweight identity used to validate an opaque compiled-project cache before source
/// payloads are transferred.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectIdentity {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub source_digest: ProtocolBytes,
}

/// Load a project, optionally seeding the build from an opaque runtime-produced cache.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectLoadRequest {
    #[n(0)]
    pub identity: ProjectIdentity,
    #[n(1)]
    pub manifest: Option<ProjectManifest>,
    #[n(2)]
    pub compiled_cache_transfer_id: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SourceLocation {
    #[n(0)]
    pub relative_path: String,
    #[n(1)]
    pub byte_start: u64,
    #[n(2)]
    pub byte_end: u64,
    #[n(3)]
    pub line: Option<u64>,
    #[n(4)]
    pub byte_column: Option<u64>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProtocolDiagnostic {
    #[n(0)]
    pub code: String,
    #[n(1)]
    pub level: RuntimeLogLevel,
    #[n(2)]
    pub message: String,
    #[n(3)]
    pub source: Option<SourceLocation>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectLoadReport {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub success: bool,
    #[n(2)]
    pub diagnostics: Vec<ProtocolDiagnostic>,
    /// The supplied cache was not exact and the frontend must retry with a full manifest.
    #[n(3)]
    pub payload_required: bool,
}

/// One-shot project analysis that never replaces the active runtime project.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectAnalysisRequest {
    #[n(0)]
    pub manifest: ProjectManifest,
    #[n(1)]
    pub selected_erb_paths: Vec<String>,
    #[n(2)]
    pub debug_mode: bool,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectAnalysisReport {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub success: bool,
    #[n(2)]
    pub diagnostics: Vec<ProtocolDiagnostic>,
    #[n(3)]
    pub analyzed_erb_paths: Vec<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileChange {
    #[n(0)]
    Upsert {
        #[n(0)]
        file: SubmittedFile,
    },
    #[n(1)]
    Remove {
        #[n(0)]
        category: FileCategory,
        #[n(1)]
        relative_path: String,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ReloadProject {
    #[n(0)]
    pub base_revision: u64,
    #[n(1)]
    pub target_revision: u64,
    #[n(2)]
    pub changes: Vec<FileChange>,
}

/// Normalize a frontend path without ever consulting the platform filesystem.
///
/// # Errors
///
/// Rejects empty, absolute, drive-qualified and parent-traversing paths.
pub fn validate_relative_path(path: &str) -> Result<String, ProtocolError> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return Err(invalid_path());
    }
    let replaced = path.replace('\\', "/");
    if replaced.len() >= 2 && replaced.as_bytes()[1] == b':' {
        return Err(invalid_path());
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(invalid_path()),
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(invalid_path());
    }
    Ok(parts.join("/"))
}

fn invalid_path() -> ProtocolError {
    ProtocolError::new(
        ProtocolErrorCode::InvalidIdentifier,
        "paths must be non-empty relative paths without parent traversal",
    )
}
