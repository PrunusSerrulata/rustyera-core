use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    ExtensionDeclaration, FileCategory, FilePayload, ProjectIdentity, ProjectManifest,
    ProtocolDiagnostic, SubmittedFile, validate_relative_path,
};
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeCallCompatibility, BytecodeEventGroup,
    BytecodeFunction, BytecodeGlobal, Digest, HostImport, NativeImport, SourceMap, SourceMapEntry,
    SourceRecord, SymbolKey,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::{ValidatedArtifact, ValidationContext, validate_bytecode};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::project::{NormalizedProjectSnapshot, NormalizedResourceIdentity};
use crate::resource::ResourceGraph;

const MAGIC: &[u8; 8] = b"RERAPROJ";
// Project files use a compact byte-sized format version. This is also a semantic epoch:
// increment it whenever compiler, analyzer or project-loading behavior can change an
// unchanged source's artifact. Older project files are then rejected instead of being used
// as an incremental compilation seed.
const VERSION: u8 = 3;
const COMPRESSION_LEVEL: i32 = 3;
const TARGET_PARALLEL_SECTIONS: usize = 32;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SOURCE_SECTION_MAGIC: &[u8; 4] = b"RSM2";
const DIGEST_SECTION_MAGIC: &[u8; 4] = b"RDI2";
const MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF2";
const SOURCE_RECORD_SECTION_MAGIC: &[u8; 4] = b"RSR2";
const INCREMENTAL_SECTION_MAGIC: &[u8; 4] = b"RIC2";

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
    identity: ProjectIdentity,
    key: [u8; 32],
    metadata: EncodedSectionRef<'a>,
    globals: EncodedSectionRef<'a>,
    incremental: EncodedSectionRef<'a>,
    project_data: EncodedSectionRef<'a>,
    sources: EncodedSectionRef<'a>,
    fingerprints: EncodedSectionRef<'a>,
    manifest: EncodedSectionRef<'a>,
    snapshot: EncodedSectionRef<'a>,
    diagnostics: EncodedSectionRef<'a>,
    functions: Vec<EncodedSectionRef<'a>>,
    source_entries: Vec<EncodedSectionRef<'a>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct CompiledSnapshotMetadata {
    project_identity: [u8; 32],
    resources: Vec<NormalizedResourceIdentity>,
    sort_with_filename: bool,
    use_new_random_ignored: bool,
    auto_save: bool,
    ctrl_z_enabled: bool,
    allow_long_input_by_activation: bool,
    save_in_binary: bool,
    compress_save: bool,
    save_slot_count: u32,
    money_label: String,
    money_first: bool,
    maximum_shop_items: u32,
    viewport_width: u32,
    viewport_height: u32,
    font_size: u32,
    line_height: u32,
    print_c_per_line: u32,
    print_c_length: u32,
    configuration: erabasic_config::ConfigStore,
    editable_configuration: erabasic_config::ConfigStore,
    extensions: std::collections::BTreeMap<String, ExtensionDeclaration>,
}

impl From<&NormalizedProjectSnapshot> for CompiledSnapshotMetadata {
    fn from(snapshot: &NormalizedProjectSnapshot) -> Self {
        Self {
            project_identity: snapshot.project_identity,
            resources: snapshot.resources.clone(),
            sort_with_filename: snapshot.sort_with_filename,
            use_new_random_ignored: snapshot.use_new_random_ignored,
            auto_save: snapshot.auto_save,
            ctrl_z_enabled: snapshot.ctrl_z_enabled,
            allow_long_input_by_activation: snapshot.allow_long_input_by_activation,
            save_in_binary: snapshot.save_in_binary,
            compress_save: snapshot.compress_save,
            save_slot_count: snapshot.save_slot_count,
            money_label: snapshot.money_label.clone(),
            money_first: snapshot.money_first,
            maximum_shop_items: snapshot.maximum_shop_items,
            viewport_width: snapshot.viewport_width,
            viewport_height: snapshot.viewport_height,
            font_size: snapshot.font_size,
            line_height: snapshot.line_height,
            print_c_per_line: snapshot.print_c_per_line,
            print_c_length: snapshot.print_c_length,
            configuration: snapshot.configuration.clone(),
            editable_configuration: snapshot.editable_configuration.clone(),
            extensions: snapshot.extensions.clone(),
        }
    }
}

