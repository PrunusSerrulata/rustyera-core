use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolError, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{
    BreakpointUpdate, CallStack, ConsoleCommand, ConsoleOutcome, DebugGrant, DebugHello,
    DebugRevoke, DebugStop, FiberPage, GameFieldPage, GameFieldValue, GameFieldWrite,
    GameFieldWriteOutcome, GrantToken, OperandStackPage, ResolvedBreakpoint, StepKind, StopToken,
    VariablePage, VariableValue, VariableWrite, VariableWriteOutcome,
};

pub const DEBUG_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(3, 0);

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
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct AuthorizedDebugRequest {
    #[n(0)]
    pub grant: GrantToken,
    #[n(1)]
    pub command: DebugCommand,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DebugResponse {
    #[n(0)]
    Accepted,
    #[n(1)]
    VariablePage(#[n(0)] VariablePage),
    #[n(2)]
    VariableValue(#[n(0)] VariableValue),
    #[n(3)]
    GameFieldPage(#[n(0)] GameFieldPage),
    #[n(4)]
    GameFieldValue(#[n(0)] GameFieldValue),
    #[n(5)]
    FiberPage(#[n(0)] FiberPage),
    #[n(6)]
    CallStack(#[n(0)] CallStack),
    #[n(7)]
    OperandStack(#[n(0)] OperandStackPage),
    #[n(8)]
    Console(#[n(0)] ConsoleOutcome),
    #[n(9)]
    Breakpoints(#[n(0)] Vec<ResolvedBreakpoint>),
    #[n(10)]
    VariablesWritten(#[n(0)] VariableWriteOutcome),
    #[n(11)]
    GameFieldsWritten(#[n(0)] GameFieldWriteOutcome),
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum DebugErrorCode {
    #[n(0)]
    PermissionDenied,
    #[n(1)]
    InvalidState,
    #[n(2)]
    StaleStop,
    #[n(3)]
    StaleRevision,
    #[n(4)]
    UnknownTarget,
    #[n(5)]
    TypeMismatch,
    #[n(6)]
    UnsafeConsoleStatement,
    #[n(7)]
    ResourceLimit,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugError {
    #[n(0)]
    pub code: DebugErrorCode,
    #[n(1)]
    pub message: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum DebugMessage {
    #[n(0)]
    Hello(#[n(0)] DebugHello),
    #[n(1)]
    Grant(#[n(0)] DebugGrant),
    #[n(2)]
    Revoke(#[n(0)] DebugRevoke),
    #[n(10)]
    Request(#[n(0)] AuthorizedDebugRequest),
    #[n(11)]
    Response(#[n(0)] DebugResponse),
    #[n(12)]
    Stopped(#[n(0)] DebugStop),
    #[n(13)]
    Error(#[n(0)] DebugError),
}

impl DebugMessage {
    #[must_use]
    pub const fn tag(&self) -> u32 {
        match self {
            Self::Hello(_) => 0,
            Self::Grant(_) => 1,
            Self::Revoke(_) => 2,
            Self::Request(_) => 10,
            Self::Response(_) => 11,
            Self::Stopped(_) => 12,
            Self::Error(_) => 13,
        }
    }

    /// # Errors
    ///
    /// Returns an error if deterministic CBOR encoding fails.
    pub fn encode_payload(&self) -> Result<Vec<u8>, ProtocolError> {
        encode_canonical(self)
    }

    /// # Errors
    ///
    /// Returns an error if payload encoding fails.
    pub fn envelope(
        &self,
        session: Option<SessionId>,
        session_epoch: Option<era_protocol::SessionEpoch>,
        sequence: u64,
        message_id: u64,
        correlation_id: Option<u64>,
    ) -> Result<Envelope, ProtocolError> {
        let mut envelope = Envelope::new(
            Channel::Debug,
            DEBUG_PROTOCOL_VERSION,
            sequence,
            message_id,
            self.tag(),
            ProtocolBytes::new(self.encode_payload()?),
        );
        envelope.session = session;
        envelope.session_epoch = session_epoch;
        envelope.correlation_id = correlation_id;
        Ok(envelope)
    }

    /// # Errors
    ///
    /// Returns an error for malformed CBOR or an envelope/payload tag mismatch.
    pub fn decode_payload(tag: u32, bytes: &[u8]) -> Result<Self, ProtocolError> {
        let message: Self = decode_canonical(bytes)?;
        if message.tag() != tag {
            return Err(ProtocolError::new(
                ProtocolErrorCode::MessageTagMismatch,
                "debug envelope tag differs from its payload",
            ));
        }
        Ok(message)
    }

    /// # Errors
    ///
    /// Returns an error for invalid envelopes, the wrong channel, an unsupported
    /// debug major version or invalid payload data.
    pub fn from_envelope(envelope: &Envelope) -> Result<Self, ProtocolError> {
        envelope.validate()?;
        if envelope.channel != Channel::Debug {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ChannelMismatch,
                "runtime envelope cannot be decoded as a debug message",
            ));
        }
        if envelope.channel_version.major != DEBUG_PROTOCOL_VERSION.major {
            return Err(ProtocolError::new(
                ProtocolErrorCode::VersionMismatch,
                "unsupported debug protocol major version",
            ));
        }
        Self::decode_payload(envelope.payload_tag, envelope.payload.as_slice())
    }
}
