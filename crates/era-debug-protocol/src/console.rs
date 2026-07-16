use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{DebugSourceLocation, DebugValue, GameFieldValue, StopToken, VariableValue};

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugDiagnostic {
    #[n(0)]
    pub code: String,
    #[n(1)]
    pub message: String,
    #[n(2)]
    pub source: Option<DebugSourceLocation>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConsoleCommand {
    #[n(0)]
    Evaluate {
        #[n(0)]
        source: String,
    },
    /// Execute only method-safe statements with no flow, wait, I/O or Host effect.
    #[n(1)]
    ExecuteSafe {
        #[n(0)]
        source: String,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ConsoleOutcome {
    #[n(0)]
    pub stop: StopToken,
    #[n(1)]
    pub value: Option<DebugValue>,
    #[n(2)]
    pub output: Vec<String>,
    #[n(3)]
    pub changed_variables: Vec<VariableValue>,
    #[n(4)]
    pub changed_game_fields: Vec<GameFieldValue>,
    #[n(5)]
    pub diagnostics: Vec<DebugDiagnostic>,
}
