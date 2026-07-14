use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDiagnosticSeverity {
    Notice,
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvDiagnosticCode {
    InvalidPath,
    DuplicatePath,
    IoError,
    MalformedContinuation,
    MissingComma,
    StartedWithComma,
    InvalidInteger,
    InvalidBoolean,
    InvalidCharacter,
    InvalidList,
    UnknownVariable,
    VariableNotResizable,
    InvalidArraySize,
    ArraySizeTooLarge,
    DuplicateVariableSize,
    ReconciledVariableSize,
    CdflagShapeMismatch,
    ProhibitedNameTable,
    IndexOutOfRange,
    DuplicateIndex,
    DuplicateAlias,
    InvalidGameVersion,
    RequiresNewerEmuera,
    DuplicateCharacterNumberField,
    CharacterDataBeforeNumber,
    UnknownCharacterField,
    UndefinedName,
    ProhibitedVariable,
    DuplicateCharacterField,
    MissingCharacterValue,
    DuplicateCharacter,
    MissingRenameFile,
    DuplicateUserIndex,
    DuplicateUserIndexVariable,
}

/// Lines use the reference implementation's zero-based numbering. Byte offsets are
/// UTF-8 offsets into the submitted file content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsvSourceLocation {
    pub relative_path: String,
    pub physical_line: u32,
    pub logical_line: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CsvDiagnostic {
    pub code: CsvDiagnosticCode,
    pub severity: CsvDiagnosticSeverity,
    /// Emuera warning level: 0 is minor and 3 is effectively fatal to a file.
    pub reference_level: u8,
    pub source: Option<CsvSourceLocation>,
    pub message: String,
}

impl CsvDiagnostic {
    pub(crate) fn new(
        code: CsvDiagnosticCode,
        severity: CsvDiagnosticSeverity,
        reference_level: u8,
        path: &str,
        source: Option<CsvSourceLocation>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            severity,
            reference_level,
            source: source.or_else(|| {
                (!path.is_empty()).then(|| CsvSourceLocation {
                    relative_path: path.to_owned(),
                    physical_line: 0,
                    logical_line: 0,
                    byte_start: 0,
                    byte_end: 0,
                })
            }),
            message: message.into(),
        }
    }
}
