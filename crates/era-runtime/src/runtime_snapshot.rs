use std::collections::BTreeMap;

use era_runtime_protocol::InteractionToken;
use erabasic_bytecode::Digest;
use erabasic_vm::VmValue;
use serde::{Deserialize, Serialize};

use crate::controller::SystemController;
use crate::operation::PendingOperations;
use crate::presentation::PresentationModel;

pub(crate) const RUNTIME_SNAPSHOT_FORMAT_VERSION: u32 = 2;
const MAGIC: [u8; 8] = *b"RERARTS\0";
const HEADER_BYTES: usize = 52;

#[derive(Serialize, Deserialize)]
pub(crate) struct RuntimeSnapshotPayload {
    pub(crate) format_version: u32,
    pub(crate) artifact_id: Digest,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resource_count: u64,
    pub(crate) epoch: u64,
    pub(crate) vm_snapshot: Vec<u8>,
    pub(crate) presentation: PresentationModel,
    pub(crate) operations: PendingOperations,
    pub(crate) controller: SystemController,
    pub(crate) logical_time_ns: u64,
    pub(crate) random_seed: Option<u64>,
    pub(crate) message_skip: bool,
    pub(crate) command_intents: BTreeMap<InteractionToken, VmValue>,
    pub(crate) reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    pub(crate) save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    pub(crate) system_menu: u8,
    pub(crate) load_slot_paths: Vec<String>,
}

pub(crate) fn encode(payload: &RuntimeSnapshotPayload) -> Result<Vec<u8>, String> {
    let payload = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(HEADER_BYTES + payload.len());
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&RUNTIME_SNAPSHOT_FORMAT_VERSION.to_le_bytes());
    output.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    output.extend_from_slice(blake3::hash(&payload).as_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

pub(crate) fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<RuntimeSnapshotPayload, String> {
    if bytes.len() > maximum_bytes {
        return Err("runtime snapshot exceeds the configured limit".into());
    }
    if bytes.len() < HEADER_BYTES || bytes[..8] != MAGIC {
        return Err("invalid runtime snapshot header".into());
    }
    let version = u32::from_le_bytes(
        bytes[8..12]
            .try_into()
            .map_err(|_| "truncated runtime snapshot version")?,
    );
    if version != RUNTIME_SNAPSHOT_FORMAT_VERSION {
        return Err(format!("unsupported runtime snapshot format {version}"));
    }
    let length = u64::from_le_bytes(
        bytes[12..20]
            .try_into()
            .map_err(|_| "truncated runtime snapshot length")?,
    );
    let length =
        usize::try_from(length).map_err(|_| "runtime snapshot length exceeds this platform")?;
    if bytes.len() != HEADER_BYTES.saturating_add(length) {
        return Err("runtime snapshot length is inconsistent".into());
    }
    let payload = &bytes[HEADER_BYTES..];
    if blake3::hash(payload).as_bytes() != &bytes[20..HEADER_BYTES] {
        return Err("runtime snapshot checksum differs".into());
    }
    let decoded: RuntimeSnapshotPayload =
        serde_json::from_slice(payload).map_err(|error| error.to_string())?;
    if decoded.format_version != RUNTIME_SNAPSHOT_FORMAT_VERSION {
        return Err("runtime snapshot payload version differs from its container".into());
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_rejects_mutated_payload() {
        let payload = RuntimeSnapshotPayload {
            format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
            artifact_id: Digest([1; 32]),
            project_identity: [2; 32],
            resource_count: 0,
            epoch: 3,
            vm_snapshot: vec![3],
            presentation: PresentationModel::default(),
            operations: PendingOperations::default(),
            controller: SystemController::default(),
            logical_time_ns: 4,
            random_seed: Some(5),
            message_skip: false,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            save_extensions: Vec::new(),
            system_menu: 0,
            load_slot_paths: Vec::new(),
        };
        let mut encoded = encode(&payload).unwrap();
        let last = encoded.last_mut().unwrap();
        *last ^= 1;
        assert!(decode(&encoded, encoded.len()).is_err());
    }
}
