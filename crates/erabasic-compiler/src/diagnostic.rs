use erabasic_hir::SourceLocation;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilerDiagnosticCode {
    InvalidHir,
    UnsupportedConstruct,
    MissingImport,
    SymbolCollision,
    Parallelism,
    Encoding,
    Validation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompilerDiagnostic {
    pub code: CompilerDiagnosticCode,
    pub location: Option<SourceLocation>,
    pub message: String,
}

impl CompilerDiagnostic {
    pub(crate) fn new(code: CompilerDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            code,
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
            location: Some(location),
            message: message.into(),
        }
    }
}
