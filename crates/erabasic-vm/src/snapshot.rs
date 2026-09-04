use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::Arc;

use erabasic_bytecode::{Digest, ProgramVersion};
use erabasic_validator::ValidatedArtifact;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Fiber, FiberId, FiberState, GenerationId, HostRebindRequest, HostWaitStability, Memory,
    NativeServiceRegistry, ProgramGeneration, Vm, VmConfig, VmError, VmHost,
};

mod model;
use codec::encode_snapshot_payload;
mod operand_provenance;
mod runtime_forms;

pub use self::model::{
    SnapshotBlocker, SnapshotContainerInspection, SnapshotEligibility, SnapshotInspection,
};

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"RERAVMS\0";
pub const SNAPSHOT_FORMAT_VERSION: u32 = 20;
const SNAPSHOT_HEADER_BYTES: usize = 60;
const SNAPSHOT_COMPRESSION_LEVEL: i32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VmSnapshot {
    format_version: u32,
    program_version: ProgramVersion,
    artifact_id: Digest,
    compatibility: erabasic_compat::CompatibilityIdentity,
    current_generation: GenerationId,
    memory: Memory,
    fibers: BTreeMap<FiberId, Fiber>,
    primary_fiber: Option<FiberId>,
    next_fiber: u64,
    next_frame: u64,
    next_request: u64,
    next_generation: u64,
    // A sorted pair list keeps native state deterministic and independent from
    // a serializer's map-key representation.
    native_states: Vec<(erabasic_bytecode::SymbolKey, Vec<u8>)>,
    compatibility_warning_sites:
        std::collections::BTreeSet<(GenerationId, erabasic_bytecode::SymbolKey, usize, u8)>,
}

#[derive(Serialize)]
struct VmSnapshotRef<'a> {
    format_version: u32,
    program_version: ProgramVersion,
    artifact_id: Digest,
    compatibility: erabasic_compat::CompatibilityIdentity,
    current_generation: GenerationId,
    memory: &'a Memory,
    fibers: &'a BTreeMap<FiberId, Fiber>,
    primary_fiber: Option<FiberId>,
    next_fiber: u64,
    next_frame: u64,
    next_request: u64,
    next_generation: u64,
    native_states: &'a [(erabasic_bytecode::SymbolKey, Vec<u8>)],
    compatibility_warning_sites:
        &'a std::collections::BTreeSet<(GenerationId, erabasic_bytecode::SymbolKey, usize, u8)>,
}

mod codec;
mod lifecycle;
pub use codec::inspect_snapshot;
