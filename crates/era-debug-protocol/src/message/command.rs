use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{BreakpointUpdate, ConsoleCommand, GameFieldWrite, StepKind, StopToken, VariableWrite};

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DebugCommand {
    #[n(0)]
    Pause,
    #[n(1)]
    Continue {
        #[n(0)]
        stop: StopToken,
    },
    #[n(2)]
    Step {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        fiber_id: u64,
        #[n(2)]
        kind: StepKind,
    },
    #[n(10)]
    ListVariables {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        cursor: Option<u64>,
        #[n(2)]
        limit: u32,
    },
    #[n(11)]
    ReadVariable {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        value: crate::VariableReference,
    },
    #[n(12)]
    WriteVariables {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        writes: Vec<VariableWrite>,
    },
    #[n(20)]
    ListGameFields {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        cursor: Option<u64>,
        #[n(2)]
        limit: u32,
    },
    #[n(21)]
    ReadGameField {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        key: String,
    },
    #[n(22)]
    WriteGameFields {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        writes: Vec<GameFieldWrite>,
    },
    #[n(30)]
    ListFibers {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        cursor: Option<u64>,
        #[n(2)]
        limit: u32,
    },
    #[n(31)]
    ReadCallStack {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        fiber_id: u64,
    },
    #[n(32)]
    ReadOperandStack {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        fiber_id: u64,
        #[n(2)]
        frame_id: u64,
        #[n(3)]
        cursor: Option<u64>,
        #[n(4)]
        limit: u32,
    },
    #[n(40)]
    Console {
        #[n(0)]
        stop: StopToken,
        #[n(1)]
        command: ConsoleCommand,
    },
    #[n(50)]
    UpdateBreakpoints {
        #[n(0)]
        update: BreakpointUpdate,
    },
    #[n(60)]
    ReadScriptOutput {
        #[n(0)]
        cursor: u64,
        #[n(1)]
        limit: u32,
    },
    #[n(61)]
    SubscribeScriptOutput {
        #[n(0)]
        enabled: bool,
    },
}
