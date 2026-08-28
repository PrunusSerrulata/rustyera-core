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

pub use self::model::{
    SnapshotBlocker, SnapshotContainerInspection, SnapshotEligibility, SnapshotInspection,
};

pub const SNAPSHOT_MAGIC: [u8; 8] = *b"RERAVMS\0";
pub const SNAPSHOT_FORMAT_VERSION: u32 = 14;
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
    arithmetic_warning_sites:
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
    arithmetic_warning_sites:
        &'a std::collections::BTreeSet<(GenerationId, erabasic_bytecode::SymbolKey, usize, u8)>,
}

impl VmSnapshot {
    #[must_use]
    pub fn compatibility(&self) -> &erabasic_compat::CompatibilityIdentity {
        &self.compatibility
    }

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
        encode_snapshot_payload(self)
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
        if bytes.len() < SNAPSHOT_HEADER_BYTES || bytes[..8] != SNAPSHOT_MAGIC {
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
        let uncompressed_length = u64::from_le_bytes(
            bytes[20..28]
                .try_into()
                .map_err(|_| VmError::Snapshot("truncated snapshot raw length".into()))?,
        );
        let uncompressed_length = usize::try_from(uncompressed_length)
            .map_err(|_| VmError::Snapshot("snapshot raw length exceeds this platform".into()))?;
        if uncompressed_length > maximum_bytes {
            return Err(VmError::Snapshot(
                "snapshot expands beyond the configured limit".into(),
            ));
        }
        if bytes.len() != SNAPSHOT_HEADER_BYTES.saturating_add(length) {
            return Err(VmError::Snapshot("snapshot length is inconsistent".into()));
        }
        let payload = &bytes[SNAPSHOT_HEADER_BYTES..];
        if blake3::hash(payload).as_bytes() != &bytes[28..SNAPSHOT_HEADER_BYTES] {
            return Err(VmError::Snapshot("snapshot checksum differs".into()));
        }
        // Trust neither the claimed raw length nor the compressed stream: cap
        // decompression one byte past the declared length so malformed payloads
        // cannot turn snapshot restore into an unbounded allocation path.
        let decompression_limit = (uncompressed_length as u64).saturating_add(1);
        let decoder = zstd::stream::read::Decoder::new(payload)
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        let mut reader = CountingReader::new(decoder.take(decompression_limit));
        let snapshot: Self = rmp_serde::from_read(&mut reader)
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        let mut tail = [0_u8; 1];
        if reader
            .read(&mut tail)
            .map_err(|error| VmError::Snapshot(error.to_string()))?
            != 0
            || reader.bytes != uncompressed_length as u64
        {
            return Err(VmError::Snapshot(
                "snapshot raw length is inconsistent".into(),
            ));
        }
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(VmError::Snapshot(
                "snapshot payload version differs from its container".into(),
            ));
        }
        snapshot
            .compatibility
            .validate()
            .map_err(|error| VmError::Snapshot(error.to_string()))?;
        Ok(snapshot)
    }
}

/// Validate and project an execution snapshot into a serialization-friendly tree.
///
/// Opaque byte payloads are represented by their length and BLAKE3 digest. This
/// keeps inspection output useful without copying native or host-owned data into
/// logs. Artifact-dependent restore checks remain the caller's responsibility.
///
/// # Errors
///
/// Returns the same errors as [`VmSnapshot::decode`], or an error if the decoded
/// state cannot be projected as JSON.
pub fn inspect_snapshot(bytes: &[u8], maximum_bytes: usize) -> Result<SnapshotInspection, VmError> {
    let snapshot = VmSnapshot::decode(bytes, maximum_bytes)?;
    let mut state = serde_json::to_value(&snapshot)
        .map_err(|error| VmError::Snapshot(format!("cannot inspect snapshot state: {error}")))?;
    normalize_digest_field(&mut state, "artifact_id");
    normalize_named_binary_fields(&mut state);
    normalize_native_states(&mut state);
    Ok(SnapshotInspection {
        container: SnapshotContainerInspection {
            magic: "RERAVMS\\0".into(),
            format_version: SNAPSHOT_FORMAT_VERSION,
            file_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            compressed_payload_bytes: read_header_u64(bytes, 12),
            uncompressed_payload_bytes: read_header_u64(bytes, 20),
            payload_blake3: hex_bytes(&bytes[28..SNAPSHOT_HEADER_BYTES]),
        },
        state,
    })
}

