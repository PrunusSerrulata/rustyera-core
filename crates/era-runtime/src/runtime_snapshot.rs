use std::collections::BTreeMap;

use era_runtime_protocol::InteractionToken;
use erabasic_bytecode::Digest;
use erabasic_vm::VmValue;
use serde::{Deserialize, Serialize};

use crate::controller::SystemController;
use crate::operation::PendingOperations;
use crate::presentation::PresentationModel;
use crate::resource::ResourceGraph;

pub(crate) const RUNTIME_SNAPSHOT_FORMAT_VERSION: u32 = 8;
pub(crate) const CULTURE_TABLE_VERSION: u32 = 1;
const MAGIC: [u8; 8] = *b"RERARTS\0";
const HEADER_BYTES: usize = 52;

#[derive(Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RuntimeSnapshotPayload {
    pub(crate) format_version: u32,
    pub(crate) artifact_id: Digest,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resource_count: u64,
    pub(crate) resource_graph: ResourceGraph,
    pub(crate) epoch: u64,
    pub(crate) vm_snapshot: Vec<u8>,
    pub(crate) presentation: PresentationModel,
    pub(crate) operations: PendingOperations,
    pub(crate) controller: SystemController,
    pub(crate) logical_time_ns: u64,
    pub(crate) random_seed: Option<u64>,
    pub(crate) selected_locale: String,
    pub(crate) culture_table_version: u32,
    pub(crate) message_skip: bool,
    pub(crate) skip_print: bool,
    pub(crate) user_defined_skip: bool,
    pub(crate) saved_skip: bool,
    pub(crate) force_kana_mode: u8,
    #[serde(with = "token_value_map")]
    pub(crate) command_intents: BTreeMap<InteractionToken, VmValue>,
    #[serde(with = "token_value_map")]
    pub(crate) reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    pub(crate) save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    pub(crate) system_menu: u8,
    pub(crate) system_menu_slot: Option<u32>,
    pub(crate) load_slot_paths: Vec<String>,
    pub(crate) occupied_slot_paths: std::collections::BTreeSet<String>,
    pub(crate) system_menu_host_request: Option<erabasic_vm::HostRequestId>,
    pub(crate) system_menu_page: u32,
}

/// JSON objects cannot represent structured interaction tokens as keys. The runtime snapshot
/// encodes those internal maps as ordered key/value pairs and rejects duplicate keys on restore.
pub(crate) mod token_value_map {
    use std::collections::BTreeMap;

    use era_runtime_protocol::InteractionToken;
    use erabasic_vm::VmValue;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    pub(crate) fn serialize<S>(
        values: &BTreeMap<InteractionToken, VmValue>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        values.iter().collect::<Vec<_>>().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<InteractionToken, VmValue>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pairs = Vec::<(InteractionToken, VmValue)>::deserialize(deserializer)?;
        let mut values = BTreeMap::new();
        for (token, value) in pairs {
            if values.insert(token, value).is_some() {
                return Err(D::Error::custom(
                    "runtime snapshot contains a duplicate interaction token",
                ));
            }
        }
        Ok(values)
    }
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
        let mut resource_graph = ResourceGraph::default();
        assert_eq!(resource_graph.create_canvas(7, 20, 10), Ok(true));
        let payload = RuntimeSnapshotPayload {
            format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
            artifact_id: Digest([1; 32]),
            project_identity: [2; 32],
            resource_count: 0,
            resource_graph,
            epoch: 3,
            vm_snapshot: vec![3],
            presentation: PresentationModel::default(),
            operations: PendingOperations::default(),
            controller: SystemController::default(),
            logical_time_ns: 4,
            random_seed: Some(5),
            selected_locale: "ja".into(),
            culture_table_version: CULTURE_TABLE_VERSION,
            message_skip: false,
            skip_print: false,
            user_defined_skip: false,
            saved_skip: false,
            force_kana_mode: 0,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            save_extensions: Vec::new(),
            system_menu: 0,
            system_menu_slot: None,
            load_slot_paths: Vec::new(),
            occupied_slot_paths: std::collections::BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
        };
        let mut encoded = encode(&payload).unwrap();
        let last = encoded.last_mut().unwrap();
        *last ^= 1;
        assert!(decode(&encoded, encoded.len()).is_err());
    }

    #[test]
    fn canvas_replay_state_round_trips_in_exact_runtime_snapshots() {
        let mut resource_graph = ResourceGraph::default();
        resource_graph.create_canvas(7, 20, 10).unwrap();
        let payload = RuntimeSnapshotPayload {
            format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
            artifact_id: Digest([1; 32]),
            project_identity: [2; 32],
            resource_count: 0,
            resource_graph,
            epoch: 3,
            vm_snapshot: vec![3],
            presentation: PresentationModel::default(),
            operations: PendingOperations::default(),
            controller: SystemController::default(),
            logical_time_ns: 4,
            random_seed: Some(5),
            selected_locale: "ja".into(),
            culture_table_version: CULTURE_TABLE_VERSION,
            message_skip: false,
            skip_print: false,
            user_defined_skip: false,
            saved_skip: false,
            force_kana_mode: 0,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            save_extensions: Vec::new(),
            system_menu: 3,
            system_menu_slot: Some(17),
            load_slot_paths: Vec::new(),
            occupied_slot_paths: std::collections::BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
        };
        let encoded = encode(&payload).unwrap();
        let decoded = decode(&encoded, encoded.len()).unwrap();
        assert_eq!(decoded.resource_graph.canvas_state(7), Some((20, 10)));
        assert_eq!(decoded.selected_locale, "ja");
        assert_eq!(decoded.culture_table_version, CULTURE_TABLE_VERSION);
        assert_eq!(decoded.force_kana_mode, 0);
        assert_eq!(decoded.system_menu, 3);
        assert_eq!(decoded.system_menu_slot, Some(17));
    }
}
