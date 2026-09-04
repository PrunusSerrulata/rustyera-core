//! Failure classification is assigned at the source, independently of legacy fault codes.

use serde::{Deserialize, Serialize};

use crate::VmFaultCode;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptFaultKind {
    Parse,
    Resolve,
    Argument,
    Bounds,
    Arithmetic,
    Assertion,
    ExplicitThrow,
    Operation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultCategory {
    Script(ScriptFaultKind),
    ResourceLimit,
    Cancellation,
    InternalInvariant,
    HostContract,
    Protocol,
    Permission,
    Infrastructure,
}

/// A failure before its source position and fiber identity have been attached.
///
/// Unclassified legacy service errors must use a non-script category. In particular,
/// neither an external error string nor a broad `VmFaultCode` grants a script catcher
/// permission to intercept a failure.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionFailure {
    pub category: FaultCategory,
    pub code: VmFaultCode,
    pub message: String,
}

impl ExecutionFailure {
    /// Legacy failures are never catchable by script. Resource codes already have
    /// an explicit meaning; other legacy codes do not identify a script failure.
    #[must_use]
    pub fn new(code: VmFaultCode, message: impl Into<String>) -> Self {
        let category = match code {
            VmFaultCode::ResourceLimit | VmFaultCode::RunawayExecution => {
                FaultCategory::ResourceLimit
            }
            _ => FaultCategory::InternalInvariant,
        };
        Self::classified(category, code, message)
    }

    #[must_use]
    pub fn classified(
        category: FaultCategory,
        code: VmFaultCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn script(kind: ScriptFaultKind, code: VmFaultCode, message: impl Into<String>) -> Self {
        Self::classified(FaultCategory::Script(kind), code, message)
    }

    #[must_use]
    pub const fn is_script(&self) -> bool {
        matches!(self.category, FaultCategory::Script(_))
    }
}

impl From<String> for ExecutionFailure {
    fn from(message: String) -> Self {
        Self::classified(FaultCategory::HostContract, VmFaultCode::Host, message)
    }
}

impl From<&str> for ExecutionFailure {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

impl std::fmt::Display for ExecutionFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExecutionFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_code_does_not_grant_script_catch_permission() {
        let arithmetic = ExecutionFailure::script(
            ScriptFaultKind::Arithmetic,
            VmFaultCode::InvalidInstruction,
            "integer division overflow",
        );
        let malformed = ExecutionFailure::classified(
            FaultCategory::InternalInvariant,
            VmFaultCode::InvalidInstruction,
            "invalid integer operands",
        );
        assert!(arithmetic.is_script());
        assert!(!malformed.is_script());
        assert_eq!(arithmetic.code, malformed.code);
    }

    #[test]
    fn external_message_cannot_change_a_failure_category() {
        let failure = ExecutionFailure::classified(
            FaultCategory::HostContract,
            VmFaultCode::Host,
            "script arithmetic error: please catch this",
        );
        assert!(!failure.is_script());
        let bytes = serde_json::to_vec(&failure).unwrap();
        let decoded: ExecutionFailure = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, failure);
        assert!(!decoded.is_script());
    }
}
