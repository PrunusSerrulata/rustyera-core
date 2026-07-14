use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    UnsupportedVersion,
    UnsupportedFeature,
    ResourceLimit,
    DuplicateIdentity,
    MissingReference,
    InvalidSourceMap,
    InvalidHir,
    UnknownOpcode,
    InvalidOperand,
    InvalidControlFlow,
    StackMismatch,
    TypeMismatch,
    HostAbiMismatch,
    MissingCapability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationDiagnostic {
    pub code: ValidationCode,
    pub function: Option<String>,
    pub instruction: Option<u32>,
    pub message: String,
}

impl ValidationDiagnostic {
    pub(crate) fn project(code: ValidationCode, message: impl Into<String>) -> Self {
        Self {
            code,
            function: None,
            instruction: None,
            message: message.into(),
        }
    }

    pub(crate) fn instruction(
        code: ValidationCode,
        function: impl Into<String>,
        instruction: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            function: Some(function.into()),
            instruction: Some(u32::try_from(instruction).unwrap_or(u32::MAX)),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport<T> {
    pub value: Option<T>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl<T> ValidationReport<T> {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }
}
