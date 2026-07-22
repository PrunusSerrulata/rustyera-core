use std::io::{Read, Write};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{ExtensionDeclaration, FilePayload, ProjectIdentity, ProjectManifest};
use erabasic_bytecode::BytecodeArtifact;
use erabasic_compiler::IncrementalState;
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::project::NormalizedProjectSnapshot;

const MAGIC: &[u8; 8] = b"RERACACH";
const VERSION: u32 = 2;
const SECTION_COUNT: u32 = 1;
const COMPRESSION_LEVEL: i32 = 7;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Serialize)]
struct CompiledCachePayloadRef<'a> {
    artifact: &'a BytecodeArtifact,
    incremental: &'a IncrementalState,
    snapshot: &'a NormalizedProjectSnapshot,
}

#[derive(Deserialize)]
struct CompiledCachePayload {
    artifact: BytecodeArtifact,
    incremental: IncrementalState,
    snapshot: NormalizedProjectSnapshot,
}

pub(crate) struct DecodedCompiledCache {
    pub(crate) key: [u8; 32],
    pub(crate) artifact: ValidatedArtifact,
    pub(crate) incremental: IncrementalState,
    pub(crate) snapshot: NormalizedProjectSnapshot,
}

pub(crate) fn project_key(
    identity: &ProjectIdentity,
    extensions: &[ExtensionDeclaration],
) -> [u8; 32] {
    let mut writer = HashWriter::new("rustyera.compiled-project-key.v2");
    serde_json::to_writer(
        &mut writer,
        &(identity.source_digest.as_slice(), extensions),
    )
    .expect("project cache identity values are serializable");
    writer.finish()
}

pub(crate) fn project_identity(manifest: &ProjectManifest) -> ProjectIdentity {
    let mut files = manifest.files.iter().collect::<Vec<_>>();
    files.sort_by_key(|file| {
        (
            file.relative_path.to_lowercase(),
            file.relative_path.clone(),
        )
    });
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.project-source-identity.v1");
    for file in files {
        let path = file.relative_path.as_bytes();
        hasher.update(&(path.len() as u64).to_le_bytes());
        hasher.update(path);
        hasher.update(&[file.category as u8]);
        let digest = file.content_hash.as_ref().map_or_else(
            || match &file.payload {
                FilePayload::Utf8(text) => *blake3::hash(text.as_bytes()).as_bytes(),
                FilePayload::Bytes(bytes) => *blake3::hash(bytes.as_slice()).as_bytes(),
                FilePayload::IoError(error) => *blake3::hash(error.message.as_bytes()).as_bytes(),
            },
            |value| {
                value
                    .as_slice()
                    .try_into()
                    .unwrap_or_else(|_| *blake3::hash(value.as_slice()).as_bytes())
            },
        );
        hasher.update(&digest);
    }
    ProjectIdentity {
        project_revision: manifest.project_revision,
        source_digest: ProtocolBytes::new(hasher.finalize().as_bytes().to_vec()),
    }
}

pub(crate) fn encode(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&project_key(&project_identity(manifest), extensions));
    output.extend_from_slice(&SECTION_COUNT.to_le_bytes());
    append_section(
        &mut output,
        &CompiledCachePayloadRef {
            artifact: artifact.artifact(),
            incremental,
            snapshot,
        },
    )?;
    let digest = blake3::hash(&output);
    output.extend_from_slice(digest.as_bytes());
    Ok(output)
}

pub(crate) fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<DecodedCompiledCache, String> {
    if bytes.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    let minimum = MAGIC.len() + 4 + 32 + 4 + 32;
    if bytes.len() < minimum || &bytes[..MAGIC.len()] != MAGIC {
        return Err("compiled project cache has an invalid header".into());
    }
    let digest_offset = bytes.len() - 32;
    if blake3::hash(&bytes[..digest_offset]).as_bytes() != &bytes[digest_offset..] {
        return Err("compiled project cache digest mismatch".into());
    }
    let mut cursor = MAGIC.len();
    let version = read_u32(bytes, &mut cursor)?;
    if version != VERSION {
        return Err(format!(
            "unsupported compiled project cache version {version}"
        ));
    }
    let key: [u8; 32] = bytes
        .get(cursor..cursor + 32)
        .ok_or("compiled project cache key is truncated")?
        .try_into()
        .expect("32-byte slice");
    cursor += 32;
    if read_u32(bytes, &mut cursor)? != SECTION_COUNT {
        return Err("compiled project cache section count differs".into());
    }
    let payload: CompiledCachePayload = decode_section(
        bytes,
        &mut cursor,
        digest_offset,
        MAXIMUM_DECODED_PAYLOAD_BYTES,
    )?;
    if cursor != digest_offset {
        return Err("compiled project cache has trailing data".into());
    }
    let unvalidated = payload.artifact.into_unvalidated();
    let context = ValidationContext::for_artifact(unvalidated.artifact());
    let validation = validate_bytecode(unvalidated, &context);
    let artifact = validation.value.ok_or_else(|| {
        validation.diagnostics.first().map_or_else(
            || "cached artifact failed validation".into(),
            |value| value.message.clone(),
        )
    })?;
    Ok(DecodedCompiledCache {
        key,
        artifact,
        incremental: payload.incremental,
        snapshot: payload.snapshot,
    })
}

