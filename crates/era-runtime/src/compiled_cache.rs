use std::collections::BTreeSet;
use std::fmt;
use std::io::Write as _;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    ConfigurationClientProfile, ExtensionDeclaration, FileCategory, FilePayload, ProjectIdentity,
    ProjectManifest, ProtocolDiagnostic, SubmittedFile, validate_relative_path,
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
const VERSION: u8 = 4;
const COMPRESSION_LEVEL: i32 = 3;
const TARGET_PARALLEL_SECTIONS: usize = 32;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SOURCE_SECTION_MAGIC: &[u8; 4] = b"RSM2";
const DIGEST_SECTION_MAGIC: &[u8; 4] = b"RDI2";
const MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF2";
const SOURCE_RECORD_SECTION_MAGIC: &[u8; 4] = b"RSR2";
const INCREMENTAL_SECTION_MAGIC: &[u8; 4] = b"RIC2";
const COOPERATIVE_MANIFEST_CHUNK_BYTES: usize = 256 * 1024;
const COOPERATIVE_ITEM_QUANTUM: usize = 256;

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
    configuration_profile: ConfigurationClientProfile,
    configuration: era_config::ConfigStore,
    editable_configuration: era_config::ConfigStore,
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
            configuration_profile: snapshot.configuration_profile,
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
            configuration_profile: self.configuration_profile,
            configuration: self.configuration,
            editable_configuration: self.editable_configuration,
            extensions: self.extensions,
        })
    }
}

/// Incremental cache encoder used by single-threaded hosts such as WebAssembly workers.
///
/// The canonical layout is planned once, manifest payloads and final assembly are byte-quantized,
/// and every encoded section is appended and released before the next one starts. Hosts call one
/// step per event-loop turn so cache work begins immediately without monopolizing later input.
pub(crate) struct CooperativeCompiledCacheEncoder {
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    cancelled: Option<Arc<AtomicBool>>,
    planner: Option<CacheLayoutPlanner>,
    plan: Option<CacheLayoutPlan>,
    next_section: usize,
    manifest_encoder: Option<ManifestSectionEncoder>,
    pending_section: Option<(Vec<u8>, usize)>,
    output: Option<(Vec<u8>, blake3::Hasher)>,
}

struct CacheLayoutPlan {
    identity: ProjectIdentity,
    cache_keys: Vec<Digest>,
    function_indices: std::collections::BTreeMap<SymbolKey, usize>,
    function_ranges: Vec<Range<usize>>,
    source_ranges: Vec<Range<usize>>,
}

impl CacheLayoutPlan {
    fn section_count(&self) -> usize {
        9 + self.function_ranges.len() + self.source_ranges.len()
    }
}

#[derive(Clone, Copy)]
enum CacheLayoutPlanningStage {
    Identity,
    FunctionIndices,
    FunctionRanges,
}

struct CacheLayoutPlanner {
    stage: CacheLayoutPlanningStage,
    identity_planner: ProjectIdentityPlanner,
    identity: Option<ProjectIdentity>,
    function_indices: std::collections::BTreeMap<SymbolKey, usize>,
    function_ranges: Vec<Range<usize>>,
    cursor: usize,
    total_weight: usize,
    target_weight: usize,
    range_start: usize,
    range_weight: usize,
    cache_keys: CacheKeyPlanner,
}

impl CacheLayoutPlanner {
    fn new(cache_keys: CacheKeyPlanner) -> Self {
        Self {
            stage: CacheLayoutPlanningStage::Identity,
            identity_planner: ProjectIdentityPlanner::new(),
            identity: None,
            function_indices: std::collections::BTreeMap::new(),
            function_ranges: Vec::new(),
            cursor: 0,
            total_weight: 0,
            target_weight: 0,
            range_start: 0,
            range_weight: 0,
            cache_keys,
        }
    }

