//! Arity policy for non-variadic user calls, independent of argument evaluation.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserCallArgumentPolicy {
    #[default]
    RejectExcess,
    WarnAndIgnoreExcess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserCallArityDiagnostic {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserCallArityDecision {
    /// Actual slots retained for evaluation; missing formals are handled elsewhere.
    pub retained: usize,
    pub excess: usize,
    pub diagnostic: Option<UserCallArityDiagnostic>,
}

impl UserCallArityDecision {
    #[must_use]
    pub fn is_rejected(self) -> bool {
        self.diagnostic == Some(UserCallArityDiagnostic::Error)
    }
}

impl UserCallArgumentPolicy {
    /// Never apply this policy to builtin or variadic signatures.
    #[must_use]
    pub fn decide(self, supplied: usize, formal: usize) -> UserCallArityDecision {
        let excess = supplied.saturating_sub(formal);
        UserCallArityDecision {
            retained: supplied.min(formal),
            excess,
            diagnostic: (excess != 0).then_some(match self {
                Self::RejectExcess => UserCallArityDiagnostic::Error,
                Self::WarnAndIgnoreExcess => UserCallArityDiagnostic::Warning,
            }),
        }
    }
}
