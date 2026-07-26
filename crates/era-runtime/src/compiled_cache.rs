use std::fmt;
use std::io::{Read, Write};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    ExtensionDeclaration, FilePayload, ProjectIdentity, ProjectManifest, ProtocolDiagnostic,
};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeCallCompatibility, BytecodeEventGroup,
    BytecodeFunction, BytecodeGlobal, Digest, HostImport, NativeImport, SourceMap, SourceMapEntry,
    SourceRecord, SymbolKey,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use rayon::prelude::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::project::NormalizedProjectSnapshot;

const MAGIC: &[u8; 8] = b"RERACACH";
// This is a semantic epoch as well as a wire-format version. Increment it whenever
// compiler, analyzer or project-loading behavior can change an unchanged source's
// artifact; an older artifact is not safe even as an incremental compilation seed.
const VERSION: u32 = 7;
const COMPRESSION_LEVEL: i32 = 3;
const TARGET_PARALLEL_SECTIONS: usize = 32;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SOURCE_SECTION_MAGIC: &[u8; 4] = b"RSM1";
const DIGEST_SECTION_MAGIC: &[u8; 4] = b"RDI1";

#[derive(Serialize)]
struct CompiledCacheMetadataRef<'a> {
    manifest: &'a ArtifactManifest,
    call_compatibility: &'a BytecodeCallCompatibility,
    native_imports: &'a [NativeImport],
    host_imports: &'a [HostImport],
    event_groups: &'a [BytecodeEventGroup],
}

#[derive(Deserialize)]
struct CompiledCacheMetadata {
    manifest: ArtifactManifest,
    call_compatibility: BytecodeCallCompatibility,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
    event_groups: Vec<BytecodeEventGroup>,
}

struct EncodedSectionRef<'a> {
    decoded_length: u64,
    compressed: &'a [u8],
}

struct CompiledCacheSections<'a> {
    key: [u8; 32],
    metadata: EncodedSectionRef<'a>,
    globals: EncodedSectionRef<'a>,
    incremental: EncodedSectionRef<'a>,
    project_data: EncodedSectionRef<'a>,
    sources: EncodedSectionRef<'a>,
    fingerprints: EncodedSectionRef<'a>,
    snapshot: EncodedSectionRef<'a>,
    diagnostics: EncodedSectionRef<'a>,
    functions: Vec<EncodedSectionRef<'a>>,
    source_entries: Vec<EncodedSectionRef<'a>>,
}

struct DecodedCacheParts {
    metadata: CompiledCacheMetadata,
    globals: Vec<BytecodeGlobal>,
    incremental: IncrementalState,
    project_data: erabasic_data::ProjectData,
    sources: Vec<SourceRecord>,
    fingerprints: Vec<Digest>,
    snapshot: NormalizedProjectSnapshot,
    diagnostics: Vec<ProtocolDiagnostic>,
    functions: Vec<BytecodeFunction>,
    source_entries: Vec<SourceMapEntry>,
}

pub(crate) struct DecodedCompiledCache {
    pub(crate) key: [u8; 32],
    pub(crate) artifact: ValidatedArtifact,
    pub(crate) incremental: IncrementalState,
    pub(crate) snapshot: NormalizedProjectSnapshot,
    pub(crate) diagnostics: Vec<ProtocolDiagnostic>,
}

/// Error returned when a compiled-project cache cannot expose its source manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledProjectCacheError {
    message: String,
}

impl fmt::Display for CompiledProjectCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CompiledProjectCacheError {}