    fn step(
        &mut self,
        manifest: &ProjectManifest,
        artifact: &ValidatedArtifact,
    ) -> Result<Option<CacheLayoutPlan>, String> {
        let functions = &artifact.artifact().functions;
        match self.stage {
            CacheLayoutPlanningStage::Identity => {
                let Some(identity) = self.identity_planner.step(manifest) else {
                    return Ok(None);
                };
                self.identity = Some(identity);
                self.cache_keys.validate(artifact.artifact())?;
                self.stage = CacheLayoutPlanningStage::FunctionIndices;
            }
            CacheLayoutPlanningStage::FunctionIndices => {
                let end = self
                    .cursor
                    .saturating_add(COOPERATIVE_ITEM_QUANTUM)
                    .min(functions.len());
                for (index, function) in functions.iter().enumerate().take(end).skip(self.cursor) {
                    self.function_indices.insert(function.key, index);
                    self.cache_keys.push(function)?;
                    self.total_weight =
                        self.total_weight.saturating_add(function.code.len().max(1));
                }
                self.cursor = end;
                if end == functions.len() {
                    self.target_weight =
                        self.total_weight.div_ceil(TARGET_PARALLEL_SECTIONS).max(1);
                    self.cursor = 0;
                    self.stage = CacheLayoutPlanningStage::FunctionRanges;
                }
            }
            CacheLayoutPlanningStage::FunctionRanges => {
                let end = self
                    .cursor
                    .saturating_add(COOPERATIVE_ITEM_QUANTUM)
                    .min(functions.len());
                for (index, function) in functions.iter().enumerate().take(end).skip(self.cursor) {
                    self.range_weight =
                        self.range_weight.saturating_add(function.code.len().max(1));
                    if self.range_weight >= self.target_weight
                        && self.function_ranges.len() + 1 < TARGET_PARALLEL_SECTIONS
                    {
                        self.function_ranges.push(self.range_start..index + 1);
                        self.range_start = index + 1;
                        self.range_weight = 0;
                    }
                }
                self.cursor = end;
                if end == functions.len() {
                    if self.range_start < functions.len() {
                        self.function_ranges.push(self.range_start..functions.len());
                    }
                    return Ok(Some(CacheLayoutPlan {
                        identity: self.identity.take().expect("cache identity was planned"),
                        cache_keys: self.cache_keys.finish(),
                        function_indices: std::mem::take(&mut self.function_indices),
                        function_ranges: std::mem::take(&mut self.function_ranges),
                        source_ranges: equal_ranges(artifact.artifact().source_map.entries.len()),
                    }));
                }
            }
        }
        Ok(None)
    }
}

#[derive(Clone, Copy)]
enum ProjectIdentityPlanningStage {
    Collect,
    Hash,
}

type OrderedManifestFiles = std::collections::btree_map::IntoValues<(String, String, usize), usize>;

struct PendingIdentityPayload {
    file_index: usize,
    offset: usize,
    hasher: blake3::Hasher,
    content_hash: bool,
}

struct ProjectIdentityPlanner {
    stage: ProjectIdentityPlanningStage,
    cursor: usize,
    ordered: std::collections::BTreeMap<(String, String, usize), usize>,
    files: Option<OrderedManifestFiles>,
    hasher: blake3::Hasher,
    pending: Option<PendingIdentityPayload>,
}

impl ProjectIdentityPlanner {
    fn new() -> Self {
        Self {
            stage: ProjectIdentityPlanningStage::Collect,
            cursor: 0,
            ordered: std::collections::BTreeMap::new(),
            files: None,
            hasher: blake3::Hasher::new_derive_key("rustyera.project-source-identity.v1"),
            pending: None,
        }
    }

