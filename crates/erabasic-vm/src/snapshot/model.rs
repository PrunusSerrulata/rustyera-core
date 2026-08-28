//! Public snapshot inspection and eligibility data, isolated from VM restore mechanics.

use serde::Serialize;
use serde_json::Value;

use crate::{FiberId, GenerationId};

/// Container metadata and decoded state from a validated execution snapshot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnapshotInspection {
    pub container: SnapshotContainerInspection,
    pub state: Value,
}

/// Header information from a validated execution snapshot container.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SnapshotContainerInspection {
    pub magic: String,
    pub format_version: u32,
    pub file_bytes: u64,
    pub compressed_payload_bytes: u64,
    pub uncompressed_payload_bytes: u64,
    pub payload_blake3: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotBlocker {
    PendingHotReload,
    PendingCompletionEvents,
    PrimaryFiberNotSnapshotStable,
    RunnableFiber(FiberId),
    TransientHostWait(FiberId),
    AwaitResume(FiberId),
    OldGenerationFrame(FiberId, GenerationId),
    LegacyGenerationState,
    NativeService(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotEligibility {
    Eligible,
    Ineligible(Vec<SnapshotBlocker>),
}