impl CompiledSnapshotMetadata {
    fn into_snapshot(self, manifest: ProjectManifest) -> Result<NormalizedProjectSnapshot, String> {
        let (resource_graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
        if let Some(diagnostic) = diagnostics.into_iter().find(|value| value.error) {
            return Err(format!(
                "embedded project resources cannot be rebuilt: {}",
                diagnostic.message
            ));
        }
        Ok(NormalizedProjectSnapshot {
            manifest: std::sync::Arc::new(manifest),
            project_identity: self.project_identity,
            resources: self.resources,
            resource_graph,
            sort_with_filename: self.sort_with_filename,
            use_new_random_ignored: self.use_new_random_ignored,
            auto_save: self.auto_save,
            ctrl_z_enabled: self.ctrl_z_enabled,
            allow_long_input_by_activation: self.allow_long_input_by_activation,
            save_in_binary: self.save_in_binary,
            compress_save: self.compress_save,
            save_slot_count: self.save_slot_count,
            money_label: self.money_label,
            money_first: self.money_first,
            maximum_shop_items: self.maximum_shop_items,
            viewport_width: self.viewport_width,
            viewport_height: self.viewport_height,
            font_size: self.font_size,
            line_height: self.line_height,
            print_c_per_line: self.print_c_per_line,
            print_c_length: self.print_c_length,
            configuration: self.configuration,
            editable_configuration: self.editable_configuration,
            extensions: self.extensions,
        })
    }
}

struct DecodedCacheParts {
    metadata: CompiledCacheMetadata,
    globals: Vec<BytecodeGlobal>,
    incremental_cache_keys: Vec<Digest>,
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

/// Error returned when a `RustyEra` project file cannot be decoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectFileError {
    message: String,
}

impl fmt::Display for ProjectFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProjectFileError {}

impl From<String> for ProjectFileError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

/// Frontend-facing data embedded in a self-contained `RustyEra` project file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedProjectFile {
    pub identity: ProjectIdentity,
    pub manifest: ProjectManifest,
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
    let snapshot = CompiledSnapshotMetadata::from(snapshot);
    let cache_keys = incremental.compact_cache_keys(artifact.artifact())?;
    encode_inner(
        manifest,
        extensions,
        artifact,
        &cache_keys,
        &snapshot,
        diagnostics,
        None,
    )
}

pub(crate) fn encode_cancellable(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    cache_keys: &[Digest],
    snapshot: &CompiledSnapshotMetadata,
    diagnostics: &[ProtocolDiagnostic],
    cancelled: &AtomicBool,
) -> Result<Vec<u8>, String> {
    encode_inner(
        manifest,
        extensions,
        artifact,
        cache_keys,
        snapshot,
        diagnostics,
        Some(cancelled),
    )
}

