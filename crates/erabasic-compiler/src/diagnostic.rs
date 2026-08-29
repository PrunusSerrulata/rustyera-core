use erabasic_hir::SourceLocation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerDiagnosticCode {
    InvalidHir,
    UnsupportedConstruct,
    MissingCapability,
    MissingImport,
    SymbolCollision,
    Parallelism,
    Encoding,
    Validation,
    FrontendObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerDiagnosticSeverity {
    Notice,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompilerDiagnostic {
    pub code: CompilerDiagnosticCode,
    pub severity: CompilerDiagnosticSeverity,
    pub location: Option<SourceLocation>,
    pub message: String,
}

impl CompilerDiagnostic {
    pub(crate) fn new(code: CompilerDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: CompilerDiagnosticSeverity::Error,
            location: None,
            message: message.into(),
        }
    }

    pub(crate) fn at(
        code: CompilerDiagnosticCode,
        location: SourceLocation,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: CompilerDiagnosticSeverity::Error,
            location: Some(location),
            message: message.into(),
        }
    }

    pub(crate) fn notice_at(
        code: CompilerDiagnosticCode,
        location: SourceLocation,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: CompilerDiagnosticSeverity::Notice,
            location: Some(location),
            message: message.into(),
        }
    }

    pub(crate) fn warning_at(
        code: CompilerDiagnosticCode,
        location: SourceLocation,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity: CompilerDiagnosticSeverity::Warning,
            location: Some(location),
            message: message.into(),
        }
    }
}
