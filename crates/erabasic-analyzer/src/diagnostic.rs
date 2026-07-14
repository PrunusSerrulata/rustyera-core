use erabasic_ast::{DiagnosticCode, Span};
use erabasic_hir::SourceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyzerDiagnosticSeverity {
    Notice,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyzerDiagnosticCode {
    InvalidPath,
    DuplicatePath,
    IoError,
    UnsupportedSource,
    Syntax,
    DuplicateSymbol,
    ReservedName,
    UnknownIdentifier,
    UnknownInstruction,
    UnknownFunction,
    InvalidDeclaration,
    InvalidDimension,
    InvalidInitializer,
    TypeMismatch,
    InvalidOperand,
    InvalidAssignment,
    InvalidArgumentCount,
    InvalidArgument,
    InvalidControlFlow,
    UndefinedLabel,
    UncalledFunction,
    UnsupportedReferenceFeature,
    DeferredIndex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerSourceLocation {
    pub source: SourceId,
    pub relative_path: String,
    pub physical_line: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerDiagnostic {
    pub code: AnalyzerDiagnosticCode,
    pub parser_code: Option<DiagnosticCode>,
    pub severity: AnalyzerDiagnosticSeverity,
    /// Emuera warning level, where level 2 is a normal load-time error.
    pub reference_level: u8,
    pub source: Option<AnalyzerSourceLocation>,
    pub message: String,
}

impl AnalyzerDiagnostic {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn at(
        code: AnalyzerDiagnosticCode,
        severity: AnalyzerDiagnosticSeverity,
        reference_level: u8,
        source: SourceId,
        path: &str,
        text: &str,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        let physical_line = u32::try_from(
            text[..span.start.min(text.len())]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count(),
        )
        .unwrap_or(u32::MAX);
        Self {
            code,
            parser_code: None,
            severity,
            reference_level,
            source: Some(AnalyzerSourceLocation {
                source,
                relative_path: path.to_owned(),
                physical_line,
                byte_start: span.start,
                byte_end: span.end,
            }),
            message: message.into(),
        }
    }

    pub(crate) fn project_fatal(
        code: AnalyzerDiagnosticCode,
        path: &str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            parser_code: None,
            severity: AnalyzerDiagnosticSeverity::Fatal,
            reference_level: 3,
            source: (!path.is_empty()).then(|| AnalyzerSourceLocation {
                source: SourceId::default(),
                relative_path: path.to_owned(),
                physical_line: 0,
                byte_start: 0,
                byte_end: 0,
            }),
            message: message.into(),
        }
    }
}
