//! Side-effect-free project compatibility resolution and diagnostic context.

use era_protocol::ProtocolVersion;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{CompatibilityIdentity, ProtocolBytes, ProtocolDiagnostic, ServiceKind, SubmittedFile};

/// Resolve only the root configuration before binding frontend storage or importing caches.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ResolveProjectCompatibility {
    #[n(0)]
    pub request_id: u64,
    #[n(1)]
    pub configuration: Option<SubmittedFile>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectCompatibilityResolved {
    #[n(0)]
    pub request_id: u64,
    /// None on any configuration error; the caller must not bind storage or load a project.
    #[n(1)]
    pub identity: Option<CompatibilityIdentity>,
    /// BLAKE3 of the submitted root configuration after BOM/line-ending normalization.
    #[n(2)]
    pub configuration_digest: Option<ProtocolBytes>,
    #[n(3)]
    pub diagnostics: Vec<ProtocolDiagnostic>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct RequiredCapability {
    #[n(0)]
    pub kind: ServiceKind,
    #[n(1)]
    pub operation: String,
    #[n(2)]
    pub version: ProtocolVersion,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct CompatibilityDiagnosticContext {
    #[n(0)]
    pub identity: Option<CompatibilityIdentity>,
    #[n(1)]
    pub stage: String,
    #[n(2)]
    pub api: Option<String>,
    #[n(3)]
    pub required_capability: Option<RequiredCapability>,
}