#[allow(clippy::too_many_lines)]
fn encode_inner(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    cache_keys: &[Digest],
    snapshot: &CompiledSnapshotMetadata,
    diagnostics: &[ProtocolDiagnostic],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err("compiled cache build cancelled".into());
    }
    let artifact = artifact.artifact();
    let function_indices = artifact
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.key, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let function_ranges = weighted_function_ranges(&artifact.functions);
    let source_ranges = equal_ranges(artifact.source_map.entries.len());
    let mut metadata = encode_section(
        &CompiledCacheMetadataRef {
            manifest: &artifact.manifest,
            call_compatibility: &artifact.call_compatibility,
            native_imports: &artifact.native_imports,
            host_imports: &artifact.host_imports,
            event_groups: &artifact.event_groups,
        },
        cancelled,
    )?;
    let (
        (globals, incremental),
        (project_data, (sources, (fingerprints, (manifest_result, snapshot)))),
    ) = rayon::join(
        || {
            rayon::join(
                || encode_section(&artifact.globals, cancelled),
                || encode_incremental_section(cache_keys, cancelled),
            )
        },
        || {
            rayon::join(
                || encode_section(&artifact.project_data, cancelled),
                || {
                    rayon::join(
                        || {
                            encode_source_record_section(
                                &artifact.source_map.sources,
                                manifest,
                                cancelled,
                            )
                        },
                        || {
                            rayon::join(
                                || {
                                    encode_digest_section(
                                        &artifact.source_map.statement_fingerprints,
                                        cancelled,
                                    )
                                },
                                || {
                                    rayon::join(
                                        || encode_manifest_section(manifest, cancelled),
                                        || encode_section(snapshot, cancelled),
                                    )
                                },
                            )
                        },
                    )
                },
            )
        },
    );
    let mut globals = globals?;
    let mut incremental = incremental?;
    let mut project_data = project_data?;
    let mut sources = sources?;
    let mut fingerprints = fingerprints?;
    let mut manifest_section = manifest_result?;
    let mut snapshot = snapshot?;
    let mut diagnostics = encode_section(diagnostics, cancelled)?;
    let mut function_sections = function_ranges
        .par_iter()
        .map(|range| encode_section(&artifact.functions[range.clone()], cancelled))
        .collect::<Result<Vec<_>, _>>()?;
    let mut source_sections = source_ranges
        .par_iter()
        .map(|range| {
            encode_source_section(
                &artifact.source_map.entries[range.clone()],
                &function_indices,
                cancelled,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let section_bytes = metadata.len()
        + globals.len()
        + incremental.len()
        + project_data.len()
        + sources.len()
        + fingerprints.len()
        + manifest_section.len()
        + snapshot.len()
        + diagnostics.len()
        + function_sections.iter().map(Vec::len).sum::<usize>()
        + source_sections.iter().map(Vec::len).sum::<usize>();
    let identity = project_identity(manifest);
    let mut output = Vec::with_capacity(
        MAGIC.len() + 1 + 8 + 32 + 32 + 4 + 4 + section_bytes + std::mem::size_of::<Digest>(),
    );
    encode_project_file_header(
        &mut output,
        &identity,
        extensions,
        function_sections.len(),
        source_sections.len(),
    )?;
    output.append(&mut metadata);
    output.append(&mut globals);
    output.append(&mut incremental);
    output.append(&mut project_data);
    output.append(&mut sources);
    output.append(&mut fingerprints);
    output.append(&mut manifest_section);
    output.append(&mut snapshot);
    output.append(&mut diagnostics);
    for section in function_sections.iter_mut().chain(&mut source_sections) {
        output.append(section);
    }
    let digest = blake3::hash(&output);
    output.extend_from_slice(digest.as_bytes());
    Ok(output)
}

fn encode_project_file_header(
    output: &mut Vec<u8>,
    identity: &ProjectIdentity,
    extensions: &[ExtensionDeclaration],
    function_sections: usize,
    source_sections: usize,
) -> Result<(), String> {
    let source_digest: [u8; 32] = identity
        .source_digest
        .as_slice()
        .try_into()
        .map_err(|_| "project identity digest is not 32 bytes")?;
    output.extend_from_slice(MAGIC);
    output.push(VERSION);
    output.extend_from_slice(&identity.project_revision.to_le_bytes());
    output.extend_from_slice(&source_digest);
    output.extend_from_slice(&project_key(identity, extensions));
    output.extend_from_slice(
        &u32::try_from(function_sections)
            .map_err(|_| "compiled cache has too many function sections")?
            .to_le_bytes(),
    );
    output.extend_from_slice(
        &u32::try_from(source_sections)
            .map_err(|_| "compiled cache has too many source sections")?
            .to_le_bytes(),
    );
    Ok(())
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
    let incremental = IncrementalState::from_compact_cache_keys(
        artifact.artifact(),
        parts.incremental_cache_keys,
    )?;
    Ok(DecodedCompiledCache {
        key: sections.key,
        artifact,
        incremental,
        snapshot: parts.snapshot,
        diagnostics: parts.diagnostics,
    })
}

/// Decode the identity and exact frontend-submitted manifest embedded in a project file.
///
/// This source-only projection reuses the runtime cache parser but deliberately avoids
/// decoding or validating bytecode sections that an extraction tool does not consume.
///
/// # Errors
///
/// Returns an error when the cache is over the caller's limit, corrupt, unsupported, or
/// does not contain a decodable project snapshot.
pub fn decode_project_file(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<DecodedProjectFile, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    let manifest = decode_manifest_section(&sections.manifest, sections.identity.project_revision)
        .map_err(ProjectFileError::from)?;
    let actual_identity = project_identity(&manifest);
    if actual_identity != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    Ok(DecodedProjectFile {
        identity: sections.identity,
        manifest,
    })
}

fn parse_cache_sections(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<CompiledCacheSections<'_>, String> {
    if bytes.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    let minimum = MAGIC.len() + 1 + 8 + 32 + 32 + 4 + 4 + 9 * 16 + 32;
    if bytes.len() < minimum || &bytes[..MAGIC.len()] != MAGIC {
        return Err("project file has an invalid header".into());
    }
    let digest_offset = bytes.len() - 32;
    if blake3::hash(&bytes[..digest_offset]).as_bytes() != &bytes[digest_offset..] {
        return Err("compiled project cache digest mismatch".into());
    }
    let mut cursor = MAGIC.len();
    let version = *bytes
        .get(cursor)
        .ok_or("project file version is truncated")?;
    cursor += 1;
    if version != VERSION {
        return Err(format!("unsupported project file version {version:02x}"));
    }
    let project_revision = read_u64(bytes, &mut cursor)?;
    let source_digest = bytes
        .get(cursor..cursor + 32)
        .ok_or("project file source identity is truncated")?
        .to_vec();
    cursor += 32;
    let identity = ProjectIdentity {
        project_revision,
        source_digest: ProtocolBytes::new(source_digest),
    };
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
    let manifest = read_section(bytes, &mut cursor, digest_offset)?;
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
        &manifest,
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
        identity,
        key,
        metadata,
        globals,
        incremental,
        project_data,
        sources,
        fingerprints,
        manifest,
        snapshot,
        diagnostics,
        functions,
        source_entries,
    })
}

