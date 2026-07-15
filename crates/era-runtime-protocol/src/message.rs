use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolError, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{
    AdvanceTime, ClientHello, ClientStateChanged, CommandRejected, DeviceStateChanged,
    EffectAcknowledgement, EffectBatch, ExitRequested, FrontendInput, PresentationDelta,
    PresentationSnapshot, ProjectLoadReport, ProjectManifest, ReloadProject, ResynchronizeRequest,
    RuntimeFault, RuntimePhase, RuntimeStateChanged, SequenceAcknowledgement, ServerHello,
    ServiceRequest, ServiceResponse, ShutdownReady, ShutdownRequest, StartRequest,
    StateExportReady, StateExportRequest, StorageRequest, StorageResponse, VersionRejected,
    WaitChange,
};

pub const RUNTIME_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(5, 0);

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RuntimeResynchronized {
    #[n(0)]
    pub epoch: u64,
    #[n(1)]
    pub phase: RuntimePhase,
    #[n(2)]
    pub runtime_revision: u64,
    #[n(3)]
    pub presentation: PresentationSnapshot,
    #[n(4)]
    pub exit_requested: Option<ExitRequested>,
}

/// Stable runtime message variants. Numeric discriminants are wire IDs and must
/// never be reused, even after a message is retired.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeMessage {
    #[n(0)]
    ClientHello(#[n(0)] ClientHello),
    #[n(1)]
    ServerHello(#[n(0)] ServerHello),
    #[n(2)]
    VersionRejected(#[n(0)] VersionRejected),
    #[n(10)]
    ProjectManifest(#[n(0)] ProjectManifest),
    #[n(11)]
    ProjectLoadReport(#[n(0)] ProjectLoadReport),
    #[n(12)]
    ReloadProject(#[n(0)] ReloadProject),
    #[n(20)]
    Start(#[n(0)] StartRequest),
    #[n(21)]
    StateChanged(#[n(0)] RuntimeStateChanged),
    #[n(22)]
    ExitRequested(#[n(0)] ExitRequested),
    #[n(30)]
    Input(#[n(0)] FrontendInput),
    #[n(31)]
    AdvanceTime(#[n(0)] AdvanceTime),
    #[n(32)]
    WaitChanged(#[n(0)] WaitChange),
    #[n(33)]
    DeviceStateChanged(#[n(0)] DeviceStateChanged),
    #[n(34)]
    ClientStateChanged(#[n(0)] ClientStateChanged),
    #[n(40)]
    PresentationSnapshot(#[n(0)] PresentationSnapshot),
    #[n(41)]
    PresentationDelta(#[n(0)] PresentationDelta),
    #[n(42)]
    EffectBatch(#[n(0)] EffectBatch),
    #[n(43)]
    EffectAcknowledgement(#[n(0)] EffectAcknowledgement),
    #[n(50)]
    StorageRequest(#[n(0)] StorageRequest),
    #[n(51)]
    StorageResponse(#[n(0)] StorageResponse),
    #[n(52)]
    ServiceRequest(#[n(0)] ServiceRequest),
    #[n(53)]
    ServiceResponse(#[n(0)] ServiceResponse),
    #[n(60)]
    StateExportRequest(#[n(0)] StateExportRequest),
    #[n(61)]
    StateExportReady(#[n(0)] StateExportReady),
    #[n(90)]
    ShutdownRequest(#[n(0)] ShutdownRequest),
    #[n(91)]
    ShutdownReady(#[n(0)] ShutdownReady),
    #[n(92)]
    Fault(#[n(0)] RuntimeFault),
    #[n(93)]
    Acknowledge(#[n(0)] SequenceAcknowledgement),
    #[n(94)]
    Resynchronize(#[n(0)] ResynchronizeRequest),
    #[n(95)]
    CommandRejected(#[n(0)] CommandRejected),
    #[n(96)]
    RuntimeResynchronized(#[n(0)] RuntimeResynchronized),
}

impl RuntimeMessage {
    #[must_use]
    pub const fn tag(&self) -> u32 {
        match self {
            Self::ClientHello(_) => 0,
            Self::ServerHello(_) => 1,
            Self::VersionRejected(_) => 2,
            Self::ProjectManifest(_) => 10,
            Self::ProjectLoadReport(_) => 11,
            Self::ReloadProject(_) => 12,
            Self::Start(_) => 20,
            Self::StateChanged(_) => 21,
            Self::ExitRequested(_) => 22,
            Self::Input(_) => 30,
            Self::AdvanceTime(_) => 31,
            Self::WaitChanged(_) => 32,
            Self::DeviceStateChanged(_) => 33,
            Self::ClientStateChanged(_) => 34,
            Self::PresentationSnapshot(_) => 40,
            Self::PresentationDelta(_) => 41,
            Self::EffectBatch(_) => 42,
            Self::EffectAcknowledgement(_) => 43,
            Self::StorageRequest(_) => 50,
            Self::StorageResponse(_) => 51,
            Self::ServiceRequest(_) => 52,
            Self::ServiceResponse(_) => 53,
            Self::StateExportRequest(_) => 60,
            Self::StateExportReady(_) => 61,
            Self::ShutdownRequest(_) => 90,
            Self::ShutdownReady(_) => 91,
            Self::Fault(_) => 92,
            Self::Acknowledge(_) => 93,
            Self::Resynchronize(_) => 94,
            Self::CommandRejected(_) => 95,
            Self::RuntimeResynchronized(_) => 96,
        }
    }

    /// Encode one typed payload for placement in an [`era_protocol::Envelope`].
    ///
    /// # Errors
    ///
    /// Returns an error if CBOR encoding fails.
    pub fn encode_payload(&self) -> Result<Vec<u8>, ProtocolError> {
        encode_canonical(self)
    }

    /// Wrap this payload in the common versioned runtime envelope.
    ///
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
            Channel::Runtime,
            RUNTIME_PROTOCOL_VERSION,
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

    /// Decode a payload and verify that the envelope tag agrees with the embedded
    /// stable message discriminant.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed CBOR or a tag mismatch.
    pub fn decode_payload(tag: u32, bytes: &[u8]) -> Result<Self, ProtocolError> {
        let message: Self = decode_canonical(bytes)?;
        if message.tag() != tag {
            return Err(ProtocolError::new(
                ProtocolErrorCode::MessageTagMismatch,
                "runtime envelope tag differs from its payload",
            ));
        }
        Ok(message)
    }

    /// Decode a common envelope as a runtime message and reject channel/version drift.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid envelopes, the wrong channel, unsupported major
    /// versions or invalid payloads.
    pub fn from_envelope(envelope: &Envelope) -> Result<Self, ProtocolError> {
        envelope.validate()?;
        if envelope.channel != Channel::Runtime {
            return Err(ProtocolError::new(
                ProtocolErrorCode::ChannelMismatch,
                "debug envelope cannot be decoded as a runtime message",
            ));
        }
        if envelope.channel_version.major != RUNTIME_PROTOCOL_VERSION.major {
            return Err(ProtocolError::new(
                ProtocolErrorCode::VersionMismatch,
                "unsupported runtime protocol major version",
            ));
        }
        Self::decode_payload(envelope.payload_tag, envelope.payload.as_slice())
    }
}
