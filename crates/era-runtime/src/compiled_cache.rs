use std::collections::BTreeSet;
use std::fmt;
use std::io::Write as _;
use std::ops::Range;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicU64;
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

mod configuration_update;

use configuration_update::{
    ConfigurationJournal, StreamingConfigurationJournal, apply_journal, configuration_digest,
    encode_record, parse_journal, replace_configuration,
};

const PROJECT_MAGIC: &[u8; 8] = b"RERAPROJ";
const CACHE_MAGIC: &[u8; 8] = b"RERACACH";
// Cache identity is source based. A project revision is only a frontend/runtime-session epoch, so
// persisting it would make otherwise identical native and browser caches differ after no-op or
// differently scoped reload histories. Full project files keep their real revision because they
// are portable project snapshots rather than derived compiler caches.
const COMPILED_CACHE_PROJECT_REVISION: u64 = 0;
// Project files use a compact byte-sized base-format version. This is also a semantic epoch:
// increment it whenever compiler, analyzer or project-loading behavior can change an unchanged
// source's artifact. The checksummed configuration journal is a separately versioned trailing
// extension introduced with v4; changing its record semantics increments its own record version.
// Older readers reject the extension as trailing data instead of using it as an incremental seed.
const LEGACY_PROJECT_VERSION: u8 = 6;
const PREVIOUS_PROJECT_VERSION: u8 = 7;
const VERSION: u8 = 8;
const PROJECT_COMPRESSION_LEVEL: i32 = 3;
const CACHE_COMPRESSION_LEVEL: i32 = 1;
const TARGET_PARALLEL_SECTIONS: usize = 32;
const FIXED_SECTION_COUNT: usize = 9;
const MANIFEST_SECTION_INDEX: usize = 6;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SOURCE_SECTION_MAGIC: &[u8; 4] = b"RSM2";
const DIGEST_SECTION_MAGIC: &[u8; 4] = b"RDI2";
const MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF2";
const COMPACT_MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF3";
const SOURCE_RECORD_SECTION_MAGIC: &[u8; 4] = b"RSR2";
const COMPACT_SOURCE_RECORD_SECTION_MAGIC: &[u8; 4] = b"RSR3";
const INCREMENTAL_SECTION_MAGIC: &[u8; 4] = b"RIC2";
const COOPERATIVE_MANIFEST_CHUNK_BYTES: usize = 256 * 1024;
#[cfg(any(target_arch = "wasm32", test))]
const COOPERATIVE_ITEM_QUANTUM: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectContainerKind {
    CompiledCache,
    FullProject,
}

impl ProjectContainerKind {
    const fn magic(self) -> &'static [u8; 8] {
        match self {
            Self::CompiledCache => CACHE_MAGIC,
            Self::FullProject => PROJECT_MAGIC,
        }
    }

    const fn compression_level(self) -> i32 {
        match self {
            Self::CompiledCache => CACHE_COMPRESSION_LEVEL,
            Self::FullProject => PROJECT_COMPRESSION_LEVEL,
        }
    }
}

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
    kind: ProjectContainerKind,
    version: u8,
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
    configuration_journal: ConfigurationJournal<'a>,
}

#[derive(Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct CompiledSnapshotMetadata {
    project_identity: [u8; 32],
    resources: Vec<NormalizedResourceIdentity>,
    resource_graph: ResourceGraph,
    sort_with_filename: bool,
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
            resource_graph: snapshot.resource_graph.clone(),
            sort_with_filename: snapshot.sort_with_filename,
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
            // Client profiles only control how the same project configuration is presented and
            // hot-applied. Keep the persistent compiler cache host-neutral so TUI, browser and
            // Tauri sessions can share one deterministic artifact.
            configuration_profile: ConfigurationClientProfile::Reference,
            configuration: snapshot.configuration.clone(),
            editable_configuration: snapshot.editable_configuration.clone(),
            extensions: snapshot.extensions.clone(),
        }
    }
}

impl CompiledSnapshotMetadata {
    fn into_snapshot(self, manifest: ProjectManifest) -> Result<NormalizedProjectSnapshot, String> {
        let resource_graph = self.resource_graph;
        // Explicit-source markers are deliberately not serialized in the cache. Rebuild them
        // from the authoritative project files so preference precedence is identical on a hit.
        let editable_configuration = crate::project::project_configuration_values(&manifest.files);
        let configuration_document = manifest
            .files
            .iter()
            .find(|file| crate::project::is_root_configuration_file(file))
            .and_then(|file| match &file.payload {
                FilePayload::Utf8(contents) => Some(contents),
                _ => None,
            })
            .map_or_else(
                || Ok(era_config::ReraConfigDocument::empty()),
                |contents| {
                    era_config::ReraConfigDocument::parse(contents)
                        .map_err(|error| error.to_string())
                },
            )?;
        let client_configuration = self.configuration.clone();
        Ok(NormalizedProjectSnapshot {
            manifest: std::sync::Arc::new(manifest),
            project_identity: self.project_identity,
            resources: self.resources,
            resource_graph,
            sort_with_filename: self.sort_with_filename,
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
            client_configuration,
            editable_configuration,
            configuration_document,
            generated_configuration_source: None,
            extensions: self.extensions,
        })
    }
}