fn decode_cache_parts(sections: &CompiledCacheSections<'_>) -> Result<DecodedCacheParts, String> {
    let diagnostics = decode_section::<Vec<ProtocolDiagnostic>>(&sections.diagnostics)?;
    let manifest = decode_manifest_section(&sections.manifest, sections.identity.project_revision)?;
    if project_identity(&manifest) != sections.identity {
        return Err("project file identity does not match its embedded manifest".into());
    }
    let metadata = decode_section::<CompiledCacheMetadata>(&sections.metadata)?;
    let globals = decode_section::<Vec<BytecodeGlobal>>(&sections.globals)?;
    let incremental_cache_keys = decode_incremental_section(&sections.incremental)?;
    let project_data = decode_section::<erabasic_data::ProjectData>(&sections.project_data)?;
    let sources = decode_source_record_section(&sections.sources, &manifest)?;
    let fingerprints = decode_digest_section(&sections.fingerprints)?;
    let snapshot =
        decode_section::<CompiledSnapshotMetadata>(&sections.snapshot)?.into_snapshot(manifest)?;
    let function_chunks = decode_function_sections(&sections.functions)?;
    let mut functions = Vec::with_capacity(function_chunks.iter().map(Vec::len).sum());
    for mut chunk in function_chunks {
        functions.append(&mut chunk);
    }
    let source_chunks = decode_source_sections(&sections.source_entries, &functions)?;
    let mut entries = Vec::with_capacity(source_chunks.iter().map(Vec::len).sum());
    for mut chunk in source_chunks {
        entries.append(&mut chunk);
    }
    Ok(DecodedCacheParts {
        metadata,
        globals,
        incremental_cache_keys,
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
    functions: &[BytecodeFunction],
) -> Result<Vec<Vec<SourceMapEntry>>, String> {
    sections
        .par_iter()
        .map(|section| decode_source_section(section, functions))
        .collect()
}

fn encode_section<T: Serialize + ?Sized>(
    value: &T,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(cancelled, |writer| {
        rmp_serde::encode::write(writer, value).map_err(|error| error.to_string())
    })
}

fn encode_manifest_section(
    manifest: &ProjectManifest,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(cancelled, |writer| {
        writer
            .write_all(MANIFEST_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(manifest.files.len())
                .map_err(|_| "project manifest has too many files")?,
        )?;
        for file in &manifest.files {
            write_bytes(writer, file.relative_path.as_bytes())?;
            writer
                .write_all(&[file.category as u8])
                .map_err(|error| error.to_string())?;
            let hash_present = u8::from(file.content_hash.is_some());
            match &file.payload {
                FilePayload::Utf8(text) => {
                    validate_embedded_content_hash(file, text.as_bytes())?;
                    writer
                        .write_all(&[hash_present, 0])
                        .map_err(|error| error.to_string())?;
                    write_bytes(writer, text.as_bytes())?;
                }
                FilePayload::Bytes(bytes) => {
                    validate_embedded_content_hash(file, bytes.as_slice())?;
                    writer
                        .write_all(&[hash_present, 1])
                        .map_err(|error| error.to_string())?;
                    write_bytes(writer, bytes.as_slice())?;
                }
                FilePayload::IoError(_) => {
                    return Err("project files with I/O errors cannot be cached".into());
                }
            }
        }
        Ok(())
    })
}

fn validate_embedded_content_hash(file: &SubmittedFile, payload: &[u8]) -> Result<(), String> {
    if file
        .content_hash
        .as_ref()
        .is_some_and(|expected| expected.as_slice() != blake3::hash(payload).as_bytes())
    {
        return Err("project manifest content hash differs from its payload".into());
    }
    Ok(())
}

fn decode_manifest_section(
    section: &EncodedSectionRef<'_>,
    project_revision: u64,
) -> Result<ProjectManifest, String> {
    decode_raw_section(section, |reader| {
        expect_magic(reader, *MANIFEST_SECTION_MAGIC, "project manifest")?;
        let count = read_count(reader, section.decoded_length, "project manifest file")?;
        let mut files = Vec::new();
        files
            .try_reserve_exact(count)
            .map_err(|_| "project manifest allocation failed")?;
        for _ in 0..count {
            let relative_path = String::from_utf8(read_bytes(reader, section.decoded_length)?)
                .map_err(|_| "project manifest path is not UTF-8")?;
            let mut tags = [0_u8; 3];
            reader
                .read_exact(&mut tags)
                .map_err(|error| error.to_string())?;
            let category = decode_file_category(tags[0])?;
            let payload_bytes = read_bytes(reader, section.decoded_length)?;
            let payload = match tags[2] {
                0 => FilePayload::Utf8(
                    String::from_utf8(payload_bytes)
                        .map_err(|_| "project manifest text payload is not UTF-8")?,
                ),
                1 => FilePayload::Bytes(ProtocolBytes::new(payload_bytes)),
                _ => return Err("project manifest payload tag is invalid".into()),
            };
            let content_hash = match tags[1] {
                0 => None,
                1 => Some(ProtocolBytes::new(match &payload {
                    FilePayload::Utf8(text) => blake3::hash(text.as_bytes()).as_bytes().to_vec(),
                    FilePayload::Bytes(bytes) => blake3::hash(bytes.as_slice()).as_bytes().to_vec(),
                    FilePayload::IoError(_) => unreachable!(),
                })),
                _ => return Err("project manifest hash-presence tag is invalid".into()),
            };
            files.push(SubmittedFile {
                relative_path,
                category,
                payload,
                content_hash,
            });
        }
        Ok(ProjectManifest {
            project_revision,
            files,
        })
    })
}

fn encode_source_record_section(
    sources: &[SourceRecord],
    manifest: &ProjectManifest,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let manifest_indices = manifest
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            validate_relative_path(&file.relative_path)
                .map(|path| (path, index))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let indices = sources
        .iter()
        .map(|source| -> Result<usize, String> {
            let index = *manifest_indices
                .get(&source.relative_path)
                .ok_or_else(|| "bytecode source is missing from the project manifest".to_owned())?;
            let file = &manifest.files[index];
            if source_record_from_file(file)? != *source {
                return Err("bytecode source differs from the project manifest".into());
            }
            Ok(index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    encode_raw_section(cancelled, |writer| {
        writer
            .write_all(SOURCE_RECORD_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(indices.len()).map_err(|_| "too many bytecode sources")?,
        )?;
        for &index in &indices {
            write_varint(
                writer,
                u64::try_from(index).map_err(|_| "manifest file index is too large")?,
            )?;
        }
        Ok(())
    })
}

fn decode_source_record_section(
    section: &EncodedSectionRef<'_>,
    manifest: &ProjectManifest,
) -> Result<Vec<SourceRecord>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(reader, *SOURCE_RECORD_SECTION_MAGIC, "source record")?;
        let count = read_count(reader, section.decoded_length, "source record")?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(count)
            .map_err(|_| "source record allocation failed")?;
        for _ in 0..count {
            let index = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "manifest source index is not addressable")?;
            let file = manifest
                .files
                .get(index)
                .ok_or("manifest source index is out of range")?;
            sources.push(source_record_from_file(file)?);
        }
        Ok(sources)
    })
}

fn source_record_from_file(file: &SubmittedFile) -> Result<SourceRecord, String> {
    let FilePayload::Utf8(text) = &file.payload else {
        return Err("bytecode source does not refer to a text manifest file".into());
    };
    let mut line_starts =
        Vec::with_capacity(text.bytes().filter(|byte| *byte == b'\n').count() + 1);
    line_starts.push(0);
    line_starts.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .map(|(index, _)| u64::try_from(index + 1).unwrap_or(u64::MAX)),
    );
    Ok(SourceRecord {
        relative_path: validate_relative_path(&file.relative_path)
            .map_err(|error| error.to_string())?,
        content_hash: Digest(*blake3::hash(text.as_bytes()).as_bytes()),
        byte_len: u64::try_from(text.len()).map_err(|_| "source text is too large")?,
        line_starts,
    })
}

