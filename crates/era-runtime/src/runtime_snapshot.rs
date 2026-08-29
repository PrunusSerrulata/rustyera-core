use std::collections::BTreeMap;
use std::io::{Read, Write};

use era_runtime_protocol::InteractionToken;
use erabasic_bytecode::Digest;
use erabasic_vm::VmValue;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::controller::SystemController;
use crate::operation::PendingOperations;
use crate::presentation::PresentationModel;
use crate::resource::ResourceGraph;

pub(crate) const RUNTIME_SNAPSHOT_FORMAT_VERSION: u32 = 26;
#[cfg(test)]
const LEGACY_RUNTIME_SNAPSHOT_FORMAT_VERSION: u32 = 20;
pub(crate) const CULTURE_TABLE_VERSION: u32 = 1;
const MAGIC: [u8; 8] = *b"RERARTS\0";
const HEADER_BYTES: usize = 60;
const COMPRESSION_LEVEL: i32 = 1;

/// Version of the developer-facing JSON-compatible inspection projection.
pub const RUNTIME_SNAPSHOT_INSPECTION_SCHEMA_VERSION: u32 = 1;

/// All information available from a structurally valid runtime snapshot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeSnapshotInspection {
    pub inspection_schema_version: u32,
    pub container: RuntimeSnapshotContainerInspection,
    pub payload: Value,
    pub validation: RuntimeSnapshotValidation,
}

/// Header information from a validated runtime snapshot container.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeSnapshotContainerInspection {
    pub magic: String,
    pub format_version: u32,
    pub file_bytes: u64,
    pub compressed_payload_bytes: u64,
    pub uncompressed_payload_bytes: u64,
    pub payload_blake3: String,
}

/// Checks completed, and checks that require data absent from the snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeSnapshotValidation {
    pub runtime_container: String,
    pub embedded_container: String,
    pub artifact_compatibility: String,
    pub restore_semantics: String,
}

/// Failure to decode or project a runtime snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSnapshotInspectionError {
    message: String,
}

impl std::fmt::Display for RuntimeSnapshotInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RuntimeSnapshotInspectionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum RuntimeSnapshotOrigin {
    Normal,
    Debug,
    Diagnosis,
}

#[derive(Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RuntimeSnapshotPayload {
    pub(crate) format_version: u32,
    pub(crate) origin: RuntimeSnapshotOrigin,
    pub(crate) artifact_id: Digest,
    pub(crate) compatibility: erabasic_compat::CompatibilityIdentity,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resource_count: u64,
    pub(crate) resource_graph: ResourceGraph,
    pub(crate) epoch: u64,
    #[serde(with = "snapshot_bytes")]
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
    pub(crate) hotkey_state: Vec<i64>,
    pub(crate) key_macros: crate::key_macro::KeyMacros,
    pub(crate) input_controller: crate::input_source::InputController,
    pub(crate) text_box: String,
    pub(crate) text_box_layout: era_runtime_protocol::TextBoxLayout,
    pub(crate) flow_input_enabled: bool,
    pub(crate) flow_input_default: i64,
    pub(crate) flow_input_can_skip: bool,
    pub(crate) flow_input_force_skip: bool,
    pub(crate) flow_input_string: bool,
    pub(crate) flow_input_default_string: String,
    pub(crate) button_generation: u64,
    pub(crate) debug_output: String,
    pub(crate) debug_output_base: u64,
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
    pub(crate) undo_checkpoint: Option<super::session::UndoCheckpoint>,
    pub(crate) undo_replay: Option<super::session::UndoReplay>,
}

mod snapshot_bytes {
    use std::fmt;

    use serde::{Deserializer, Serializer, de::Visitor};

    pub(super) fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(bytes)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a VM snapshot byte string")
            }

            fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E> {
                Ok(bytes.to_vec())
            }

            fn visit_borrowed_bytes<E>(self, bytes: &'de [u8]) -> Result<Self::Value, E> {
                Ok(bytes.to_vec())
            }

            fn visit_byte_buf<E>(self, bytes: Vec<u8>) -> Result<Self::Value, E> {
                Ok(bytes)
            }
        }

        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

