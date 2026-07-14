use erabasic_data::ProjectData;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceIoErrorKind {
    NotFound,
    PermissionDenied,
    InvalidData,
    Interrupted,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceIoError {
    pub kind: SourceIoErrorKind,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SourcePayload {
    Utf8(String),
    IoError(SourceIoError),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectSource {
    pub relative_path: String,
    pub payload: SourcePayload,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalysisInput {
    pub project_data: ProjectData,
    pub sources: Vec<ProjectSource>,
}