fn encode_incremental_section(
    cache_keys: &[Digest],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(cancelled, |writer| {
        writer
            .write_all(INCREMENTAL_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(cache_keys.len()).map_err(|_| "too many incremental cache keys")?,
        )?;
        for key in cache_keys {
            writer
                .write_all(&key.0)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

fn decode_incremental_section(section: &EncodedSectionRef<'_>) -> Result<Vec<Digest>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(reader, *INCREMENTAL_SECTION_MAGIC, "incremental cache")?;
        let count = read_count(reader, section.decoded_length / 32, "incremental cache key")?;
        let mut keys = Vec::new();
        keys.try_reserve_exact(count)
            .map_err(|_| "incremental cache allocation failed")?;
        for _ in 0..count {
            let mut key = [0_u8; 32];
            reader
                .read_exact(&mut key)
                .map_err(|error| error.to_string())?;
            keys.push(Digest(key));
        }
        Ok(keys)
    })
}

fn write_bytes(writer: &mut dyn std::io::Write, bytes: &[u8]) -> Result<(), String> {
    write_varint(
        writer,
        u64::try_from(bytes.len()).map_err(|_| "compiled cache byte string is too large")?,
    )?;
    writer.write_all(bytes).map_err(|error| error.to_string())
}

fn read_bytes(reader: &mut dyn std::io::Read, maximum: u64) -> Result<Vec<u8>, String> {
    let length = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| "compiled cache byte string is not addressable")?;
    if u64::try_from(length).unwrap_or(u64::MAX) > maximum {
        return Err("compiled cache byte string exceeds its section".into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "compiled cache byte string allocation failed")?;
    bytes.resize(length, 0);
    reader
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn read_count(reader: &mut dyn std::io::Read, maximum: u64, name: &str) -> Result<usize, String> {
    let count = usize::try_from(read_stream_varint(reader)?)
        .map_err(|_| format!("compiled cache {name} count is not addressable"))?;
    if u64::try_from(count).unwrap_or(u64::MAX) > maximum {
        return Err(format!("compiled cache {name} count is invalid"));
    }
    Ok(count)
}

fn expect_magic(
    reader: &mut dyn std::io::Read,
    expected: [u8; 4],
    name: &str,
) -> Result<(), String> {
    let mut actual = [0_u8; 4];
    reader
        .read_exact(&mut actual)
        .map_err(|error| error.to_string())?;
    if actual != expected {
        return Err(format!(
            "compiled cache {name} section has an invalid header"
        ));
    }
    Ok(())
}

fn decode_file_category(value: u8) -> Result<FileCategory, String> {
    match value {
        0 => Ok(FileCategory::Csv),
        1 => Ok(FileCategory::Erh),
        2 => Ok(FileCategory::Erb),
        3 => Ok(FileCategory::ResourceManifest),
        4 => Ok(FileCategory::Resource),
        5 => Ok(FileCategory::Configuration),
        _ => Err("project manifest file category is invalid".into()),
    }
}

fn encode_digest_section(
    digests: &[Digest],
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(cancelled, |writer| {
        writer
            .write_all(DIGEST_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u64::try_from(digests.len())
                    .map_err(|_| "compiled cache digest count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;
        for digest in digests {
            if digest.0[16..].iter().any(|byte| *byte != 0) {
                return Err(
                    "statement fingerprint exceeds the project-file 128-bit identity".into(),
                );
            }
            writer
                .write_all(&digest.0[..16])
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
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
                .checked_mul(16)
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
        let end = cursor + 16;
        let mut digest = [0_u8; 32];
        digest[..16].copy_from_slice(&decoded[cursor..end]);
        digests.push(Digest(digest));
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
#[allow(clippy::too_many_lines)]
fn encode_source_section(
    entries: &[SourceMapEntry],
    function_indices: &std::collections::BTreeMap<SymbolKey, usize>,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    encode_raw_section(cancelled, |writer| {
        writer
            .write_all(SOURCE_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u64::try_from(entries.len())
                    .map_err(|_| "compiled cache source entry count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                &u32::try_from(group_count)
                    .map_err(|_| "compiled cache source group count is too large")?
                    .to_le_bytes(),
            )
            .map_err(|error| error.to_string())?;

        let mut group_start = 0;
        while group_start < entries.len() {
            let function = entries[group_start].function;
            let group_length =
                entries[group_start..].partition_point(|entry| entry.function == function);
            write_varint(
                writer,
                u64::try_from(
                    *function_indices
                        .get(&function)
                        .ok_or("source-map function is missing from the artifact")?,
                )
                .map_err(|_| "source-map function index is too large")?,
            )?;
            writer
                .write_all(
                    &u32::try_from(group_length)
                        .map_err(|_| "compiled cache source group is too large")?
                        .to_le_bytes(),
                )
                .map_err(|error| error.to_string())?;
            let mut previous_code_end = 0_u64;
            let mut previous_byte_start = 0_u64;
            for entry in &entries[group_start..group_start + group_length] {
                write_varint(
                    writer,
                    entry
                        .code_start
                        .checked_sub(previous_code_end)
                        .ok_or("source-map code ranges are not ordered")?,
                )?;
                write_varint(
                    writer,
                    entry
                        .code_end
                        .checked_sub(entry.code_start)
                        .ok_or("source-map code range is reversed")?,
                )?;
                write_varint(
                    writer,
                    encode_signed_delta(entry.byte_start, previous_byte_start)?,
                )?;
                write_varint(
                    writer,
                    entry
                        .byte_end
                        .checked_sub(entry.byte_start)
                        .ok_or("source-map byte range is reversed")?,
                )?;
                write_varint(writer, u64::from(entry.statement_fingerprint))?;
                write_varint(writer, u64::from(entry.source_index))?;
                match entry.origin_chain.as_deref() {
                    None => write_varint(writer, 0)?,
                    Some(origins) => {
                        write_varint(
                            writer,
                            u64::try_from(origins.len())
                                .map_err(|_| "source-map origin chain is too long")?
                                .checked_add(1)
                                .ok_or("source-map origin chain is too long")?,
                        )?;
                        for &(source_index, byte_start, byte_end) in origins {
                            write_varint(writer, u64::from(source_index))?;
                            write_varint(writer, byte_start)?;
                            write_varint(
                                writer,
                                byte_end
                                    .checked_sub(byte_start)
                                    .ok_or("source-map origin byte range is reversed")?,
                            )?;
                        }
                    }
                }
                previous_code_end = entry.code_end;
                previous_byte_start = entry.byte_start;
            }
            group_start += group_length;
        }
        Ok(())
    })
}

fn decode_source_section(
    section: &EncodedSectionRef<'_>,
    functions: &[BytecodeFunction],
) -> Result<Vec<SourceMapEntry>, String> {
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
        let function_index = usize::try_from(read_varint(&decoded, &mut cursor)?)
            .map_err(|_| "compiled cache source function index is not addressable")?;
        let function = functions
            .get(function_index)
            .ok_or("compiled cache source function index is out of range")?
            .key;
        let group_length = usize::try_from(read_u32(&decoded, &mut cursor)?)
            .map_err(|_| "compiled cache source group length is not addressable")?;
        if group_length == 0 || group_length > entry_count.saturating_sub(entries.len()) {
            return Err("compiled cache source group length is invalid".into());
        }
        let mut previous_code_end = 0_u64;
        let mut previous_byte_start = 0_u64;
        for _ in 0..group_length {
            let code_start = previous_code_end
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code offset overflow")?;
            let code_end = code_start
                .checked_add(read_varint(&decoded, &mut cursor)?)
                .ok_or("compiled cache source code range overflow")?;
            let byte_start =
                decode_signed_delta(previous_byte_start, read_varint(&decoded, &mut cursor)?)?;
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
            previous_byte_start = byte_start;
        }
    }
    if entries.len() != entry_count || cursor != decoded.len() {
        return Err("compiled cache source section has trailing or missing entries".into());
    }
    Ok(entries)
}

fn encode_signed_delta(value: u64, previous: u64) -> Result<u64, String> {
    let delta = i128::from(value) - i128::from(previous);
    let delta = i64::try_from(delta).map_err(|_| "source-map byte delta is out of range")?;
    Ok((delta.cast_unsigned() << 1) ^ (delta >> 63).cast_unsigned())
}

fn decode_signed_delta(previous: u64, encoded: u64) -> Result<u64, String> {
    let magnitude = i128::from(encoded >> 1);
    let delta = if encoded & 1 == 0 {
        magnitude
    } else {
        -magnitude - 1
    };
    u64::try_from(i128::from(previous) + delta)
        .map_err(|_| "compiled cache source byte delta overflows".into())
}

mod io;
#[cfg(test)]
mod tests;

use self::io::{
    HashWriter, decode_raw_section, decode_section, encode_raw_section, equal_ranges, read_section,
    read_stream_varint, read_u32, read_u64, read_varint, weighted_function_ranges, write_varint,
};