fn append_section<T: Serialize>(output: &mut Vec<u8>, value: &T) -> Result<(), String> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)
        .map_err(|error| error.to_string())?;
    let mut writer = CountingWriter::new(encoder);
    serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
    let decoded_length = writer.bytes;
    let compressed = writer
        .into_inner()
        .finish()
        .map_err(|error| error.to_string())?;
    output.extend_from_slice(&decoded_length.to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(compressed.len())
            .map_err(|_| "compiled cache section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(blake3::hash(&compressed).as_bytes());
    output.extend_from_slice(&compressed);
    Ok(())
}

fn decode_section<T: DeserializeOwned>(
    bytes: &[u8],
    cursor: &mut usize,
    digest_offset: usize,
    maximum_decoded_bytes: u64,
) -> Result<T, String> {
    let decoded_length = read_u64(bytes, cursor)?;
    if decoded_length > maximum_decoded_bytes {
        return Err("compiled cache decoded section exceeds its limit".into());
    }
    let compressed_length = usize::try_from(read_u64(bytes, cursor)?)
        .map_err(|_| "compiled cache section is not addressable")?;
    let expected_digest = bytes
        .get(*cursor..cursor.saturating_add(32))
        .ok_or("compiled cache section digest is truncated")?;
    *cursor += 32;
    let end = cursor
        .checked_add(compressed_length)
        .ok_or("compiled cache section length overflow")?;
    if end > digest_offset {
        return Err("compiled cache section is truncated".into());
    }
    let compressed = &bytes[*cursor..end];
    *cursor = end;
    if blake3::hash(compressed).as_bytes() != expected_digest {
        return Err("compiled cache section digest mismatch".into());
    }
    let decoder =
        zstd::stream::read::Decoder::new(compressed).map_err(|error| error.to_string())?;
    let mut reader = CountingReader::new(decoder.take(decoded_length.saturating_add(1)));
    let value = serde_json::from_reader(&mut reader).map_err(|error| error.to_string())?;
    let mut tail = [0_u8; 1];
    if reader.read(&mut tail).map_err(|error| error.to_string())? != 0
        || reader.bytes != decoded_length
    {
        return Err("compiled cache decoded section length differs".into());
    }
    Ok(value)
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let end = cursor.saturating_add(4);
    let value = bytes
        .get(*cursor..end)
        .ok_or("compiled project cache is truncated")?;
    *cursor = end;
    Ok(u32::from_le_bytes(
        value.try_into().expect("four-byte slice"),
    ))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let end = cursor.saturating_add(8);
    let value = bytes
        .get(*cursor..end)
        .ok_or("compiled project cache is truncated")?;
    *cursor = end;
    Ok(u64::from_le_bytes(
        value.try_into().expect("eight-byte slice"),
    ))
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
        self.bytes = self.bytes.saturating_add(written as u64);
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
        self.bytes = self.bytes.saturating_add(read as u64);
        Ok(read)
    }
}

struct HashWriter {
    hasher: blake3::Hasher,
}

impl HashWriter {
    fn new(domain: &str) -> Self {
        Self {
            hasher: blake3::Hasher::new_derive_key(domain),
        }
    }

    fn finish(self) -> [u8; 32] {
        *self.hasher.finalize().as_bytes()
    }
}

impl Write for HashWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};

    use super::*;

    fn manifest(source: &str, revision: u64) -> ProjectManifest {
        ProjectManifest {
            project_revision: revision,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
                content_hash: None,
            }],
        }
    }

    #[test]
    fn compiled_project_cache_round_trips_and_keys_source_content() {
        let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
        let mut build = crate::project::build_project(&project, None);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        build.incremental.compact();
        let bytes = encode(
            &project,
            &[],
            build.artifact.as_ref().unwrap(),
            &build.incremental,
            build.snapshot.as_ref().unwrap(),
        )
        .unwrap();
        let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();

        assert_eq!(decoded.key, project_key(&project_identity(&project), &[]));
        assert_eq!(
            decoded.artifact.artifact().manifest.artifact_id,
            build.artifact.unwrap().artifact().manifest.artifact_id
        );
        assert_eq!(
            project_key(&project_identity(&project), &[]),
            project_key(
                &project_identity(&manifest("@SYSTEM_TITLE\nRETURN\n", 9)),
                &[]
            )
        );
        assert_ne!(
            project_key(&project_identity(&project), &[]),
            project_key(
                &project_identity(&manifest("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n", 1)),
                &[]
            )
        );
    }

    #[test]
    fn compiled_project_cache_rejects_corruption() {
        assert!(decode(b"not a compiled cache", 1024).is_err());
    }
}
