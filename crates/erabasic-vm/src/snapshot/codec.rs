#[allow(clippy::wildcard_imports)]
use super::*;
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

pub(super) fn encode_snapshot_payload<T: Serialize + ?Sized>(
    snapshot: &T,
) -> Result<Vec<u8>, VmError> {
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
