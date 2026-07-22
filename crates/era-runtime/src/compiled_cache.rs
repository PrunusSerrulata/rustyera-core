use std::io::{Read, Write};

use era_runtime_protocol::{ExtensionDeclaration, ProjectManifest};
use erabasic_bytecode::{DecodeLimits, decode_artifact, encode_artifact};
use erabasic_compiler::IncrementalState;
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::project::NormalizedProjectSnapshot;

const MAGIC: &[u8; 8] = b"RERACACH";
const VERSION: u32 = 1;

pub(crate) struct DecodedCompiledCache {
    pub(crate) key: [u8; 32],
    pub(crate) artifact: ValidatedArtifact,
    pub(crate) incremental: IncrementalState,
    pub(crate) snapshot: NormalizedProjectSnapshot,
}

pub(crate) fn project_key(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
) -> [u8; 32] {
    let mut normalized = manifest.clone();
    normalized.project_revision = 0;
    let bytes = serde_json::to_vec(&(normalized, extensions))
        .expect("project cache identity values are serializable");
    *blake3::hash(&bytes).as_bytes()
}

pub(crate) fn encode(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
) -> Result<Vec<u8>, String> {
    let artifact = encode_artifact(artifact.artifact()).map_err(|error| error.to_string())?;
    let incremental = serde_json::to_vec(incremental).map_err(|error| error.to_string())?;
    let snapshot = serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;
    let mut plain = Vec::new();
    plain.extend_from_slice(MAGIC);
    plain.extend_from_slice(&VERSION.to_le_bytes());
    plain.extend_from_slice(&project_key(manifest, extensions));
    append_section(&mut plain, &artifact)?;
    append_section(&mut plain, &incremental)?;
    append_section(&mut plain, &snapshot)?;
    let digest = blake3::hash(&plain);
    plain.extend_from_slice(digest.as_bytes());
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&plain)
        .map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

pub(crate) fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<DecodedCompiledCache, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut plain = Vec::new();
    decoder
        .by_ref()
        .take(
            u64::try_from(maximum_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut plain)
        .map_err(|error| error.to_string())?;
    if plain.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    if plain.len() < MAGIC.len() + 4 + 32 + 32 || &plain[..MAGIC.len()] != MAGIC {
        return Err("compiled project cache has an invalid header".into());
    }
    let digest_offset = plain.len() - 32;
    if blake3::hash(&plain[..digest_offset]).as_bytes() != &plain[digest_offset..] {
        return Err("compiled project cache digest mismatch".into());
    }
    let mut cursor = MAGIC.len();
    let version = read_u32(&plain, &mut cursor)?;
    if version != VERSION {
        return Err(format!(
            "unsupported compiled project cache version {version}"
        ));
    }
    let key: [u8; 32] = plain[cursor..cursor + 32]
        .try_into()
        .map_err(|_| "compiled project cache key is truncated")?;
    cursor += 32;
    let artifact_bytes = read_section(&plain[..digest_offset], &mut cursor)?;
    let incremental_bytes = read_section(&plain[..digest_offset], &mut cursor)?;
    let snapshot_bytes = read_section(&plain[..digest_offset], &mut cursor)?;
    if cursor != digest_offset {
        return Err("compiled project cache has trailing data".into());
    }
    let unvalidated = decode_artifact(artifact_bytes, &DecodeLimits::default())
        .map_err(|error| error.to_string())?;
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
        incremental: serde_json::from_slice(incremental_bytes)
            .map_err(|error| error.to_string())?,
        snapshot: serde_json::from_slice(snapshot_bytes).map_err(|error| error.to_string())?,
    })
}

fn append_section(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    output.extend_from_slice(
        &u64::try_from(bytes.len())
            .map_err(|_| "compiled cache section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(bytes);
    Ok(())
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

fn read_section<'a>(bytes: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    let length_end = cursor.saturating_add(8);
    let raw = bytes
        .get(*cursor..length_end)
        .ok_or("compiled project cache is truncated")?;
    *cursor = length_end;
    let length = usize::try_from(u64::from_le_bytes(
        raw.try_into().expect("eight-byte slice"),
    ))
    .map_err(|_| "compiled cache section is not addressable")?;
    let end = cursor
        .checked_add(length)
        .ok_or("compiled cache section length overflow")?;
    let value = bytes
        .get(*cursor..end)
        .ok_or("compiled project cache section is truncated")?;
    *cursor = end;
    Ok(value)
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

        assert_eq!(decoded.key, project_key(&project, &[]));
        assert_eq!(
            decoded.artifact.artifact().manifest.artifact_id,
            build.artifact.unwrap().artifact().manifest.artifact_id
        );
        assert_eq!(
            project_key(&project, &[]),
            project_key(&manifest("@SYSTEM_TITLE\nRETURN\n", 9), &[])
        );
        assert_ne!(
            project_key(&project, &[]),
            project_key(&manifest("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n", 1), &[])
        );
    }

    #[test]
    fn compiled_project_cache_rejects_corruption() {
        assert!(decode(b"not a gzip cache", 1024).is_err());
    }
}