/// Ordered key/value pairs keep interaction-token maps deterministic across
/// serializers and let restore reject duplicate keys explicitly.
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
    if payload.format_version != RUNTIME_SNAPSHOT_FORMAT_VERSION {
        return Err("runtime snapshot payload does not use the current format".into());
    }
    encode_container(payload, RUNTIME_SNAPSHOT_FORMAT_VERSION)
}

fn encode_container(
    payload: &RuntimeSnapshotPayload,
    format_version: u32,
) -> Result<Vec<u8>, String> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)
        .map_err(|error| error.to_string())?;
    let mut writer = CountingWriter::new(encoder);
    rmp_serde::encode::write(&mut writer, payload).map_err(|error| error.to_string())?;
    let uncompressed_len = writer.bytes;
    let payload = writer
        .into_inner()
        .finish()
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(HEADER_BYTES + payload.len());
    output.extend_from_slice(&MAGIC);
    output.extend_from_slice(&format_version.to_le_bytes());
    output.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    output.extend_from_slice(&uncompressed_len.to_le_bytes());
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
    let uncompressed_length = u64::from_le_bytes(
        bytes[20..28]
            .try_into()
            .map_err(|_| "truncated runtime snapshot raw length")?,
    );
    let uncompressed_length = usize::try_from(uncompressed_length)
        .map_err(|_| "runtime snapshot raw length exceeds this platform")?;
    if uncompressed_length > maximum_bytes {
        return Err("runtime snapshot expands beyond the configured limit".into());
    }
    if bytes.len() != HEADER_BYTES.saturating_add(length) {
        return Err("runtime snapshot length is inconsistent".into());
    }
    let payload = &bytes[HEADER_BYTES..];
    if blake3::hash(payload).as_bytes() != &bytes[28..HEADER_BYTES] {
        return Err("runtime snapshot checksum differs".into());
    }
    // Limit expansion independently from the untrusted length header. Reading
    // one byte past the declaration lets the length check reject extra output.
    let decompression_limit = (uncompressed_length as u64).saturating_add(1);
    let decoder = zstd::stream::read::Decoder::new(payload).map_err(|error| error.to_string())?;
    let mut reader = CountingReader::new(decoder.take(decompression_limit));
    let snapshot: RuntimeSnapshotPayload =
        rmp_serde::from_read(&mut reader).map_err(|error| error.to_string())?;
    let mut tail = [0_u8; 1];
    if reader.read(&mut tail).map_err(|error| error.to_string())? != 0
        || reader.bytes != uncompressed_length as u64
    {
        return Err("runtime snapshot raw length is inconsistent".into());
    }
    if snapshot.format_version != version {
        return Err("runtime snapshot payload version differs from its container".into());
    }
    Ok(snapshot)
}