impl From<String> for CompiledProjectCacheError {
    fn from(message: String) -> Self {
        Self { message }
    }
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

#[cfg(test)]
pub(crate) fn encode(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
) -> Result<Vec<u8>, String> {
    encode_inner(
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        None,
    )
}

pub(crate) fn encode_cancellable(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, String> {
    encode_inner(
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        Some(cancelled),
    )
}

fn encode_inner(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err("compiled cache build cancelled".into());
    }
    let artifact = artifact.artifact();
    let function_ranges = weighted_function_ranges(&artifact.functions);
    let source_ranges = equal_ranges(artifact.source_map.entries.len());
    let metadata = encode_section(
        &CompiledCacheMetadataRef {
            manifest: &artifact.manifest,
            call_compatibility: &artifact.call_compatibility,
            native_imports: &artifact.native_imports,
            host_imports: &artifact.host_imports,
            event_groups: &artifact.event_groups,
        },
        cancelled,
    )?;
    let ((globals, incremental), (project_data, (sources, (fingerprints, snapshot)))) = rayon::join(
        || {
            rayon::join(
                || encode_section(&artifact.globals, cancelled),
                || encode_section(incremental, cancelled),
            )
        },
        || {
            rayon::join(
                || encode_section(&artifact.project_data, cancelled),
                || {
                    rayon::join(
                        || encode_section(&artifact.source_map.sources, cancelled),
                        || {
                            rayon::join(
                                || {
                                    encode_digest_section(
                                        &artifact.source_map.statement_fingerprints,
                                        cancelled,
                                    )
                                },
                                || encode_section(snapshot, cancelled),
                            )
                        },
                    )
                },
            )
        },
    );
    let globals = globals?;
    let incremental = incremental?;
    let project_data = project_data?;
    let sources = sources?;
    let fingerprints = fingerprints?;
    let snapshot = snapshot?;
    let diagnostics = encode_section(diagnostics, cancelled)?;
    let function_sections = function_ranges
        .par_iter()
        .map(|range| encode_section(&artifact.functions[range.clone()], cancelled))
        .collect::<Result<Vec<_>, _>>()?;
    let source_sections = source_ranges
        .par_iter()
        .map(|range| encode_source_section(&artifact.source_map.entries[range.clone()], cancelled))
        .collect::<Result<Vec<_>, _>>()?;
    let section_bytes = metadata.len()
        + globals.len()
        + incremental.len()
        + project_data.len()
        + sources.len()
        + fingerprints.len()
        + snapshot.len()
        + diagnostics.len()
        + function_sections.iter().map(Vec::len).sum::<usize>()
        + source_sections.iter().map(Vec::len).sum::<usize>();
    let mut output = Vec::with_capacity(
        MAGIC.len() + 4 + 32 + 4 + 4 + section_bytes + std::mem::size_of::<Digest>(),
    );
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&VERSION.to_le_bytes());
    output.extend_from_slice(&project_key(&project_identity(manifest), extensions));
    output.extend_from_slice(
        &u32::try_from(function_sections.len())
            .map_err(|_| "compiled cache has too many function sections")?
            .to_le_bytes(),
    );
    output.extend_from_slice(
        &u32::try_from(source_sections.len())
            .map_err(|_| "compiled cache has too many source sections")?
            .to_le_bytes(),
    );
    output.extend_from_slice(&metadata);
    output.extend_from_slice(&globals);
    output.extend_from_slice(&incremental);
    output.extend_from_slice(&project_data);
    output.extend_from_slice(&sources);
    output.extend_from_slice(&fingerprints);
    output.extend_from_slice(&snapshot);
    output.extend_from_slice(&diagnostics);
    for section in function_sections.iter().chain(&source_sections) {
        output.extend_from_slice(section);
    }
    let digest = blake3::hash(&output);
    output.extend_from_slice(digest.as_bytes());
    Ok(output)
}

pub(crate) fn decode(bytes: &[u8], maximum_bytes: usize) -> Result<DecodedCompiledCache, String> {
    let sections = parse_cache_sections(bytes, maximum_bytes)?;
    let parts = decode_cache_parts(&sections)?;
    let artifact = BytecodeArtifact {
        manifest: parts.metadata.manifest,
        call_compatibility: parts.metadata.call_compatibility,
        project_data: parts.project_data,
        globals: parts.globals,
        native_imports: parts.metadata.native_imports,
        host_imports: parts.metadata.host_imports,
        functions: parts.functions,
        event_groups: parts.metadata.event_groups,
        source_map: SourceMap {
            sources: parts.sources,
            statement_fingerprints: parts.fingerprints,
            entries: parts.source_entries,
        },
    };
    let unvalidated = artifact.into_unvalidated();
    let context = ValidationContext::for_artifact(unvalidated.artifact());
    let validation = validate_bytecode(unvalidated, &context);
    let artifact = validation.value.ok_or_else(|| {
        validation.diagnostics.first().map_or_else(
            || "cached artifact failed validation".into(),
            |value| value.message.clone(),
        )
    })?;
    Ok(DecodedCompiledCache {
        key: sections.key,
        artifact,
        incremental: parts.incremental,
        snapshot: parts.snapshot,
        diagnostics: parts.diagnostics,
    })
}