    fn step(&mut self, manifest: &ProjectManifest) -> Option<ProjectIdentity> {
        if let Some(pending) = self.pending.as_mut() {
            let file = &manifest.files[pending.file_index];
            let bytes = if pending.content_hash {
                file.content_hash
                    .as_ref()
                    .expect("pending content hash exists")
                    .as_slice()
            } else {
                identity_payload(file)
            };
            let end = pending
                .offset
                .saturating_add(COOPERATIVE_MANIFEST_CHUNK_BYTES)
                .min(bytes.len());
            pending.hasher.update(&bytes[pending.offset..end]);
            pending.offset = end;
            if end == bytes.len() {
                self.hasher.update(pending.hasher.finalize().as_bytes());
                self.pending = None;
            }
            return None;
        }
        match self.stage {
            ProjectIdentityPlanningStage::Collect => {
                let end = self
                    .cursor
                    .saturating_add(COOPERATIVE_ITEM_QUANTUM)
                    .min(manifest.files.len());
                for (index, file) in manifest
                    .files
                    .iter()
                    .enumerate()
                    .take(end)
                    .skip(self.cursor)
                {
                    self.ordered.insert(
                        (
                            file.relative_path.to_lowercase(),
                            file.relative_path.clone(),
                            index,
                        ),
                        index,
                    );
                }
                self.cursor = end;
                if end == manifest.files.len() {
                    self.files = Some(std::mem::take(&mut self.ordered).into_values());
                    self.stage = ProjectIdentityPlanningStage::Hash;
                }
            }
            ProjectIdentityPlanningStage::Hash => {
                for _ in 0..COOPERATIVE_ITEM_QUANTUM {
                    let Some(file_index) = self
                        .files
                        .as_mut()
                        .expect("ordered manifest files exist")
                        .next()
                    else {
                        return Some(ProjectIdentity {
                            project_revision: manifest.project_revision,
                            source_digest: ProtocolBytes::new(
                                self.hasher.finalize().as_bytes().to_vec(),
                            ),
                        });
                    };
                    let file = &manifest.files[file_index];
                    let path = file.relative_path.as_bytes();
                    self.hasher.update(&(path.len() as u64).to_le_bytes());
                    self.hasher.update(path);
                    self.hasher.update(&[file.category as u8]);
                    if let Some(content_hash) = &file.content_hash
                        && content_hash.as_slice().len() == blake3::OUT_LEN
                    {
                        self.hasher.update(content_hash.as_slice());
                        continue;
                    }
                    self.pending = Some(PendingIdentityPayload {
                        file_index,
                        offset: 0,
                        hasher: blake3::Hasher::new(),
                        content_hash: file.content_hash.is_some(),
                    });
                    return None;
                }
            }
        }
        None
    }
}

fn identity_payload(file: &SubmittedFile) -> &[u8] {
    match &file.payload {
        FilePayload::Utf8(text) => text.as_bytes(),
        FilePayload::Bytes(bytes) => bytes.as_slice(),
        FilePayload::IoError(error) => error.message.as_bytes(),
    }
}

enum CacheKeyPlanner {
    #[cfg(test)]
    Ready(Option<Vec<Digest>>),
    Incremental {
        state: Arc<IncrementalState>,
        keys: Vec<Digest>,
    },
}

impl CacheKeyPlanner {
    fn validate(&self, artifact: &BytecodeArtifact) -> Result<(), String> {
        match self {
            #[cfg(test)]
            Self::Ready(_) => Ok(()),
            Self::Incremental { state, .. } => state.validate_compact_cache_artifact(artifact),
        }
    }