fn read_header_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated snapshot header contains this field"),
    )
}

fn normalize_digest_field(value: &mut Value, field: &str) {
    let Some(value) = value.get_mut(field) else {
        return;
    };
    if let Some(bytes) = json_bytes(value) {
        *value = Value::String(hex_bytes(&bytes));
    }
}

fn normalize_named_binary_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if matches!(
                    name.as_str(),
                    "rebind_payload" | "structured_state" | "Bytes"
                ) && let Some(bytes) = json_bytes(value)
                {
                    *value = binary_summary(&bytes);
                } else {
                    normalize_named_binary_fields(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_named_binary_fields(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn normalize_native_states(state: &mut Value) {
    let Some(Value::Array(states)) = state.get_mut("native_states") else {
        return;
    };
    for state in states {
        let Value::Array(pair) = state else {
            continue;
        };
        let Some(value) = pair.get_mut(1) else {
            continue;
        };
        if let Some(bytes) = json_bytes(value) {
            *value = binary_summary(&bytes);
        }
    }
}

fn json_bytes(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|value| value.as_u64().and_then(|value| u8::try_from(value).ok()))
        .collect()
}

fn binary_summary(bytes: &[u8]) -> Value {
    serde_json::json!({
        "byte_length": u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        "blake3": blake3::hash(bytes).to_hex().to_string(),
    })
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn encode_snapshot_payload<T: Serialize + ?Sized>(snapshot: &T) -> Result<Vec<u8>, VmError> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), SNAPSHOT_COMPRESSION_LEVEL)
        .map_err(|error| VmError::Snapshot(error.to_string()))?;
    let mut writer = CountingWriter::new(encoder);
    rmp_serde::encode::write(&mut writer, snapshot)
        .map_err(|error| VmError::Snapshot(error.to_string()))?;
    let uncompressed_len = writer.bytes;
    let payload = writer
        .into_inner()
        .finish()
        .map_err(|error| VmError::Snapshot(error.to_string()))?;
    let mut bytes = Vec::with_capacity(SNAPSHOT_HEADER_BYTES + payload.len());
    bytes.extend_from_slice(&SNAPSHOT_MAGIC);
    bytes.extend_from_slice(&SNAPSHOT_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&uncompressed_len.to_le_bytes());
    bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W> CountingWriter<W> {
    const fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct CountingReader<R> {
    inner: R,
    bytes: u64,
}

impl<R> CountingReader<R> {
    const fn new(inner: R) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        Ok(read)
    }
}

impl Vm {
    #[must_use]
    pub fn snapshot_eligibility(&self, natives: &NativeServiceRegistry) -> SnapshotEligibility {
        let mut blockers = Vec::new();
        if self.pending_reload.is_some() {
            blockers.push(SnapshotBlocker::PendingHotReload);
        }
        if !has_snapshot_stable_root(self.primary_fiber, &self.fibers) {
            blockers.push(SnapshotBlocker::PrimaryFiberNotSnapshotStable);
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

    /// Capture the VM at a stable input wait or after all fibers have terminated.
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
            compatibility: self.artifact().manifest.compatibility.clone(),
            current_generation: self.current_generation,
            memory: self.memory.clone(),
            fibers: self.fibers.clone(),
            primary_fiber: self.primary_fiber,
            next_fiber: self.next_fiber,
            next_frame: self.next_frame,
            next_request: self.next_request,
            next_generation: self.next_generation,
            arithmetic_warning_sites: self.arithmetic_warning_sites.clone(),
            native_states: natives
                .snapshots()
                .map_err(VmError::Snapshot)?
                .into_iter()
                .collect(),
        })
    }

    /// Encode the current stable state without cloning dense VM memory.
    ///
    /// # Errors
    ///
    /// Returns an error when the VM is not snapshot-eligible, a native service
    /// cannot be captured, or the payload cannot be serialized.
    pub fn encode_snapshot(&self, natives: &NativeServiceRegistry) -> Result<Vec<u8>, VmError> {
        if let SnapshotEligibility::Ineligible(blockers) = self.snapshot_eligibility(natives) {
            return Err(VmError::Snapshot(format!(
                "VM is not at a stable snapshot point: {blockers:?}"
            )));
        }
        self.encode_unrestricted_snapshot(natives)
    }

    /// Encode the current VM state for debugging or diagnosis, even when it cannot be restored.
    ///
    /// This deliberately bypasses capture-time stability checks so a faulting or runnable VM can
    /// be inspected. [`Vm::restore_snapshot`] still applies all artifact and state validation.
    ///
    /// # Errors
    ///
    /// Returns an error when native state cannot be captured or serialization fails.
    pub fn encode_unrestricted_snapshot(
        &self,
        natives: &NativeServiceRegistry,
    ) -> Result<Vec<u8>, VmError> {
        let native_states = natives
            .snapshots()
            .map_err(VmError::Snapshot)?
            .into_iter()
            .collect::<Vec<_>>();
        encode_snapshot_payload(&VmSnapshotRef {
            format_version: SNAPSHOT_FORMAT_VERSION,
            program_version: self.artifact().manifest.program_version,
            artifact_id: self.artifact_id(),
            compatibility: self.artifact().manifest.compatibility.clone(),
            current_generation: self.current_generation,
            memory: &self.memory,
            fibers: &self.fibers,
            primary_fiber: self.primary_fiber,
            next_fiber: self.next_fiber,
            next_frame: self.next_frame,
            next_request: self.next_request,
            next_generation: self.next_generation,
            native_states: &native_states,
            arithmetic_warning_sites: &self.arithmetic_warning_sites,
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
        mut snapshot: VmSnapshot,
        host: &mut impl VmHost,
        natives: &mut NativeServiceRegistry,
    ) -> Result<Self, VmError> {
        let expected = artifact.artifact();
        if snapshot.compatibility != expected.manifest.compatibility {
            return Err(VmError::Snapshot(format!(
                "snapshot compatibility differs: expected {:?}, received {:?}",
                expected.manifest.compatibility, snapshot.compatibility
            )));
        }
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION
            || snapshot.artifact_id != expected.manifest.artifact_id
            || snapshot.program_version != expected.manifest.program_version
        {
            return Err(VmError::Snapshot(
                "snapshot does not match the exact bytecode artifact".into(),
            ));
        }
        validate_snapshot(&snapshot, expected, config)?;
        snapshot
            .memory
            .materialize_snapshot()
            .map_err(VmError::Snapshot)?;
        for fiber in snapshot.fibers.values_mut() {
            for frame in &mut fiber.frames {
                for cell in frame.locals.values_mut() {
                    cell.materialize_snapshot().map_err(VmError::Snapshot)?;
                }
            }
        }
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
        let native_states = snapshot.native_states.iter().cloned().collect();

        let artifact = artifact.into_shared();
        let generation = snapshot.current_generation;
        let mut vm = Self {
            config,
            generations: BTreeMap::from([(generation, Arc::new(ProgramGeneration::new(artifact)))]),
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
            arithmetic_warning_sites: snapshot.arithmetic_warning_sites,
            pending_arithmetic_warnings: Vec::new(),
            debug: crate::debug::DebugState::default(),
            regex_cache: crate::regex_compat::RegexCache::default(),
            find_element_cache: HashMap::new(),
            find_element_cache_retained_bytes: 0,
            function_memo_cache: HashMap::new(),
            function_memo_cache_retained_bytes: 0,
            active_function_memos: HashMap::new(),
            path_memo_cache: HashMap::new(),
            path_memo_key_count: 0,
            path_memo_retained_bytes: 0,
            active_path_memo_fiber: std::cell::Cell::new(None),
            active_path_memo: std::cell::RefCell::new(None),
            #[cfg(test)]
            path_memo_replays: 0,
        };
        for fiber in vm.fibers.values() {
            for frame in &fiber.frames {
                if !vm.valid_frame_references(fiber, frame) || !vm.valid_frame_methods(fiber, frame)
                {
                    return Err(VmError::Snapshot(
                        "snapshot method resolution state is invalid".into(),
                    ));
                }
                if let Some(continuation) = &frame.runtime_form
                    && !continuation.valid_method_state(&vm, fiber)
                {
                    return Err(VmError::Snapshot(
                        "snapshot STRFORM method state is invalid".into(),
                    ));
                }
            }
        }
        let previous_native = natives.snapshots().map_err(VmError::Snapshot)?;
        natives
            .restore_snapshots(&native_states)
            .map_err(VmError::Snapshot)?;
        if let Err(error) = host.rebind_snapshot(&rebinds) {
            let _ = natives.restore_snapshots(&previous_native);
            return Err(VmError::Snapshot(error));
        }
        vm.retire_terminal_fibers();
        Ok(vm)
    }
}

fn validate_arithmetic_warning_sites(
    snapshot: &VmSnapshot,
    artifact: &erabasic_bytecode::BytecodeArtifact,
) -> Result<(), VmError> {
    if snapshot.arithmetic_warning_sites.is_empty() {
        return Ok(());
    }
    let code_lengths: HashMap<_, _> = artifact
        .functions
        .iter()
        .map(|function| (function.key, function.code.len()))
        .collect();
    for (generation, function, instruction, warning) in &snapshot.arithmetic_warning_sites {
        if *generation != snapshot.current_generation
            || *warning > 1
            || code_lengths
                .get(function)
                .is_none_or(|length| instruction >= length)
        {
            return Err(VmError::Snapshot(
                "snapshot arithmetic diagnostic identity is invalid".into(),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_snapshot(
    snapshot: &VmSnapshot,
    artifact: &erabasic_bytecode::BytecodeArtifact,
    config: VmConfig,
) -> Result<(), VmError> {
    validate_arithmetic_warning_sites(snapshot, artifact)?;
    if !snapshot.memory.legacy.is_empty() {
        return Err(VmError::Snapshot(
            "stable snapshots cannot contain legacy-generation storage".into(),
        ));
    }
    if !has_snapshot_stable_root(snapshot.primary_fiber, &snapshot.fibers) {
        return Err(VmError::Snapshot(
            "snapshot does not have a stable or quiescent primary fiber".into(),
        ));
    }
    let live_fibers = snapshot
        .fibers
        .values()
        .filter(|fiber| {
            !matches!(
                fiber.state,
                FiberState::Completed(_) | FiberState::Cancelled | FiberState::Faulted(_)
            )
        })
        .count();
    if live_fibers > config.maximum_fibers {
        return Err(VmError::Snapshot(
            "snapshot exceeds the configured live fiber limit".into(),
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
            if !frame_ids.insert(frame.id)
                || frame
                    .operand_slots()
                    .is_none_or(|slots| slots > config.maximum_operand_stack)
            {
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
            if let Some(continuation) = &frame.runtime_form {
                let (_, _, origin_instruction) = continuation.origin();
                let valid_call = frame.instruction == origin_instruction.saturating_add(1)
                    && function
                        .code
                        .get(origin_instruction)
                        .is_some_and(|instruction| {
                            if erabasic_bytecode::Opcode::try_from(instruction.opcode)
                                != Ok(erabasic_bytecode::Opcode::CallNative)
                            {
                                return false;
                            }
                            let Some(encoded_index) = instruction.payload.get(..4) else {
                                return false;
                            };
                            let mut bytes = [0; 4];
                            bytes.copy_from_slice(encoded_index);
                            let Some(import) = function
                                .imports
                                .get(u32::from_le_bytes(bytes) as usize)
                                .filter(|import| {
                                    import.kind == erabasic_bytecode::ImportKind::Native
                                })
                            else {
                                return false;
                            };
                            artifact.native_imports.iter().any(|native| {
                                native.import.key == import.key
                                    && native.import.name.eq_ignore_ascii_case("STRFORM")
                                    && matches!(
                                        native.import.parameters.as_slice(),
                                        [erabasic_bytecode::BytecodeType::String]
                                    )
                                    && native.import.result
                                        == Some(erabasic_bytecode::BytecodeType::String)
                            })
                        });
                if !valid_call
                    || !continuation.valid_for_frame(
                        frame.generation,
                        frame.function,
                        frame.id,
                        config.maximum_operand_stack,
                    )
                {
                    return Err(VmError::Snapshot(
                        "snapshot STRFORM continuation is invalid".into(),
                    ));
                }
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
                if let Some(parameter) = function
                    .parameters
                    .iter()
                    .find(|parameter| parameter.key == definition.key && parameter.by_reference)
                {
                    if cell.value_type != parameter.value_type
                        || !matches!(
                            cell.value_type,
                            erabasic_bytecode::BytecodeType::IntegerPlace
                                | erabasic_bytecode::BytecodeType::StringPlace
                        )
                        || cell.dimensions != [1]
                        || cell.len() != 1
                        || !cell.storage_is_valid()
                    {
                        return Err(VmError::Snapshot(format!(
                            "snapshot REF local {} has invalid alias storage",
                            definition.name
                        )));
                    }
                } else {
                    validate_cell(cell, definition)?;
                }
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
        if cell.is_none()
            && !matches!(
                definition.storage,
                erabasic_bytecode::BytecodeStorage::FunctionStatic
                    | erabasic_bytecode::BytecodeStorage::FunctionPersistent
            )
        {
            return Err(VmError::Snapshot(format!(
                "snapshot variable {} differs from the artifact layout",
                definition.name
            )));
        }
        if let Some(cell) = cell {
            validate_cell(cell, definition)?;
        }
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
    let maximum_frame = frame_ids.iter().map(|id| id.0).max().unwrap_or(0);
    let maximum_request = request_ids.iter().map(|id| id.0).max().unwrap_or(0);
    if snapshot.next_fiber == 0
        || snapshot.fibers.contains_key(&FiberId(snapshot.next_fiber))
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

fn has_snapshot_stable_root(primary: Option<FiberId>, fibers: &BTreeMap<FiberId, Fiber>) -> bool {
    if fibers.is_empty() {
        return primary.is_none();
    }
    let Some(primary) = primary.and_then(|id| fibers.get(&id)) else {
        return false;
    };
    if matches!(
        primary.state,
        FiberState::WaitingHost(crate::WaitingHost {
            stability: HostWaitStability::StableInput,
            ..
        })
    ) {
        return true;
    }
    // Runtime-owned system menus wait outside the VM after their dispatch root
    // returns. At that point the runtime snapshot carries the resumable input
    // operation and controller state, while the VM is safely quiescent.
    fibers.values().all(|fiber| {
        matches!(
            fiber.state,
            FiberState::Completed(_) | FiberState::Cancelled
        )
    })
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
        || cell.len() != expected_length
        || !cell.storage_is_valid()
    {
        return Err(VmError::Snapshot(format!(
            "snapshot variable {} has invalid storage",
            definition.name
        )));
    }
    Ok(())
}