/// Decode the exact frontend-submitted project manifest embedded in a compiled cache.
///
/// This source-only projection reuses the runtime cache parser but deliberately avoids
/// decoding or validating bytecode sections that an extraction tool does not consume.
///
/// # Errors
///
/// Returns an error when the cache is over the caller's limit, corrupt, unsupported, or
/// does not contain a decodable project snapshot.
pub fn decode_compiled_project_manifest(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<ProjectManifest, CompiledProjectCacheError> {
    let sections =
        parse_cache_sections(bytes, maximum_bytes).map_err(CompiledProjectCacheError::from)?;
    let snapshot = decode_section::<NormalizedProjectSnapshot>(&sections.snapshot)
        .map_err(CompiledProjectCacheError::from)?;
    Ok(snapshot.manifest)
}

fn parse_cache_sections(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<CompiledCacheSections<'_>, String> {
    if bytes.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    let minimum = MAGIC.len() + 4 + 32 + 4 + 4 + 8 * 16 + 32;
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
    let function_section_count = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| "compiled cache function section count is not addressable")?;
    let source_section_count = usize::try_from(read_u32(bytes, &mut cursor)?)
        .map_err(|_| "compiled cache source section count is not addressable")?;
    if function_section_count > TARGET_PARALLEL_SECTIONS.saturating_mul(2)
        || source_section_count > TARGET_PARALLEL_SECTIONS
    {
        return Err("compiled project cache has too many parallel sections".into());
    }
    let metadata = read_section(bytes, &mut cursor, digest_offset)?;
    let globals = read_section(bytes, &mut cursor, digest_offset)?;
    let incremental = read_section(bytes, &mut cursor, digest_offset)?;
    let project_data = read_section(bytes, &mut cursor, digest_offset)?;
    let sources = read_section(bytes, &mut cursor, digest_offset)?;
    let fingerprints = read_section(bytes, &mut cursor, digest_offset)?;
    let snapshot = read_section(bytes, &mut cursor, digest_offset)?;
    let diagnostics = read_section(bytes, &mut cursor, digest_offset)?;
    let mut functions = Vec::with_capacity(function_section_count);
    for _ in 0..function_section_count {
        functions.push(read_section(bytes, &mut cursor, digest_offset)?);
    }
    let mut source_entries = Vec::with_capacity(source_section_count);
    for _ in 0..source_section_count {
        source_entries.push(read_section(bytes, &mut cursor, digest_offset)?);
    }
    if cursor != digest_offset {
        return Err("compiled project cache has trailing data".into());
    }
    let decoded_bytes = [
        &metadata,
        &globals,
        &incremental,
        &project_data,
        &sources,
        &fingerprints,
        &snapshot,
        &diagnostics,
    ]
    .into_iter()
    .chain(&functions)
    .chain(&source_entries)
    .try_fold(0_u64, |total, section| {
        total.checked_add(section.decoded_length)
    })
    .ok_or("compiled cache decoded length overflow")?;
    if decoded_bytes > MAXIMUM_DECODED_PAYLOAD_BYTES {
        return Err("compiled cache decoded sections exceed their limit".into());
    }
    Ok(CompiledCacheSections {
        key,
        metadata,
        globals,
        incremental,
        project_data,
        sources,
        fingerprints,
        snapshot,
        diagnostics,
        functions,
        source_entries,
    })
}