    fn push(&mut self, function: &BytecodeFunction) -> Result<(), String> {
        match self {
            #[cfg(test)]
            Self::Ready(_) => {}
            Self::Incremental { state, keys } => {
                keys.push(state.compact_cache_key(function)?);
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Vec<Digest> {
        match self {
            #[cfg(test)]
            Self::Ready(keys) => keys.take().expect("cache keys were planned"),
            Self::Incremental { keys, .. } => std::mem::take(keys),
        }
    }
}

impl CooperativeCompiledCacheEncoder {
    #[cfg(test)]
    pub(crate) fn new(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        cache_keys: Vec<Digest>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            manifest,
            extensions,
            artifact,
            snapshot,
            diagnostics,
            cancelled,
            planner: Some(CacheLayoutPlanner::new(CacheKeyPlanner::Ready(Some(
                cache_keys,
            )))),
            plan: None,
            next_section: 0,
            manifest_encoder: None,
            pending_section: None,
            output: None,
        }
    }

    pub(crate) fn new_with_incremental(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        incremental: Arc<IncrementalState>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            manifest,
            extensions,
            artifact,
            snapshot,
            diagnostics,
            cancelled,
            planner: Some(CacheLayoutPlanner::new(CacheKeyPlanner::Incremental {
                state: incremental,
                keys: Vec::new(),
            })),
            plan: None,
            next_section: 0,
            manifest_encoder: None,
            pending_section: None,
            output: None,
        }
    }

    pub(crate) fn step(&mut self) -> Result<Option<Vec<u8>>, String> {
        if self
            .cancelled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err("compiled cache build cancelled".into());
        }
        if let Some((section, offset)) = self.pending_section.as_mut() {
            let end = offset
                .saturating_add(COOPERATIVE_MANIFEST_CHUNK_BYTES)
                .min(section.len());
            let chunk = &section[*offset..end];
            let (output, hasher) = self.output.as_mut().expect("cache output was initialized");
            output.extend_from_slice(chunk);
            hasher.update(chunk);
            *offset = end;
            if end == section.len() {
                self.pending_section = None;
                self.next_section += 1;
            }
            return Ok(None);
        }
        if self.plan.is_none() {
            self.poll_layout()?;
            return Ok(None);
        }
        let plan = self.plan.as_ref().expect("cache layout was planned");
        let fixed_sections = 9;
        let function_start = fixed_sections;
        let source_start = function_start + plan.function_ranges.len();
        if self.next_section < plan.section_count() {
            let cancelled = self.cancelled.as_deref();
            let section = match self.next_section {
                0 => encode_section(
                    &CompiledCacheMetadataRef {
                        manifest: &self.artifact.artifact().manifest,
                        call_compatibility: &self.artifact.artifact().call_compatibility,
                        native_imports: &self.artifact.artifact().native_imports,
                        host_imports: &self.artifact.artifact().host_imports,
                        event_groups: &self.artifact.artifact().event_groups,
                    },
                    cancelled,
                )?,
                1 => encode_section(&self.artifact.artifact().globals, cancelled)?,
                2 => encode_incremental_section(&plan.cache_keys, cancelled)?,
                3 => encode_section(&self.artifact.artifact().project_data, cancelled)?,
                4 => encode_source_record_section(
                    &self.artifact.artifact().source_map.sources,
                    &self.manifest,
                    cancelled,
                )?,
                5 => encode_digest_section(
                    &self.artifact.artifact().source_map.statement_fingerprints,
                    cancelled,
                )?,
                6 => {
                    let encoder = self
                        .manifest_encoder
                        .get_or_insert(ManifestSectionEncoder::new(self.manifest.files.len())?);
                    let Some(section) = encoder.step(&self.manifest)? else {
                        return Ok(None);
                    };
                    self.manifest_encoder = None;
                    section
                }
                7 => encode_section(&self.snapshot, cancelled)?,
                8 => encode_section(&self.diagnostics, cancelled)?,
                index if index < source_start => {
                    let range = plan.function_ranges[index - function_start].clone();
                    encode_section(&self.artifact.artifact().functions[range], cancelled)?
                }
                index => {
                    let range = plan.source_ranges[index - source_start].clone();
                    encode_source_section(
                        &self.artifact.artifact().source_map.entries[range],
                        &plan.function_indices,
                        cancelled,
                    )?
                }
            };
            self.pending_section = Some((section, 0));
            return Ok(None);
        }
        let (mut output, hasher) = self.output.take().expect("cache output was initialized");
        output.extend_from_slice(hasher.finalize().as_bytes());
        Ok(Some(output))
    }

    fn poll_layout(&mut self) -> Result<(), String> {
        let Some(plan) = self
            .planner
            .as_mut()
            .expect("cache layout planner exists")
            .step(&self.manifest, &self.artifact)?
        else {
            return Ok(());
        };
        let mut output = Vec::new();
        encode_project_file_header(
            &mut output,
            &plan.identity,
            &self.extensions,
            self.snapshot.configuration_profile,
            plan.function_ranges.len(),
            plan.source_ranges.len(),
        )?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(&output);
        self.output = Some((output, hasher));
        self.planner = None;
        self.plan = Some(plan);
        Ok(())
    }
}

struct ManifestSectionEncoder {
    writer:
        Option<self::io::CountingWriter<'static, zstd::stream::write::Encoder<'static, Vec<u8>>>>,
    file_index: usize,
    payload_offset: usize,
    payload_hasher: Option<blake3::Hasher>,
}

impl ManifestSectionEncoder {
    fn new(file_count: usize) -> Result<Self, String> {
        let encoder = zstd::stream::Encoder::new(Vec::new(), COMPRESSION_LEVEL)
            .map_err(|error| error.to_string())?;
        let mut writer = self::io::CountingWriter::new(encoder, None);
        writer
            .write_all(MANIFEST_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            &mut writer,
            u64::try_from(file_count).map_err(|_| "project manifest has too many files")?,
        )?;
        Ok(Self {
            writer: Some(writer),
            file_index: 0,
            payload_offset: 0,
            payload_hasher: None,
        })
    }