/// Validate and project a complete runtime snapshot without restoring it.
///
/// Opaque binary fields are replaced with their length and BLAKE3 digest. The
/// result includes the decoded embedded execution snapshot, but cannot perform
/// compatibility or restore checks that require the original bytecode artifact.
///
/// # Errors
///
/// Returns an error for an invalid runtime or embedded snapshot, configured
/// size-limit violations, or a state that cannot be projected as JSON.
pub fn inspect_runtime_snapshot(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<RuntimeSnapshotInspection, RuntimeSnapshotInspectionError> {
    let snapshot = decode(bytes, maximum_bytes).map_err(inspection_error)?;
    snapshot
        .compatibility
        .validate()
        .map_err(|error| inspection_error(error.to_string()))?;
    let format_version = snapshot.format_version;
    let execution = erabasic_vm::inspect_snapshot(&snapshot.vm_snapshot, maximum_bytes)
        .map_err(|error| inspection_error(format!("invalid embedded snapshot: {error}")))?;
    let expected = serde_json::to_value(&snapshot.compatibility)
        .map_err(|error| inspection_error(error.to_string()))?;
    if execution.state.get("compatibility") != Some(&expected) {
        return Err(inspection_error(
            "runtime and embedded snapshot compatibility differ",
        ));
    }
    let mut payload = serde_json::to_value(&snapshot)
        .map_err(|error| inspection_error(format!("cannot inspect runtime state: {error}")))?;
    let fields = payload
        .as_object_mut()
        .ok_or_else(|| inspection_error("runtime snapshot state is not an object"))?;
    fields.remove("vm_snapshot");
    let execution = serde_json::to_value(execution)
        .map_err(|error| inspection_error(format!("cannot inspect execution state: {error}")))?;
    fields.insert("execution_state".into(), execution);
    fields.insert(
        "system_menu_name".into(),
        Value::String(system_menu_name(snapshot.system_menu).into()),
    );
    normalize_runtime_values(&mut payload);
    Ok(RuntimeSnapshotInspection {
        inspection_schema_version: RUNTIME_SNAPSHOT_INSPECTION_SCHEMA_VERSION,
        container: RuntimeSnapshotContainerInspection {
            magic: "RERARTS\\0".into(),
            format_version,
            file_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            compressed_payload_bytes: read_header_u64(bytes, 12),
            uncompressed_payload_bytes: read_header_u64(bytes, 20),
            payload_blake3: hex_bytes(&bytes[28..HEADER_BYTES]),
        },
        payload,
        validation: RuntimeSnapshotValidation {
            runtime_container: "valid".into(),
            embedded_container: "valid".into(),
            artifact_compatibility: "not_checked".into(),
            restore_semantics: "not_checked".into(),
        },
    })
}

fn inspection_error(message: impl Into<String>) -> RuntimeSnapshotInspectionError {
    RuntimeSnapshotInspectionError {
        message: message.into(),
    }
}

fn read_header_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("validated runtime snapshot header contains this field"),
    )
}