fn decode_cache_parts(sections: &CompiledCacheSections<'_>) -> Result<DecodedCacheParts, String> {
    let diagnostics = decode_section::<Vec<ProtocolDiagnostic>>(&sections.diagnostics)?;
    let (
        (metadata, (globals, incremental)),
        (project_data, (sources, (fingerprints, (snapshot, (function_chunks, source_chunks))))),
    ) = rayon::join(
        || {
            rayon::join(
                || decode_section::<CompiledCacheMetadata>(&sections.metadata),
                || {
                    rayon::join(
                        || decode_section::<Vec<BytecodeGlobal>>(&sections.globals),
                        || decode_section::<IncrementalState>(&sections.incremental),
                    )
                },
            )
        },
        || {
            rayon::join(
                || decode_section::<erabasic_data::ProjectData>(&sections.project_data),
                || {
                    rayon::join(
                        || decode_section::<Vec<SourceRecord>>(&sections.sources),
                        || {
                            rayon::join(
                                || decode_digest_section(&sections.fingerprints),
                                || {
                                    rayon::join(
                                        || {
                                            decode_section::<NormalizedProjectSnapshot>(
                                                &sections.snapshot,
                                            )
                                        },
                                        || {
                                            rayon::join(
                                                || decode_function_sections(&sections.functions),
                                                || decode_source_sections(&sections.source_entries),
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                },
            )
        },
    );
    let metadata = metadata?;
    let globals = globals?;
    let incremental = incremental?;
    let project_data = project_data?;
    let sources = sources?;
    let fingerprints = fingerprints?;
    let snapshot = snapshot?;
    let function_chunks = function_chunks?;
    let source_chunks = source_chunks?;
    let mut functions = Vec::with_capacity(function_chunks.iter().map(Vec::len).sum());
    for mut chunk in function_chunks {
        functions.append(&mut chunk);
    }
    let mut entries = Vec::with_capacity(source_chunks.iter().map(Vec::len).sum());
    for mut chunk in source_chunks {
        entries.append(&mut chunk);
    }
    Ok(DecodedCacheParts {
        metadata,
        globals,
        incremental,
        project_data,
        sources,
        fingerprints,
        snapshot,
        diagnostics,
        functions,
        source_entries: entries,
    })
}

fn decode_function_sections(
    sections: &[EncodedSectionRef<'_>],
) -> Result<Vec<Vec<BytecodeFunction>>, String> {
    sections
        .par_iter()
        .map(decode_section::<Vec<BytecodeFunction>>)
        .collect()
}

fn decode_source_sections(
    sections: &[EncodedSectionRef<'_>],
) -> Result<Vec<Vec<SourceMapEntry>>, String> {
    sections.par_iter().map(decode_source_section).collect()
}

fn encode_section<T: Serialize + ?Sized>(
    value: &T,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)
        .map_err(|error| error.to_string())?;
    let mut writer = CountingWriter::new(encoder, cancelled);
    rmp_serde::encode::write(&mut writer, value).map_err(|error| error.to_string())?;
    let decoded_length = writer.bytes;
    let compressed = writer
        .into_inner()
        .finish()
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(16 + compressed.len());
    output.extend_from_slice(&decoded_length.to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(compressed.len())
            .map_err(|_| "compiled cache section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(&compressed);
    Ok(output)
}

fn encode_digest_section(
    digests: &[Digest],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err("compiled cache build cancelled".into());
    }
    let mut decoded =
        Vec::with_capacity(DIGEST_SECTION_MAGIC.len() + 8 + digests.len().saturating_mul(32));
    decoded.extend_from_slice(DIGEST_SECTION_MAGIC);
    decoded.extend_from_slice(
        &u64::try_from(digests.len())
            .map_err(|_| "compiled cache digest count is too large")?
            .to_le_bytes(),
    );
    for digest in digests {
        decoded.extend_from_slice(&digest.0);
    }
    let decoded_length =
        u64::try_from(decoded.len()).map_err(|_| "compiled cache digest section is too large")?;
    let compressed =
        zstd::bulk::compress(&decoded, COMPRESSION_LEVEL).map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(16 + compressed.len());
    output.extend_from_slice(&decoded_length.to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(compressed.len())
            .map_err(|_| "compiled cache digest section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(&compressed);
    Ok(output)
}

fn decode_digest_section(section: &EncodedSectionRef<'_>) -> Result<Vec<Digest>, String> {
    let decoded_length = usize::try_from(section.decoded_length)
        .map_err(|_| "compiled cache digest section is not addressable")?;
    let decoded = zstd::bulk::decompress(section.compressed, decoded_length)
        .map_err(|error| error.to_string())?;
    if decoded.len() != decoded_length {
        return Err("compiled cache decoded digest section length differs".into());
    }
    if decoded.get(..DIGEST_SECTION_MAGIC.len()) != Some(DIGEST_SECTION_MAGIC) {
        return Err("compiled cache digest section has an invalid header".into());
    }
    let mut cursor = DIGEST_SECTION_MAGIC.len();
    let count = usize::try_from(read_u64(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache digest count is not addressable")?;
    let expected_length = cursor
        .checked_add(
            count
                .checked_mul(32)
                .ok_or("compiled cache digest section length overflow")?,
        )
        .ok_or("compiled cache digest section length overflow")?;
    if expected_length != decoded.len() {
        return Err("compiled cache digest section length is invalid".into());
    }
    let mut digests = Vec::new();
    digests
        .try_reserve_exact(count)
        .map_err(|_| "compiled cache digest allocation failed")?;
    while cursor < decoded.len() {
        let end = cursor + 32;
        digests.push(Digest(
            decoded[cursor..end].try_into().expect("32-byte slice"),
        ));
        cursor = end;
    }
    Ok(digests)
}

/// Encode source locations without repeating a textual function key for every instruction range.
///
/// Source-map entries are already canonicalized by `(function, code_start, code_end)`. The cache
/// groups each contiguous function and delta-encodes code ranges while retaining every source and
/// origin field. This is deliberately a cache-local representation; the public bytecode and JSON
/// representations stay unchanged.
fn encode_source_section(
    entries: &[SourceMapEntry],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    let mut decoded = Vec::with_capacity(entries.len().saturating_mul(24));
    decoded.extend_from_slice(SOURCE_SECTION_MAGIC);
    decoded.extend_from_slice(
        &u64::try_from(entries.len())
            .map_err(|_| "compiled cache source entry count is too large")?
            .to_le_bytes(),
    );
    decoded.extend_from_slice(
        &u32::try_from(group_count)
            .map_err(|_| "compiled cache source group count is too large")?
            .to_le_bytes(),
    );

    let mut group_start = 0;
    while group_start < entries.len() {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("compiled cache build cancelled".into());
        }
        let function = entries[group_start].function;
        let group_length =
            entries[group_start..].partition_point(|entry| entry.function == function);
        decoded.extend_from_slice(&function.0);
        decoded.extend_from_slice(
            &u32::try_from(group_length)
                .map_err(|_| "compiled cache source group is too large")?
                .to_le_bytes(),
        );
        let mut previous_code_end = 0_u64;
        for entry in &entries[group_start..group_start + group_length] {
            append_varint(
                &mut decoded,
                entry
                    .code_start
                    .checked_sub(previous_code_end)
                    .ok_or("source-map code ranges are not ordered")?,
            );
            append_varint(
                &mut decoded,
                entry
                    .code_end
                    .checked_sub(entry.code_start)
                    .ok_or("source-map code range is reversed")?,
            );
            append_varint(&mut decoded, entry.byte_start);
            append_varint(
                &mut decoded,
                entry
                    .byte_end
                    .checked_sub(entry.byte_start)
                    .ok_or("source-map byte range is reversed")?,
            );
            append_varint(&mut decoded, u64::from(entry.statement_fingerprint));
            append_varint(&mut decoded, u64::from(entry.source_index));
            match entry.origin_chain.as_deref() {
                None => append_varint(&mut decoded, 0),
                Some(origins) => {
                    append_varint(
                        &mut decoded,
                        u64::try_from(origins.len())
                            .map_err(|_| "source-map origin chain is too long")?
                            .checked_add(1)
                            .ok_or("source-map origin chain is too long")?,
                    );
                    for &(source_index, byte_start, byte_end) in origins {
                        append_varint(&mut decoded, u64::from(source_index));
                        append_varint(&mut decoded, byte_start);
                        append_varint(
                            &mut decoded,
                            byte_end
                                .checked_sub(byte_start)
                                .ok_or("source-map origin byte range is reversed")?,
                        );
                    }
                }
            }
            previous_code_end = entry.code_end;
        }
        group_start += group_length;
    }
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err("compiled cache build cancelled".into());
    }
    let decoded_length =
        u64::try_from(decoded.len()).map_err(|_| "compiled cache source section is too large")?;
    let compressed =
        zstd::bulk::compress(&decoded, COMPRESSION_LEVEL).map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(16 + compressed.len());
    output.extend_from_slice(&decoded_length.to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(compressed.len())
            .map_err(|_| "compiled cache source section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(&compressed);
    Ok(output)
}

fn decode_source_section(section: &EncodedSectionRef<'_>) -> Result<Vec<SourceMapEntry>, String> {
    let decoded_length = usize::try_from(section.decoded_length)
        .map_err(|_| "compiled cache source section is not addressable")?;
    let decoded = zstd::bulk::decompress(section.compressed, decoded_length)
        .map_err(|error| error.to_string())?;
    if decoded.len() != decoded_length {
        return Err("compiled cache decoded source section length differs".into());
    }
    if decoded.get(..SOURCE_SECTION_MAGIC.len()) != Some(SOURCE_SECTION_MAGIC) {
        return Err("compiled cache source section has an invalid header".into());
    }
    let mut cursor = SOURCE_SECTION_MAGIC.len();
    let entry_count = usize::try_from(read_u64(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache source entry count is not addressable")?;
    let group_count = usize::try_from(read_u32(&decoded, &mut cursor)?)
        .map_err(|_| "compiled cache source group count is not addressable")?;
    if (entry_count == 0) != (group_count == 0) || group_count > entry_count {
        return Err("compiled cache source group count is invalid".into());
    }
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(entry_count)
        .map_err(|_| "compiled cache source entry allocation failed")?;
    for _ in 0..group_count {
        let function_end = cursor.saturating_add(16);
        let function = SymbolKey(
            decoded
                .get(cursor..function_end)
                .ok_or("compiled cache source function key is truncated")?
                .try_into()
                .expect("16-byte slice"),
        );
        cursor = function_end;
        let group_length = usize::try_from(read_u32(&decoded, &mut cursor)?)
            .map_err(|_| "compiled cache source group length is not addressable")?;
        if group_length == 0 || group_length > entry_count.saturating_sub(entries.len()) {
            return Err("compiled cache source group length is invalid".into());
        }
        let mut previous_code_end = 0_u64;
        for _ in 0..group_length {
            let code_start = previous_code_end
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code offset overflow")?;
            let code_end = code_start
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code range overflow")?;
            let byte_start = read_varint(&decoded, &mut cursor)?;
            let byte_end = byte_start
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source byte range overflow")?;
            let statement_fingerprint = u32::try_from(read_varint(&decoded, &mut cursor)?)
                .map_err(|_| "compiled cache statement fingerprint is out of range")?;
            let source_index = u32::try_from(read_varint(&decoded, &mut cursor)?)
                .map_err(|_| "compiled cache source index is out of range")?;
            let encoded_origin_count = read_varint(&decoded, &mut cursor)?;
            let origin_chain = if encoded_origin_count == 0 {
                None
            } else {
                let origin_count = usize::try_from(encoded_origin_count - 1)
                    .map_err(|_| "compiled cache source origin count is not addressable")?;
                if origin_count > decoded.len().saturating_sub(cursor) / 3 {
                    return Err("compiled cache source origin count is invalid".into());
                }
                let mut origins = Vec::new();
                origins
                    .try_reserve_exact(origin_count)
                    .map_err(|_| "compiled cache source origin allocation failed")?;
                for _ in 0..origin_count {
                    let source_index = u32::try_from(read_varint(&decoded, &mut cursor)?)
                        .map_err(|_| "compiled cache origin source index is out of range")?;
                    let byte_start = read_varint(&decoded, &mut cursor)?;
                    let byte_end = byte_start
                        .checked_add(read_varint(&decoded, &mut cursor)?)
                        .ok_or("compiled cache source origin byte range overflow")?;
                    origins.push((source_index, byte_start, byte_end));
                }
                Some(Box::new(origins))
            };
            entries.push(SourceMapEntry {
                function,
                code_start,
                code_end,
                byte_start,
                byte_end,
                statement_fingerprint,
                origin_chain,
                source_index,
            });
            previous_code_end = code_end;
        }
    }
    if entries.len() != entry_count || cursor != decoded.len() {
        return Err("compiled cache source section has trailing or missing entries".into());
    }
    Ok(entries)
}

fn append_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        output.push(u8::try_from(value & 0x7f).expect("masked varint byte fits in u8") | 0x80);
        value >>= 7;
    }
    output.push(u8::try_from(value).expect("final varint byte fits in u8"));
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut value = 0_u64;
    for index in 0..10 {
        let byte = *bytes
            .get(*cursor)
            .ok_or("compiled cache source varint is truncated")?;
        *cursor += 1;
        if index == 9 && byte > 1 {
            return Err("compiled cache source varint overflows u64".into());
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("compiled cache source varint is too long".into())
}

fn read_section<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    digest_offset: usize,
) -> Result<EncodedSectionRef<'a>, String> {
    let decoded_length = read_u64(bytes, cursor)?;
    let compressed_length = usize::try_from(read_u64(bytes, cursor)?)
        .map_err(|_| "compiled cache section is not addressable")?;
    let end = cursor
        .checked_add(compressed_length)
        .ok_or("compiled cache section length overflow")?;
    if end > digest_offset {
        return Err("compiled cache section is truncated".into());
    }
    let compressed = &bytes[*cursor..end];
    *cursor = end;
    Ok(EncodedSectionRef {
        decoded_length,
        compressed,
    })
}

fn decode_section<T: DeserializeOwned>(section: &EncodedSectionRef<'_>) -> Result<T, String> {
    let decoder =
        zstd::stream::read::Decoder::new(section.compressed).map_err(|error| error.to_string())?;
    let mut reader = CountingReader::new(decoder.take(section.decoded_length.saturating_add(1)));
    let value = rmp_serde::from_read(&mut reader).map_err(|error| error.to_string())?;
    let mut tail = [0_u8; 1];
    if reader.read(&mut tail).map_err(|error| error.to_string())? != 0
        || reader.bytes != section.decoded_length
    {
        return Err("compiled cache decoded section length differs".into());
    }
    Ok(value)
}

fn weighted_function_ranges(functions: &[BytecodeFunction]) -> Vec<Range<usize>> {
    if functions.is_empty() {
        return Vec::new();
    }
    let total_weight = functions
        .iter()
        .map(|function| function.code.len().max(1))
        .sum::<usize>();
    let target_weight = total_weight.div_ceil(TARGET_PARALLEL_SECTIONS).max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut weight = 0_usize;
    for (index, function) in functions.iter().enumerate() {
        weight = weight.saturating_add(function.code.len().max(1));
        if weight >= target_weight && ranges.len() + 1 < TARGET_PARALLEL_SECTIONS {
            ranges.push(start..index + 1);
            start = index + 1;
            weight = 0;
        }
    }
    if start < functions.len() {
        ranges.push(start..functions.len());
    }
    ranges
}

fn equal_ranges(length: usize) -> Vec<Range<usize>> {
    if length == 0 {
        return Vec::new();
    }
    let chunk_length = length.div_ceil(TARGET_PARALLEL_SECTIONS);
    (0..length)
        .step_by(chunk_length)
        .map(|start| start..start.saturating_add(chunk_length).min(length))
        .collect()
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

struct CountingWriter<'a, W> {
    inner: W,
    bytes: u64,
    cancelled: Option<&'a AtomicBool>,
}

impl<'a, W> CountingWriter<'a, W> {
    const fn new(inner: W, cancelled: Option<&'a AtomicBool>) -> Self {
        Self {
            inner,
            bytes: 0,
            cancelled,
        }
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "compiled cache build cancelled",
            ));
        }
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
    use std::fmt::Write as _;

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
            &build.report.diagnostics,
        )
        .unwrap();
        let decoded = decode(&bytes, 64 * 1024 * 1024).unwrap();
        let decoded_manifest = decode_compiled_project_manifest(&bytes, 64 * 1024 * 1024).unwrap();

        assert_eq!(decoded.key, project_key(&project_identity(&project), &[]));
        assert_eq!(decoded_manifest, project);
        assert_eq!(decoded.diagnostics, build.report.diagnostics);
        assert_eq!(
            decoded.artifact.artifact(),
            build.artifact.as_ref().unwrap().artifact()
        );
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
        assert!(decode_compiled_project_manifest(b"not a compiled cache", 1024).is_err());
    }

    #[test]
    fn source_manifest_projection_honors_limits_and_cache_version() {
        let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
        let mut build = crate::project::build_project(&project, None);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        build.incremental.compact();
        let mut bytes = encode(
            &project,
            &[],
            build.artifact.as_ref().unwrap(),
            &build.incremental,
            build.snapshot.as_ref().unwrap(),
            &build.report.diagnostics,
        )
        .unwrap();

        assert!(decode_compiled_project_manifest(&bytes, bytes.len() - 1).is_err());
        bytes[8..12].copy_from_slice(&4_u32.to_le_bytes());
        let digest_offset = bytes.len() - 32;
        let digest = blake3::hash(&bytes[..digest_offset]);
        bytes[digest_offset..].copy_from_slice(digest.as_bytes());
        assert!(
            decode_compiled_project_manifest(&bytes, 64 * 1024 * 1024)
                .unwrap_err()
                .to_string()
                .contains("unsupported compiled project cache version 4")
        );
    }

    #[test]
    fn compiled_project_cache_encoding_honors_cancellation() {
        let project = manifest("@SYSTEM_TITLE\nRETURN\n", 1);
        let mut build = crate::project::build_project(&project, None);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        build.incremental.compact();
        let cancelled = AtomicBool::new(true);

        let error = encode_cancellable(
            &project,
            &[],
            build.artifact.as_ref().unwrap(),
            &build.incremental,
            build.snapshot.as_ref().unwrap(),
            &build.report.diagnostics,
            &cancelled,
        )
        .unwrap_err();

        assert_eq!(error, "compiled cache build cancelled");
    }

    #[test]
    fn compact_source_section_preserves_none_and_empty_origin_chains() {
        let function = SymbolKey::derive("compiled-cache-source-test", b"function");
        let entries = vec![
            SourceMapEntry {
                function,
                code_start: 0,
                code_end: 1,
                byte_start: 4,
                byte_end: 5,
                statement_fingerprint: 2,
                origin_chain: None,
                source_index: 3,
            },
            SourceMapEntry {
                function,
                code_start: 1,
                code_end: 2,
                byte_start: 6,
                byte_end: 7,
                statement_fingerprint: 8,
                origin_chain: Some(Box::default()),
                source_index: 9,
            },
        ];
        let encoded = encode_source_section(&entries, None).unwrap();
        let mut cursor = 0;
        let section = read_section(&encoded, &mut cursor, encoded.len()).unwrap();

        assert_eq!(decode_source_section(&section).unwrap(), entries);
        assert_eq!(cursor, encoded.len());
    }

    #[test]
    fn sharded_binary_cache_bounds_small_project_parallel_overhead() {
        #[derive(Serialize)]
        struct V2PayloadRef<'a> {
            artifact: &'a BytecodeArtifact,
            incremental: &'a IncrementalState,
            snapshot: &'a NormalizedProjectSnapshot,
            diagnostics: &'a [ProtocolDiagnostic],
        }

        const MAXIMUM_PARALLEL_SECTION_OVERHEAD: usize = 16 * 1024;

        let mut source = "@SYSTEM_TITLE\nRETURN\n".to_owned();
        for index in 0..256 {
            write!(
                source,
                "@CACHE_SIZE_{index}\nPRINTL repeated compiled cache payload\nRETURN\n"
            )
            .unwrap();
        }
        let project = manifest(&source, 1);
        let mut build = crate::project::build_project(&project, None);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        build.incremental.compact();
        let payload = V2PayloadRef {
            artifact: build.artifact.as_ref().unwrap().artifact(),
            incremental: &build.incremental,
            snapshot: build.snapshot.as_ref().unwrap(),
            diagnostics: &build.report.diagnostics,
        };
        let encoder = zstd::stream::Encoder::new(Vec::new(), 7).unwrap();
        let mut writer = CountingWriter::new(encoder, None);
        serde_json::to_writer(&mut writer, &payload).unwrap();
        let v2_payload = writer.into_inner().finish().unwrap();
        let cache_bytes = encode(
            &project,
            &[],
            build.artifact.as_ref().unwrap(),
            &build.incremental,
            build.snapshot.as_ref().unwrap(),
            &build.report.diagnostics,
        )
        .unwrap();

        assert!(
            cache_bytes.len() <= v2_payload.len() + MAXIMUM_PARALLEL_SECTION_OVERHEAD,
            "encoded={} v2={}",
            cache_bytes.len(),
            v2_payload.len()
        );
    }
}
