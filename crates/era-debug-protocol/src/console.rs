use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{DebugValue, GameFieldValue, VariableValue};

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
    pub value: Option<DebugValue>,
    #[n(1)]
    pub output: Vec<String>,
    #[n(2)]
    pub changed_variables: Vec<VariableValue>,
    #[n(3)]
    pub changed_game_fields: Vec<GameFieldValue>,
    #[n(4)]
    pub diagnostics: Vec<String>,
}