    fn step(&mut self, manifest: &ProjectManifest) -> Result<Option<Vec<u8>>, String> {
        let Some(file) = manifest.files.get(self.file_index) else {
            let writer = self
                .writer
                .take()
                .expect("manifest encoder retains its writer");
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
            return Ok(Some(output));
        };
        let payload = match &file.payload {
            FilePayload::Utf8(text) => text.as_bytes(),
            FilePayload::Bytes(bytes) => bytes.as_slice(),
            FilePayload::IoError(_) => {
                return Err("project files with I/O errors cannot be cached".into());
            }
        };
        let writer = self
            .writer
            .as_mut()
            .expect("manifest encoder retains its writer");
        if self.payload_hasher.is_none() {
            write_bytes(writer, file.relative_path.as_bytes())?;
            writer
                .write_all(&[
                    file.category as u8,
                    u8::from(file.content_hash.is_some()),
                    u8::from(matches!(&file.payload, FilePayload::Bytes(_))),
                ])
                .map_err(|error| error.to_string())?;
            write_varint(
                writer,
                u64::try_from(payload.len())
                    .map_err(|_| "compiled cache byte string is too large")?,
            )?;
            self.payload_hasher = Some(blake3::Hasher::new());
            return Ok(None);
        }
        let end = self
            .payload_offset
            .saturating_add(COOPERATIVE_MANIFEST_CHUNK_BYTES)
            .min(payload.len());
        let chunk = &payload[self.payload_offset..end];
        writer.write_all(chunk).map_err(|error| error.to_string())?;
        self.payload_hasher
            .as_mut()
            .expect("payload hasher was initialized")
            .update(chunk);
        self.payload_offset = end;
        if end == payload.len() {
            let actual = self
                .payload_hasher
                .take()
                .expect("payload hasher was initialized")
                .finalize();
            if file
                .content_hash
                .as_ref()
                .is_some_and(|expected| expected.as_slice() != actual.as_bytes())
            {
                return Err("project manifest content hash differs from its payload".into());
            }
            self.file_index += 1;
            self.payload_offset = 0;
        }
        Ok(None)
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
    configuration_profile: ConfigurationClientProfile,
) -> [u8; 32] {
    let mut writer = HashWriter::new("rustyera.compiled-project-key.v2");
    serde_json::to_writer(
        &mut writer,
        &(
            identity.source_digest.as_slice(),
            extensions,
            configuration_profile,
        ),
    )
    .expect("project cache identity values are serializable");
    writer.finish()
}

pub(crate) fn project_identity(manifest: &ProjectManifest) -> ProjectIdentity {
    let mut files = manifest
        .files
        .iter()
        .map(|file| {
            (
                file.relative_path.to_lowercase(),
                file.relative_path.as_str(),
                file,
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));
    let mut hasher = blake3::Hasher::new_derive_key("rustyera.project-source-identity.v1");
    for (_, _, file) in files {
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
    let mut encoder = CooperativeCompiledCacheEncoder::new(
        Arc::new(manifest.clone()),
        extensions.to_vec(),
        artifact.clone(),
        cache_keys,
        snapshot,
        diagnostics.to_vec(),
        None,
    );
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn encode_cancellable(
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    incremental: Arc<IncrementalState>,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    cancelled: Arc<AtomicBool>,
) -> Result<Vec<u8>, String> {
    let mut encoder = CooperativeCompiledCacheEncoder::new_with_incremental(
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        Some(cancelled),
    );
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes);
        }
    }
}

fn encode_project_file_header(
    output: &mut Vec<u8>,
    identity: &ProjectIdentity,
    extensions: &[ExtensionDeclaration],
    configuration_profile: ConfigurationClientProfile,
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
    output.extend_from_slice(&project_key(identity, extensions, configuration_profile));
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

/// Decode a compact project-file manifest for frontend-owned resource and diagnostic I/O.
///
/// Non-resource payloads that are not referenced by a cached diagnostic are cleared. Their
/// original content hashes remain available for identity validation, while the full cache import
/// remains authoritative for runtime loading.
///
/// # Errors
///
/// Returns an error under the same conditions as [`decode_project_file`].
pub fn decode_project_file_frontend_manifest(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<DecodedProjectFile, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    let (manifest, diagnostics) = rayon::join(
        || decode_manifest_section(&sections.manifest, sections.identity.project_revision),
        || decode_section::<Vec<ProtocolDiagnostic>>(&sections.diagnostics),
    );
    let mut manifest = manifest.map_err(ProjectFileError::from)?;
    if project_identity(&manifest) != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    compact_frontend_manifest(&mut manifest, &diagnostics.map_err(ProjectFileError::from)?);
    Ok(DecodedProjectFile {
        identity: sections.identity,
        manifest,
    })
}

fn compact_frontend_manifest(manifest: &mut ProjectManifest, diagnostics: &[ProtocolDiagnostic]) {
    let diagnostic_sources = diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.source.as_ref())
        .map(|source| source.relative_path.to_lowercase())
        .collect::<BTreeSet<_>>();
    for file in &mut manifest.files {
        if file.content_hash.is_none() {
            let payload = match &file.payload {
                FilePayload::Utf8(text) => text.as_bytes(),
                FilePayload::Bytes(bytes) => bytes.as_slice(),
                FilePayload::IoError(_) => continue,
            };
            file.content_hash = Some(ProtocolBytes::new(
                blake3::hash(payload).as_bytes().to_vec(),
            ));
        }
        if file.category == FileCategory::Resource
            || diagnostic_sources.contains(&file.relative_path.to_lowercase())
        {
            continue;
        }
        match &mut file.payload {
            FilePayload::Utf8(text) => text.clear(),
            FilePayload::Bytes(bytes) => *bytes = ProtocolBytes::new(Vec::new()),
            FilePayload::IoError(error) => error.message.clear(),
        }
    }
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
    let (primary, secondary) = rayon::join(
        || {
            let (diagnostics, manifest) = rayon::join(
                || decode_section::<Vec<ProtocolDiagnostic>>(&sections.diagnostics),
                || decode_manifest_section(&sections.manifest, sections.identity.project_revision),
            );
            let (metadata, globals) = rayon::join(
                || decode_section::<CompiledCacheMetadata>(&sections.metadata),
                || decode_section::<Vec<BytecodeGlobal>>(&sections.globals),
            );
            Ok::<_, String>((diagnostics?, manifest?, metadata?, globals?))
        },
        || {
            let ((incremental_cache_keys, project_data), (fingerprints, function_chunks)) =
                rayon::join(
                    || {
                        rayon::join(
                            || decode_incremental_section(&sections.incremental),
                            || decode_section::<erabasic_data::ProjectData>(&sections.project_data),
                        )
                    },
                    || {
                        rayon::join(
                            || decode_digest_section(&sections.fingerprints),
                            || decode_function_sections(&sections.functions),
                        )
                    },
                );
            Ok::<_, String>((
                incremental_cache_keys?,
                project_data?,
                fingerprints?,
                function_chunks?,
            ))
        },
    );
    let (diagnostics, manifest, metadata, globals) = primary?;
    if project_identity(&manifest) != sections.identity {
        return Err("project file identity does not match its embedded manifest".into());
    }
    let (incremental_cache_keys, project_data, fingerprints, function_chunks) = secondary?;
    let mut functions = Vec::with_capacity(function_chunks.iter().map(Vec::len).sum());
    for mut chunk in function_chunks {
        functions.append(&mut chunk);
    }
    let ((sources, snapshot), source_chunks) = rayon::join(
        || {
            rayon::join(
                || decode_source_record_section(&sections.sources, &manifest),
                || decode_section::<CompiledSnapshotMetadata>(&sections.snapshot),
            )
        },
        || decode_source_sections(&sections.source_entries, &functions),
    );
    let sources = sources?;
    let snapshot = snapshot?.into_snapshot(manifest)?;
    let source_chunks = source_chunks?;
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
    let content_hash = file.content_hash.as_ref().map_or_else(
        || *blake3::hash(text.as_bytes()).as_bytes(),
        |hash| {
            hash.as_slice()
                .try_into()
                .unwrap_or_else(|_| *blake3::hash(text.as_bytes()).as_bytes())
        },
    );
    Ok(SourceRecord {
        relative_path: validate_relative_path(&file.relative_path)
            .map_err(|error| error.to_string())?,
        content_hash: Digest(content_hash),
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
    read_stream_varint, read_u32, read_u64, read_varint, write_varint,
};
