use std::collections::{BTreeMap, VecDeque};

use erabasic_bytecode::{Digest, ProgramVersion};
use erabasic_validator::ValidatedArtifact;
use serde::{Deserialize, Serialize};

use crate::{
    Fiber, FiberId, FiberState, GenerationId, HostRebindRequest, HostWaitStability, Memory,
    NativeServiceRegistry, ProgramGeneration, Vm, VmConfig, VmError, VmHost,
};

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"RERAVMS\0";
pub const SNAPSHOT_FORMAT_VERSION: u32 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotBlocker {
    PendingHotReload,
    PrimaryFiberNotAtStableInput,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VmSnapshot {
    format_version: u32,
    program_version: ProgramVersion,
    artifact_id: Digest,
    current_generation: GenerationId,
    memory: Memory,
    fibers: BTreeMap<FiberId, Fiber>,
    primary_fiber: Option<FiberId>,
    next_fiber: u64,
    next_frame: u64,
    next_request: u64,
    next_generation: u64,
    // JSON object keys cannot losslessly represent a 128-bit SymbolKey. A sorted
    // pair list keeps the snapshot deterministic and format-independent.
    native_states: Vec<(erabasic_bytecode::SymbolKey, Vec<u8>)>,
}

impl VmSnapshot {
    #[must_use]
    pub const fn program_version(&self) -> ProgramVersion {
        self.program_version
    }

    #[must_use]
    pub const fn artifact_id(&self) -> Digest {
        self.artifact_id
    }

    /// Encode a deterministic, checksummed snapshot container without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot payload cannot be serialized.
    pub fn encode(&self) -> Result<Vec<u8>, VmError> {
        let payload =
            serde_json::to_vec(self).map_err(|error| VmError::Snapshot(error.to_string()))?;
        let mut bytes = Vec::with_capacity(8 + 4 + 8 + 32 + payload.len());
        bytes.extend_from_slice(&SNAPSHOT_MAGIC);
        bytes.extend_from_slice(&SNAPSHOT_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    /// Decode and checksum a snapshot container without performing I/O.
    ///
    /// # Errors
    ///
    /// Returns an error for limits, malformed headers, checksum failures, unsupported
    /// versions, or invalid serialized payloads.
    pub fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<Self, VmError> {
        if bytes.len() > maximum_bytes {
            return Err(VmError::Snapshot(
                "snapshot exceeds the configured limit".into(),
            ));
        }
        if bytes.len() < 52 || bytes[..8] != SNAPSHOT_MAGIC {
            return Err(VmError::Snapshot("invalid snapshot header".into()));
        }
        let version = u32::from_le_bytes(
            bytes[8..12]
                .try_into()
                .map_err(|_| VmError::Snapshot("truncated snapshot version".into()))?,
        );
        if version != SNAPSHOT_FORMAT_VERSION {
            return Err(VmError::Snapshot(format!(
                "unsupported snapshot format version {version}"
            )));
        }
        let length = u64::from_le_bytes(
            bytes[12..20]
                .try_into()
                .map_err(|_| VmError::Snapshot("truncated snapshot length".into()))?,
        );
        let length = usize::try_from(length)
            .map_err(|_| VmError::Snapshot("snapshot length exceeds this platform".into()))?;
        if bytes.len() != 52usize.saturating_add(length) {
            return Err(VmError::Snapshot("snapshot length is inconsistent".into()));
        }
        let payload = &bytes[52..];
        if blake3::hash(payload).as_bytes() != &bytes[20..52] {
            return Err(VmError::Snapshot("snapshot checksum differs".into()));
        }
        let snapshot: Self = serde_json::from_slice(payload)
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(VmError::Snapshot(
                "snapshot payload version differs from its container".into(),
            ));
        }
        Ok(snapshot)
    }
}

impl Vm {
    #[must_use]
    pub fn snapshot_eligibility(&self, natives: &NativeServiceRegistry) -> SnapshotEligibility {
        let mut blockers = Vec::new();
        if self.pending_reload.is_some() {
            blockers.push(SnapshotBlocker::PendingHotReload);
        }
        let primary_stable = self
            .primary_fiber
            .and_then(|id| self.fibers.get(&id))
            .is_some_and(|fiber| {
                matches!(
                    fiber.state,
                    FiberState::WaitingHost(crate::WaitingHost {
                        stability: HostWaitStability::StableInput,
                        ..
                    })
                )
            });
        if !primary_stable {
            blockers.push(SnapshotBlocker::PrimaryFiberNotAtStableInput);
        }
        for (id, fiber) in &self.fibers {
            match &fiber.state {
                FiberState::Runnable => blockers.push(SnapshotBlocker::RunnableFiber(*id)),
                FiberState::WaitingHost(wait) if wait.stability == HostWaitStability::Transient => {
                    blockers.push(SnapshotBlocker::TransientHostWait(*id));
                }
                FiberState::WaitingResume(_) => blockers.push(SnapshotBlocker::AwaitResume(*id)),
                FiberState::WaitingHost(_)
                | FiberState::Completed(_)
                | FiberState::Faulted(_)
                | FiberState::Cancelled => {}
            }
            for frame in &fiber.frames {
                if frame.generation != self.current_generation {
                    blockers.push(SnapshotBlocker::OldGenerationFrame(*id, frame.generation));
                }
            }
        }
        if !self.memory.legacy.is_empty() {
            blockers.push(SnapshotBlocker::LegacyGenerationState);
        }
        if let Err(error) = natives.snapshots() {
            blockers.push(SnapshotBlocker::NativeService(error));
        }
        if blockers.is_empty() {
            SnapshotEligibility::Eligible
        } else {
            SnapshotEligibility::Ineligible(blockers)
        }
    }

    /// Capture the VM at a stable input wait.
    ///
    /// # Errors
    ///
    /// Returns an error when any fiber, reload, generation, host wait, or native
    /// service makes the current state unstable.
    pub fn snapshot(&self, natives: &NativeServiceRegistry) -> Result<VmSnapshot, VmError> {
        if let SnapshotEligibility::Ineligible(blockers) = self.snapshot_eligibility(natives) {
            return Err(VmError::Snapshot(format!(
                "VM is not at a stable snapshot point: {blockers:?}"
            )));
        }
        Ok(VmSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            program_version: self.artifact().manifest.program_version,
            artifact_id: self.artifact_id(),
            current_generation: self.current_generation,
            memory: self.memory.clone(),
            fibers: self.fibers.clone(),
            primary_fiber: self.primary_fiber,
            next_fiber: self.next_fiber,
            next_frame: self.next_frame,
            next_request: self.next_request,
            next_generation: self.next_generation,
            native_states: natives
                .snapshots()
                .map_err(VmError::Snapshot)?
                .into_iter()
                .collect(),
        })
    }

    /// Restore only against the exact artifact identity. Native state is rolled
    /// back if the host rejects its atomic wait-rebind batch.
    ///
    /// # Errors
    ///
    /// Returns an error for an artifact mismatch, invalid snapshot state, unavailable
    /// native service, or failed atomic host rebind.
    pub fn restore_snapshot(
        artifact: ValidatedArtifact,
        config: VmConfig,
        snapshot: VmSnapshot,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
    ) -> Result<Self, VmError> {
        let expected = artifact.artifact();
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION
            || snapshot.artifact_id != expected.manifest.artifact_id
            || snapshot.program_version != expected.manifest.program_version
        {
            return Err(VmError::Snapshot(
                "snapshot does not match the exact bytecode artifact".into(),
            ));
        }
        validate_snapshot(&snapshot, expected, config)?;
        let rebinds = snapshot
            .fibers
            .iter()
            .filter_map(|(fiber, state)| match &state.state {
                FiberState::WaitingHost(wait) => Some(HostRebindRequest {
                    id: wait.request,
                    fiber: *fiber,
                    import: wait.import.clone(),
                    payload: wait.rebind_payload.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        let previous_native = natives.snapshots().map_err(VmError::Snapshot)?;
        let native_states = snapshot.native_states.iter().cloned().collect();
        natives
            .restore_snapshots(&native_states)
            .map_err(VmError::Snapshot)?;
        if let Err(error) = host.rebind_snapshot(&rebinds) {
            let _ = natives.restore_snapshots(&previous_native);
            return Err(VmError::Snapshot(error));
        }

        let artifact = artifact.into_inner();
        let generation = snapshot.current_generation;
        Ok(Self {
            config,
            generations: BTreeMap::from([(generation, ProgramGeneration { artifact })]),
            current_generation: generation,
            memory: snapshot.memory,
            fibers: snapshot.fibers,
            runnable: VecDeque::new(),
            primary_fiber: snapshot.primary_fiber,
            next_fiber: snapshot.next_fiber,
            next_frame: snapshot.next_frame,
            next_request: snapshot.next_request,
            next_generation: snapshot.next_generation,
            pending_reload: None,
        })
    }
}

#[allow(clippy::too_many_lines)]
fn validate_snapshot(
    snapshot: &VmSnapshot,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    config: VmConfig,
) -> Result<(), VmError> {
    if !snapshot.memory.legacy.is_empty() {
        return Err(VmError::Snapshot(
            "stable snapshots cannot contain legacy-generation storage".into(),
        ));
    }
    let primary_stable = snapshot
        .primary_fiber
        .and_then(|id| snapshot.fibers.get(&id))
        .is_some_and(|fiber| {
            matches!(
                fiber.state,
                FiberState::WaitingHost(crate::WaitingHost {
                    stability: HostWaitStability::StableInput,
                    ..
                })
            )
        });
    if !primary_stable || snapshot.fibers.len() > config.maximum_fibers {
        return Err(VmError::Snapshot(
            "snapshot does not have one stable primary fiber".into(),
        ));
    }
    let mut frame_ids = std::collections::BTreeSet::new();
    let mut request_ids = std::collections::BTreeSet::new();
    for (fiber_id, fiber) in &snapshot.fibers {
        if fiber.id != *fiber_id || fiber.frames.len() > config.maximum_call_depth {
            return Err(VmError::Snapshot(
                "snapshot fiber identity or call depth is invalid".into(),
            ));
        }
        if matches!(
            fiber.state,
            FiberState::Runnable | FiberState::WaitingResume(_)
        ) {
            return Err(VmError::Snapshot(
                "snapshot contains a non-stable fiber".into(),
            ));
        }
        if let FiberState::WaitingHost(wait) = &fiber.state
            && wait.stability != HostWaitStability::StableInput
        {
            return Err(VmError::Snapshot(
                "snapshot contains a transient host wait".into(),
            ));
        }
        if let FiberState::WaitingHost(wait) = &fiber.state {
            let valid = request_ids.insert(wait.request)
                && artifact.host_imports.iter().any(|import| {
                    import.import == wait.import && import.import.result == wait.result
                });
            if !valid {
                return Err(VmError::Snapshot(
                    "snapshot host wait does not match an artifact import".into(),
                ));
            }
        }
        for frame in &fiber.frames {
            if !frame_ids.insert(frame.id) || frame.stack.len() > config.maximum_operand_stack {
                return Err(VmError::Snapshot(
                    "snapshot frame identity or stack size is invalid".into(),
                ));
            }
            if frame.generation != snapshot.current_generation {
                return Err(VmError::Snapshot(
                    "snapshot contains an old-generation frame".into(),
                ));
            }
            let Some(function) = artifact
                .functions
                .iter()
                .find(|function| function.key == frame.function)
            else {
                return Err(VmError::Snapshot(
                    "snapshot frame function is missing".into(),
                ));
            };
            if frame.instruction > function.code.len() {
                return Err(VmError::Snapshot(
                    "snapshot frame instruction is out of bounds".into(),
                ));
            }
            for definition in artifact.globals.iter().filter(|definition| {
                definition.storage == erabasic_bytecode::BytecodeStorage::FunctionLocal
                    && definition.owner == Some(function.key)
            }) {
                let Some(cell) = frame.locals.get(&definition.key) else {
                    return Err(VmError::Snapshot(format!(
                        "snapshot local {} is missing",
                        definition.name
                    )));
                };
                validate_cell(cell, definition)?;
            }
        }
    }
    for definition in &artifact.globals {
        let cell = match definition.storage {
            erabasic_bytecode::BytecodeStorage::Project
            | erabasic_bytecode::BytecodeStorage::Constant
            | erabasic_bytecode::BytecodeStorage::Calculated => {
                snapshot.memory.shared.get(&definition.key)
            }
            erabasic_bytecode::BytecodeStorage::FunctionStatic
            | erabasic_bytecode::BytecodeStorage::FunctionPersistent => {
                snapshot.memory.statics.get(&definition.key)
            }
            erabasic_bytecode::BytecodeStorage::FunctionLocal
            | erabasic_bytecode::BytecodeStorage::Character => continue,
        };
        if cell.is_none() {
            return Err(VmError::Snapshot(format!(
                "snapshot variable {} differs from the artifact layout",
                definition.name
            )));
        }
        validate_cell(cell.expect("cell was checked"), definition)?;
    }
    for character in &snapshot.memory.characters {
        for definition in artifact.globals.iter().filter(|definition| {
            definition.storage == erabasic_bytecode::BytecodeStorage::Character
        }) {
            let Some(cell) = character.get(&definition.key) else {
                return Err(VmError::Snapshot(format!(
                    "snapshot character variable {} is missing",
                    definition.name
                )));
            };
            validate_cell(cell, definition)?;
        }
    }
    let maximum_fiber = snapshot.fibers.keys().map(|id| id.0).max().unwrap_or(0);
    let maximum_frame = frame_ids.iter().map(|id| id.0).max().unwrap_or(0);
    let maximum_request = request_ids.iter().map(|id| id.0).max().unwrap_or(0);
    if snapshot.next_fiber <= maximum_fiber
        || snapshot.next_frame <= maximum_frame
        || snapshot.next_request <= maximum_request
        || snapshot.next_generation <= snapshot.current_generation.0
    {
        return Err(VmError::Snapshot(
            "snapshot identity counters would reuse an existing id".into(),
        ));
    }
    Ok(())
}

fn validate_cell(
    cell: &crate::VariableCell,
    definition: &erabasic_bytecode::BytecodeGlobal,
) -> Result<(), VmError> {
    let expected_length = definition
        .dimensions
        .iter()
        .try_fold(1u64, |length, dimension| length.checked_mul(*dimension))
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0);
    if cell.value_type != definition.value_type
        || cell.dimensions != definition.dimensions
        || cell.values.len() != expected_length
        || cell
            .values
            .iter()
            .any(|value| value.value_type() != definition.value_type)
    {
        return Err(VmError::Snapshot(format!(
            "snapshot variable {} has invalid storage",
            definition.name
        )));
    }
    Ok(())
}