fn normalize_runtime_values(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if is_digest_field(name)
                    && let Some(bytes) = json_bytes(value)
                {
                    *value = Value::String(hex_bytes(&bytes));
                } else if is_binary_field(name)
                    && let Some(bytes) = json_bytes(value)
                {
                    *value = binary_summary(&bytes);
                } else {
                    normalize_runtime_values(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_runtime_values(value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn is_digest_field(name: &str) -> bool {
    matches!(
        name,
        "artifact_id" | "project_identity" | "digest" | "content_digest"
    )
}

fn is_binary_field(name: &str) -> bool {
    matches!(
        name,
        "bytes" | "encoded" | "data" | "payload" | "save_bytes" | "text_payload" | "Bytes"
    )
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

const fn system_menu_name(value: u8) -> &'static str {
    match value {
        0 => "title",
        1 => "load_slots",
        2 => "save_slots",
        3 => "confirm_overwrite",
        _ => "unknown",
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_rejects_mutated_payload() {
        let mut resource_graph = ResourceGraph::default();
        assert_eq!(resource_graph.create_canvas(7, 20, 10), Ok(true));
        let mut payload = RuntimeSnapshotPayload {
            format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
            origin: RuntimeSnapshotOrigin::Normal,
            artifact_id: Digest([1; 32]),
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            hotkey_state: Vec::new(),
            key_macros: crate::key_macro::KeyMacros::default(),
            input_controller: crate::input_source::InputController::default(),
            text_box: String::new(),
            text_box_layout: era_runtime_protocol::TextBoxLayout::default(),
            flow_input_enabled: false,
            flow_input_default: 0,
            flow_input_can_skip: false,
            flow_input_force_skip: false,
            flow_input_string: false,
            flow_input_default_string: String::new(),
            button_generation: 0,
            debug_output: String::new(),
            debug_output_base: 0,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            save_extensions: Vec::new(),
            system_menu: 0,
            system_menu_slot: None,
            load_slot_paths: Vec::new(),
            occupied_slot_paths: std::collections::BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
            undo_checkpoint: None,
            undo_replay: None,
        };
        let mut encoded = encode(&payload).unwrap();
        let last = encoded.last_mut().unwrap();
        *last ^= 1;
        assert!(decode(&encoded, encoded.len()).is_err());

        payload.format_version = LEGACY_RUNTIME_SNAPSHOT_FORMAT_VERSION;
        let legacy = encode_container(&payload, LEGACY_RUNTIME_SNAPSHOT_FORMAT_VERSION).unwrap();
        assert!(decode(&legacy, usize::MAX).is_err());
    }

    #[test]
    fn canvas_replay_state_round_trips_in_exact_runtime_snapshots() {
        let mut resource_graph = ResourceGraph::default();
        resource_graph.create_canvas(7, 20, 10).unwrap();
        assert!(resource_graph.set_animation_timer(7));
        let text_line_background = era_runtime_protocol::Color {
            red: 17,
            green: 34,
            blue: 51,
            alpha: 127,
        };
        let mut presentation = PresentationModel::default();
        presentation.set_text_line_background(Some(text_line_background));
        let payload = RuntimeSnapshotPayload {
            format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
            origin: RuntimeSnapshotOrigin::Debug,
            artifact_id: Digest([1; 32]),
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_identity: [2; 32],
            resource_count: 0,
            resource_graph,
            epoch: 3,
            vm_snapshot: vec![3],
            presentation,
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
            hotkey_state: Vec::new(),
            key_macros: crate::key_macro::KeyMacros::default(),
            input_controller: crate::input_source::InputController::default(),
            text_box: String::new(),
            text_box_layout: era_runtime_protocol::TextBoxLayout::default(),
            flow_input_enabled: false,
            flow_input_default: 0,
            flow_input_can_skip: false,
            flow_input_force_skip: false,
            flow_input_string: false,
            flow_input_default_string: String::new(),
            button_generation: 0,
            debug_output: String::new(),
            debug_output_base: 0,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            save_extensions: Vec::new(),
            system_menu: 3,
            system_menu_slot: Some(17),
            load_slot_paths: Vec::new(),
            occupied_slot_paths: std::collections::BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
            undo_checkpoint: None,
            undo_replay: None,
        };
        let uncompressed = rmp_serde::to_vec(&payload).unwrap();
        let encoded = encode(&payload).unwrap();
        assert!(encoded.len() < uncompressed.len());
        let mut understated = encoded.clone();
        understated[20..28].copy_from_slice(&((uncompressed.len() as u64) - 1).to_le_bytes());
        assert!(decode(&understated, uncompressed.len()).is_err());
        let decoded = decode(&encoded, uncompressed.len()).unwrap();
        assert_eq!(decoded.origin, RuntimeSnapshotOrigin::Debug);
        assert_eq!(decoded.resource_graph.canvas_state(7), Some((20, 10)));
        assert_eq!(decoded.resource_graph.animation_timer(), 10);
        assert_eq!(
            decoded
                .presentation
                .snapshot()
                .settings
                .text_line_background,
            Some(text_line_background)
        );
        assert_eq!(decoded.selected_locale, "ja");
        assert_eq!(decoded.culture_table_version, CULTURE_TABLE_VERSION);
        assert_eq!(decoded.force_kana_mode, 0);
        assert_eq!(decoded.system_menu, 3);
        assert_eq!(decoded.system_menu_slot, Some(17));
    }

    #[test]
    fn inspection_normalizes_binary_fields_without_touching_numeric_state() {
        let bytes = b"opaque";
        let mut value = serde_json::json!({
            "artifact_id": [1, 2, 3],
            "resource": {
                "bytes": bytes,
                "encoded": [9, 8, 7],
                "digest": [10, 11],
            },
            "save_extensions": [{"payload": [4, 5]}],
            "hotkey_state": [1, 2, 3],
        });
        normalize_runtime_values(&mut value);
        assert_eq!(value["artifact_id"], "010203");
        assert_eq!(value["resource"]["digest"], "0a0b");
        assert_eq!(value["resource"]["bytes"]["byte_length"], 6);
        assert_eq!(
            value["resource"]["bytes"]["blake3"],
            blake3::hash(bytes).to_hex().to_string()
        );
        assert_eq!(value["save_extensions"][0]["payload"]["byte_length"], 2);
        assert_eq!(value["hotkey_state"], serde_json::json!([1, 2, 3]));
    }
}
