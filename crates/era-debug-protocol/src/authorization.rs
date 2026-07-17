use std::collections::BTreeSet;

use era_protocol::{ProtocolVersion, SessionId, VersionRange};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Decode, Encode, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum DebugScope {
    #[n(0)]
    VariablesRead,
    #[n(1)]
    VariablesWrite,
    #[n(2)]
    GameFieldsRead,
    #[n(3)]
    GameFieldsWrite,
    #[n(4)]
    ExecutionRead,
    #[n(5)]
    ExecutionControl,
    #[n(6)]
    ConsoleEvaluate,
    #[n(7)]
    ConsoleExecute,
    #[n(8)]
    BreakpointsManage,
    #[n(9)]
    ScriptOutput,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugHello {
    #[n(0)]
    pub versions: VersionRange,
    #[n(1)]
    pub requested_scopes: Vec<DebugScope>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugGrant {
    #[n(0)]
    pub version: ProtocolVersion,
    #[n(1)]
    pub token: GrantToken,
    #[n(2)]
    pub scopes: Vec<DebugScope>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct GrantToken {
    #[n(0)]
    pub grant_id: SessionId,
    #[n(1)]
    pub session_epoch: u64,
    #[n(2)]
    pub program_generation: u64,
    #[n(3)]
    pub issued_runtime_revision: u64,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct DebugRevoke {
    #[n(0)]
    pub grant_id: SessionId,
    #[n(1)]
    pub reason: String,
}

/// Compute the deterministic intersection between immutable session policy and a
/// frontend request. A request can never widen the creator-provided policy.
#[must_use]
pub fn grant_scopes(policy: &[DebugScope], requested: &[DebugScope]) -> Vec<DebugScope> {
    let policy: BTreeSet<_> = policy.iter().copied().collect();
    requested
        .iter()
        .copied()
        .filter(|scope| policy.contains(scope))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