/// Incremental cache encoder used by single-threaded hosts such as WebAssembly workers.
///
/// The canonical layout is planned once, manifest payloads and final assembly are byte-quantized,
/// and every encoded section is appended and released before the next one starts. Hosts call one
/// step per event-loop turn so cache work begins immediately without monopolizing later input.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) struct CooperativeCompiledCacheEncoder {
    kind: ProjectContainerKind,
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    cancelled: Option<Arc<AtomicBool>>,
    progress: Option<crate::ProjectProgressReporter>,
    planner: Option<CacheLayoutPlanner>,
    plan: Option<CacheLayoutPlan>,
    next_section: usize,
    manifest_encoder: Option<ManifestSectionEncoder>,
    pending_section: Option<(Vec<u8>, usize)>,
    output: Option<(Vec<u8>, blake3::Hasher)>,
    progress_completed: u64,
    progress_total: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
struct CooperativeEncoderInput {
    kind: ProjectContainerKind,
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    cache_keys: CacheKeyPlanner,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    cancelled: Option<Arc<AtomicBool>>,
    progress: Option<crate::ProjectProgressReporter>,
}

#[cfg(any(target_arch = "wasm32", test))]
struct CacheLayoutPlan {
    identity: ProjectIdentity,
    cache_keys: Vec<Digest>,
    function_indices: std::collections::BTreeMap<SymbolKey, usize>,
    function_ranges: Vec<Range<usize>>,
    source_ranges: Vec<Range<usize>>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl CacheLayoutPlan {
    fn section_count(&self) -> usize {
        9 + self.function_ranges.len() + self.source_ranges.len()
    }
}

#[derive(Clone, Copy)]
#[cfg(any(target_arch = "wasm32", test))]
enum CacheLayoutPlanningStage {
    Identity,
    FunctionIndices,
    FunctionRanges,
}

#[cfg(any(target_arch = "wasm32", test))]
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

#[cfg(any(target_arch = "wasm32", test))]
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
#[cfg(any(target_arch = "wasm32", test))]
enum ProjectIdentityPlanningStage {
    Collect,
    Hash,
}

#[cfg(any(target_arch = "wasm32", test))]
type OrderedManifestFiles = std::collections::btree_map::IntoValues<(String, String, usize), usize>;

#[cfg(any(target_arch = "wasm32", test))]
struct PendingIdentityPayload {
    file_index: usize,
    offset: usize,
    hasher: blake3::Hasher,
    content_hash: bool,
}

#[cfg(any(target_arch = "wasm32", test))]
struct ProjectIdentityPlanner {
    stage: ProjectIdentityPlanningStage,
    cursor: usize,
    ordered: std::collections::BTreeMap<(String, String, usize), usize>,
    files: Option<OrderedManifestFiles>,
    hasher: blake3::Hasher,
    pending: Option<PendingIdentityPayload>,
}

#[cfg(any(target_arch = "wasm32", test))]
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

#[cfg(any(target_arch = "wasm32", test))]
fn identity_payload(file: &SubmittedFile) -> &[u8] {
    match &file.payload {
        FilePayload::Utf8(text) => text.as_bytes(),
        FilePayload::Bytes(bytes) => bytes.as_slice(),
        FilePayload::ExternalResource(_) => &[],
        FilePayload::IoError(error) => error.message.as_bytes(),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
enum CacheKeyPlanner {
    #[cfg(test)]
    Ready(Option<Vec<Digest>>),
    Incremental {
        state: Arc<IncrementalState>,
        keys: Vec<Digest>,
    },
}

#[cfg(any(target_arch = "wasm32", test))]
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

/// A compact append-only update for the `reraconfig.toml` embedded in a project file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectConfigurationUpdate {
    /// Byte offset at which an interrupted trailing update must be truncated before appending.
    pub truncate_to: u64,
    /// Complete journal record to append. Empty when the requested source is already current.
    pub append: Vec<u8>,
    /// Project identity after applying this update.
    pub identity: ProjectIdentity,
}

mod cooperative;
mod decode;
mod identity;
mod native;
mod sections;
mod stream;

use cooperative::ManifestSectionEncoder;
#[cfg(test)]
pub(crate) use decode::decode;
pub(crate) use decode::decode_with_progress;
pub use decode::{
    decode_project_file, decode_project_file_frontend_manifest,
    prepare_project_configuration_update,
};
#[cfg(test)]
pub(crate) use identity::{encode_compiled_cache_for_test, encode_full_project_for_test};
pub(crate) use identity::{project_identity, project_key, validate_full_project_manifest};
#[cfg(any(target_arch = "wasm32", test))]
use native::encode_project_file_header;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{
    ProjectContainerControl, encode_cancellable, encode_full_project_cancellable,
};
#[allow(clippy::wildcard_imports)]
use sections::*;
pub use stream::{DecodedProjectFileStream, ProjectFileStreamDecoder};

#[cfg(test)]
use decode::{
    CacheDecodeDelays, compact_frontend_manifest, decode_cache_parts,
    decode_cache_parts_with_delays, parse_cache_sections,
};
#[cfg(test)]
use native::encode_manifest_section;

mod io;
#[cfg(test)]
mod tests;

use self::io::{
    HashWriter, decode_raw_section, decode_section, encode_raw_section, equal_ranges, read_section,
    read_stream_varint, read_u32, read_u64, read_varint, write_varint,
};
