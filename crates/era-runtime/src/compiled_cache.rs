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
    ConfigurationJournal, apply_journal, configuration_digest, encode_record, parse_journal,
    replace_configuration,
};

const PROJECT_MAGIC: &[u8; 8] = b"RERAPROJ";
const CACHE_MAGIC: &[u8; 8] = b"RERACACH";
// Project files use a compact byte-sized base-format version. This is also a semantic epoch:
// increment it whenever compiler, analyzer or project-loading behavior can change an unchanged
// source's artifact. The checksummed configuration journal is a separately versioned trailing
// extension introduced with v4; changing its record semantics increments its own record version.
// Older readers reject the extension as trailing data instead of using it as an incremental seed.
const LEGACY_PROJECT_VERSION: u8 = 6;
const VERSION: u8 = 7;
const PROJECT_COMPRESSION_LEVEL: i32 = 3;
const CACHE_COMPRESSION_LEVEL: i32 = 1;
const TARGET_PARALLEL_SECTIONS: usize = 32;
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
        let (resource_graph, diagnostics) = ResourceGraph::from_manifest(&manifest);
        if let Some(diagnostic) = diagnostics.into_iter().find(|value| value.error) {
            return Err(format!(
                "embedded project resources cannot be rebuilt: {}",
                diagnostic.message
            ));
        }
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
            editable_configuration: self.editable_configuration,
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

#[cfg(any(target_arch = "wasm32", test))]
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
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::CompiledCache,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Ready(Some(cache_keys)),
            snapshot,
            diagnostics,
            cancelled,
            progress: None,
        })
    }

    fn new_for_kind(input: CooperativeEncoderInput) -> Self {
        Self {
            kind: input.kind,
            manifest: input.manifest,
            extensions: input.extensions,
            artifact: input.artifact,
            snapshot: input.snapshot,
            diagnostics: input.diagnostics,
            cancelled: input.cancelled,
            progress: input.progress,
            planner: Some(CacheLayoutPlanner::new(input.cache_keys)),
            plan: None,
            next_section: 0,
            manifest_encoder: None,
            pending_section: None,
            output: None,
            progress_completed: 0,
            progress_total: 1,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_with_incremental(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        incremental: Arc<IncrementalState>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::CompiledCache,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Incremental {
                state: incremental,
                keys: Vec::new(),
            },
            snapshot,
            diagnostics,
            cancelled,
            progress: None,
        })
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new_full_project(
        manifest: Arc<ProjectManifest>,
        extensions: Vec<ExtensionDeclaration>,
        artifact: ValidatedArtifact,
        incremental: Arc<IncrementalState>,
        snapshot: CompiledSnapshotMetadata,
        diagnostics: Vec<ProtocolDiagnostic>,
        cancelled: Option<Arc<AtomicBool>>,
        progress: Option<crate::ProjectProgressReporter>,
    ) -> Self {
        Self::new_for_kind(CooperativeEncoderInput {
            kind: ProjectContainerKind::FullProject,
            manifest,
            extensions,
            artifact,
            cache_keys: CacheKeyPlanner::Incremental {
                state: incremental,
                keys: Vec::new(),
            },
            snapshot,
            diagnostics,
            cancelled,
            progress,
        })
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
            self.report_cooperative_progress();
            return Ok(None);
        }
        if self.plan.is_none() {
            self.poll_layout()?;
            self.report_cooperative_progress();
            return Ok(None);
        }
        let plan = self.plan.as_ref().expect("cache layout was planned");
        if self.next_section < plan.section_count() {
            let Some(section) = self.encode_next_section()? else {
                self.report_cooperative_progress();
                return Ok(None);
            };
            self.pending_section = Some((section, 0));
            self.report_cooperative_progress();
            return Ok(None);
        }
        let (mut output, hasher) = self.output.take().expect("cache output was initialized");
        output.extend_from_slice(hasher.finalize().as_bytes());
        if let Some(reporter) = &self.progress {
            let completed = self.progress_completed.saturating_add(1);
            reporter.report(crate::ProjectProgress {
                stage: crate::ProjectProgressStage::Packaging,
                completed,
                total: completed,
            });
        }
        Ok(Some(output))
    }

    fn report_cooperative_progress(&mut self) {
        let Some(reporter) = &self.progress else {
            return;
        };
        self.progress_completed = self.progress_completed.saturating_add(1);
        self.progress_total = self
            .progress_total
            .max(self.progress_completed.saturating_add(1));
        reporter.report(crate::ProjectProgress {
            stage: crate::ProjectProgressStage::Packaging,
            completed: self.progress_completed,
            total: self.progress_total,
        });
    }

    fn encode_next_section(&mut self) -> Result<Option<Vec<u8>>, String> {
        let plan = self.plan.as_ref().expect("cache layout was planned");
        let function_start = 9;
        let source_start = function_start + plan.function_ranges.len();
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
                self.kind,
                cancelled,
            )?,
            1 => encode_section(&self.artifact.artifact().globals, self.kind, cancelled)?,
            2 => encode_incremental_section(&plan.cache_keys, self.kind, cancelled)?,
            3 => encode_section(&self.artifact.artifact().project_data, self.kind, cancelled)?,
            4 if self.kind == ProjectContainerKind::CompiledCache => {
                encode_compact_source_record_section(
                    &self.artifact.artifact().source_map.sources,
                    &self.manifest,
                    self.kind,
                    cancelled,
                )?
            }
            4 => encode_source_record_section(
                &self.artifact.artifact().source_map.sources,
                &self.manifest,
                self.kind,
                cancelled,
            )?,
            5 => encode_digest_section(
                &self.artifact.artifact().source_map.statement_fingerprints,
                self.kind,
                cancelled,
            )?,
            6 => {
                let encoder = self
                    .manifest_encoder
                    .get_or_insert(ManifestSectionEncoder::new(
                        self.manifest.files.len(),
                        self.kind,
                    )?);
                let Some(section) = encoder.step(&self.manifest)? else {
                    return Ok(None);
                };
                self.manifest_encoder = None;
                section
            }
            7 => encode_section(&self.snapshot, self.kind, cancelled)?,
            8 => encode_section(&self.diagnostics, self.kind, cancelled)?,
            index if index < source_start => {
                let range = plan.function_ranges[index - function_start].clone();
                encode_section(
                    &self.artifact.artifact().functions[range],
                    self.kind,
                    cancelled,
                )?
            }
            index => {
                let range = plan.source_ranges[index - source_start].clone();
                encode_source_section(
                    &self.artifact.artifact().source_map.entries[range],
                    &plan.function_indices,
                    self.kind,
                    cancelled,
                )?
            }
        };
        Ok(Some(section))
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
            self.kind,
            &plan.identity,
            &self.extensions,
            plan.function_ranges.len(),
            plan.source_ranges.len(),
        )?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(&output);
        self.output = Some((output, hasher));
        self.planner = None;
        self.progress_total = cooperative_work_estimate(&self.manifest, &self.artifact, &plan);
        self.plan = Some(plan);
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn cooperative_work_estimate(
    manifest: &ProjectManifest,
    artifact: &ValidatedArtifact,
    plan: &CacheLayoutPlan,
) -> u64 {
    let payload_quanta = manifest.files.iter().fold(0_usize, |total, file| {
        let bytes = match &file.payload {
            FilePayload::Utf8(value) => value.len(),
            FilePayload::Bytes(value) => value.as_slice().len(),
            FilePayload::IoError(error) => error.message.len(),
        };
        total.saturating_add(bytes.max(1).div_ceil(COOPERATIVE_MANIFEST_CHUNK_BYTES))
    });
    let planning_quanta = artifact
        .artifact()
        .functions
        .len()
        .saturating_add(artifact.artifact().source_map.entries.len())
        .div_ceil(COOPERATIVE_ITEM_QUANTUM);
    let estimate = plan
        .section_count()
        .saturating_mul(4)
        .saturating_add(manifest.files.len().saturating_mul(3))
        .saturating_add(payload_quanta.saturating_mul(3))
        .saturating_add(planning_quanta.saturating_mul(2))
        .saturating_add(32);
    u64::try_from(estimate).unwrap_or(u64::MAX)
}

struct ManifestSectionEncoder {
    writer:
        Option<self::io::CountingWriter<'static, zstd::stream::write::Encoder<'static, Vec<u8>>>>,
    file_index: usize,
    payload_offset: usize,
    payload_hasher: Option<blake3::Hasher>,
    kind: ProjectContainerKind,
}

impl ManifestSectionEncoder {
    fn new(file_count: usize, kind: ProjectContainerKind) -> Result<Self, String> {
        let encoder = zstd::stream::Encoder::new(Vec::new(), kind.compression_level())
            .map_err(|error| error.to_string())?;
        let mut writer = self::io::CountingWriter::new(encoder, None);
        writer
            .write_all(match kind {
                ProjectContainerKind::CompiledCache => COMPACT_MANIFEST_SECTION_MAGIC,
                ProjectContainerKind::FullProject => MANIFEST_SECTION_MAGIC,
            })
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
            kind,
        })
    }

    fn step(&mut self, manifest: &ProjectManifest) -> Result<Option<Vec<u8>>, String> {
        let Some(file) = manifest.files.get(self.file_index) else {
            return self.finish().map(Some);
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
            if self.kind == ProjectContainerKind::CompiledCache {
                let hash = file.content_hash.as_ref().map_or_else(
                    || blake3::hash(payload).as_bytes().to_vec(),
                    |value| value.as_slice().to_vec(),
                );
                if hash.len() != blake3::OUT_LEN {
                    return Err("project manifest content hash is not 32 bytes".into());
                }
                let omitted = !matches!(
                    file.category,
                    FileCategory::Configuration | FileCategory::ResourceManifest
                );
                writer
                    .write_all(&[
                        file.category as u8,
                        u8::from(matches!(&file.payload, FilePayload::Bytes(_))),
                        u8::from(omitted),
                    ])
                    .map_err(|error| error.to_string())?;
                writer.write_all(&hash).map_err(|error| error.to_string())?;
                if omitted {
                    self.file_index += 1;
                    return Ok(None);
                }
            } else {
                writer
                    .write_all(&[
                        file.category as u8,
                        u8::from(file.content_hash.is_some()),
                        u8::from(matches!(&file.payload, FilePayload::Bytes(_))),
                    ])
                    .map_err(|error| error.to_string())?;
            }
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

    fn finish(&mut self) -> Result<Vec<u8>, String> {
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
        Ok(output)
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

pub(crate) fn project_key(
    identity: &ProjectIdentity,
    extensions: &[ExtensionDeclaration],
) -> [u8; 32] {
    let mut writer = HashWriter::new("rustyera.compiled-project-key.v3");
    serde_json::to_writer(
        &mut writer,
        &(identity.source_digest.as_slice(), extensions),
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

pub(crate) fn validate_full_project_manifest(
    manifest: &ProjectManifest,
    expected_identity: &ProjectIdentity,
    sources: &[SourceRecord],
) -> Result<(), String> {
    if &project_identity(manifest) != expected_identity {
        return Err("project files changed after the active project was loaded".into());
    }
    let mut paths = BTreeSet::new();
    for file in &manifest.files {
        let path =
            validate_relative_path(&file.relative_path).map_err(|error| error.to_string())?;
        if !paths.insert(path.to_lowercase()) {
            return Err("full project manifest contains duplicate paths".into());
        }
        let payload = match &file.payload {
            FilePayload::Utf8(text) => text.as_bytes(),
            FilePayload::Bytes(bytes) => bytes.as_slice(),
            FilePayload::IoError(_) => {
                return Err("full project manifest contains an unreadable file".into());
            }
        };
        if file
            .content_hash
            .as_ref()
            .is_some_and(|expected| expected.as_slice() != blake3::hash(payload).as_bytes())
        {
            return Err("full project manifest content hash differs from its payload".into());
        }
    }
    let files = manifest
        .files
        .iter()
        .map(|file| {
            validate_relative_path(&file.relative_path)
                .map(|path| (path, file))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    for source in sources {
        let file = files
            .get(&source.relative_path)
            .ok_or("full project manifest is missing a compiled source")?;
        if source_record_from_file(file)? != *source {
            return Err("full project manifest source differs from the active artifact".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn encode_full_project_for_test(
    manifest: &ProjectManifest,
    extensions: &[ExtensionDeclaration],
    artifact: &ValidatedArtifact,
    incremental: &IncrementalState,
    snapshot: &NormalizedProjectSnapshot,
    diagnostics: &[ProtocolDiagnostic],
) -> Result<Vec<u8>, String> {
    let snapshot = CompiledSnapshotMetadata::from(snapshot);
    let cache_keys = incremental.compact_cache_keys(artifact.artifact())?;
    let mut encoder = CooperativeCompiledCacheEncoder::new_for_kind(CooperativeEncoderInput {
        kind: ProjectContainerKind::FullProject,
        manifest: Arc::new(manifest.clone()),
        extensions: extensions.to_vec(),
        artifact: artifact.clone(),
        cache_keys: CacheKeyPlanner::Ready(Some(cache_keys)),
        snapshot,
        diagnostics: diagnostics.to_vec(),
        cancelled: None,
        progress: None,
    });
    loop {
        if let Some(bytes) = encoder.step()? {
            return Ok(bytes);
        }
    }
}

#[cfg(test)]
pub(crate) fn encode_compiled_cache_for_test(
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
pub(crate) struct ProjectContainerControl {
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) progress: Option<crate::ProjectProgressReporter>,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeContainerInput {
    kind: ProjectContainerKind,
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    incremental: Arc<IncrementalState>,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    control: ProjectContainerControl,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeSectionPlan<'a> {
    kind: ProjectContainerKind,
    manifest: &'a ProjectManifest,
    bytecode: &'a BytecodeArtifact,
    snapshot: &'a CompiledSnapshotMetadata,
    diagnostics: &'a [ProtocolDiagnostic],
    cache_keys: &'a [Digest],
    function_indices: &'a std::collections::BTreeMap<SymbolKey, usize>,
    function_ranges: &'a [Range<usize>],
    source_ranges: &'a [Range<usize>],
    cancelled: &'a AtomicBool,
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
    encode_native_container(NativeContainerInput {
        kind: ProjectContainerKind::CompiledCache,
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        control: ProjectContainerControl {
            cancelled,
            progress: None,
        },
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn encode_full_project_cancellable(
    manifest: Arc<ProjectManifest>,
    extensions: Vec<ExtensionDeclaration>,
    artifact: ValidatedArtifact,
    incremental: Arc<IncrementalState>,
    snapshot: CompiledSnapshotMetadata,
    diagnostics: Vec<ProtocolDiagnostic>,
    control: ProjectContainerControl,
) -> Result<Vec<u8>, String> {
    encode_native_container(NativeContainerInput {
        kind: ProjectContainerKind::FullProject,
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        control,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_native_container(input: NativeContainerInput) -> Result<Vec<u8>, String> {
    let NativeContainerInput {
        kind,
        manifest,
        extensions,
        artifact,
        incremental,
        snapshot,
        diagnostics,
        control: ProjectContainerControl {
            cancelled,
            progress,
        },
    } = input;
    if cancelled.load(Ordering::Relaxed) {
        return Err("compiled cache build cancelled".into());
    }
    let bytecode = artifact.artifact();
    let cache_keys = incremental.compact_cache_keys(bytecode)?;
    let identity = project_identity(&manifest);
    let function_indices = bytecode
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.key, index))
        .collect::<std::collections::BTreeMap<_, _>>();
    let function_ranges = weighted_function_ranges(&bytecode.functions);
    let source_ranges = equal_ranges(bytecode.source_map.entries.len());
    let section_count = 9 + function_ranges.len() + source_ranges.len();
    let completed = AtomicU64::new(0);
    let plan = NativeSectionPlan {
        kind,
        manifest: &manifest,
        bytecode,
        snapshot: &snapshot,
        diagnostics: &diagnostics,
        cache_keys: &cache_keys,
        function_indices: &function_indices,
        function_ranges: &function_ranges,
        source_ranges: &source_ranges,
        cancelled: &cancelled,
    };
    let sections = (0..section_count)
        .into_par_iter()
        .map(|index| {
            let section = encode_native_section(index, &plan)?;
            let current = completed.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            if let Some(reporter) = &progress {
                reporter.report(crate::ProjectProgress {
                    stage: crate::ProjectProgressStage::Packaging,
                    completed: current,
                    total: u64::try_from(section_count).unwrap_or(u64::MAX),
                });
            }
            Ok(section)
        })
        .collect::<Result<Vec<_>, String>>()?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("compiled cache build cancelled".into());
    }
    let mut output = Vec::new();
    encode_project_file_header(
        &mut output,
        kind,
        &identity,
        &extensions,
        function_ranges.len(),
        source_ranges.len(),
    )?;
    for section in sections {
        output.extend_from_slice(&section);
    }
    output.extend_from_slice(blake3::hash(&output).as_bytes());
    Ok(output)
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_native_section(index: usize, plan: &NativeSectionPlan<'_>) -> Result<Vec<u8>, String> {
    if plan.cancelled.load(Ordering::Relaxed) {
        return Err("compiled cache build cancelled".to_owned());
    }
    let function_start = 9;
    let source_start = function_start + plan.function_ranges.len();
    let cancelled = Some(plan.cancelled);
    match index {
        0 => encode_section(
            &CompiledCacheMetadataRef {
                manifest: &plan.bytecode.manifest,
                call_compatibility: &plan.bytecode.call_compatibility,
                native_imports: &plan.bytecode.native_imports,
                host_imports: &plan.bytecode.host_imports,
                event_groups: &plan.bytecode.event_groups,
            },
            plan.kind,
            cancelled,
        ),
        1 => encode_section(&plan.bytecode.globals, plan.kind, cancelled),
        2 => encode_incremental_section(plan.cache_keys, plan.kind, cancelled),
        3 => encode_section(&plan.bytecode.project_data, plan.kind, cancelled),
        4 if plan.kind == ProjectContainerKind::CompiledCache => {
            encode_compact_source_record_section(
                &plan.bytecode.source_map.sources,
                plan.manifest,
                plan.kind,
                cancelled,
            )
        }
        4 => encode_source_record_section(
            &plan.bytecode.source_map.sources,
            plan.manifest,
            plan.kind,
            cancelled,
        ),
        5 => encode_digest_section(
            &plan.bytecode.source_map.statement_fingerprints,
            plan.kind,
            cancelled,
        ),
        6 => encode_manifest_section(plan.manifest, plan.kind, cancelled),
        7 => encode_section(plan.snapshot, plan.kind, cancelled),
        8 => encode_section(plan.diagnostics, plan.kind, cancelled),
        value if value < source_start => encode_section(
            &plan.bytecode.functions[plan.function_ranges[value - function_start].clone()],
            plan.kind,
            cancelled,
        ),
        value => encode_source_section(
            &plan.bytecode.source_map.entries[plan.source_ranges[value - source_start].clone()],
            plan.function_indices,
            plan.kind,
            cancelled,
        ),
    }
}

fn weighted_function_ranges(functions: &[BytecodeFunction]) -> Vec<Range<usize>> {
    if functions.is_empty() {
        return Vec::new();
    }
    let total = functions
        .iter()
        .map(|function| function.code.len().max(1))
        .sum::<usize>();
    let target = total.div_ceil(TARGET_PARALLEL_SECTIONS).max(1);
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut weight = 0_usize;
    for (index, function) in functions.iter().enumerate() {
        weight = weight.saturating_add(function.code.len().max(1));
        if weight >= target && ranges.len() + 1 < TARGET_PARALLEL_SECTIONS {
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

fn encode_manifest_section(
    manifest: &ProjectManifest,
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let mut encoder = ManifestSectionEncoder::new(manifest.files.len(), kind)?;
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err("compiled cache build cancelled".into());
        }
        if let Some(section) = encoder.step(manifest)? {
            return Ok(section);
        }
    }
}

fn encode_project_file_header(
    output: &mut Vec<u8>,
    kind: ProjectContainerKind,
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
    output.extend_from_slice(kind.magic());
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
    require_full_project(&sections)?;
    let mut manifest =
        decode_manifest_section(&sections.manifest, sections.identity.project_revision)
            .map_err(ProjectFileError::from)?;
    let actual_identity = project_identity(&manifest);
    if actual_identity != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    Ok(DecodedProjectFile {
        identity: project_identity(&manifest),
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
    require_full_project(&sections)?;
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
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    let identity = project_identity(&manifest);
    compact_frontend_manifest(&mut manifest, &diagnostics.map_err(ProjectFileError::from)?);
    Ok(DecodedProjectFile { identity, manifest })
}

/// Validate a project file and prepare one compact append-only configuration update.
///
/// The returned bytes contain only the journal record, not a regenerated project container.
/// Callers must truncate an interrupted trailing record to [`ProjectConfigurationUpdate::truncate_to`]
/// before appending. The embedded configuration is compared with `expected_digest` using
/// normalized LF line endings; an empty digest represents a missing `reraconfig.toml`.
///
/// # Errors
///
/// Returns an error when the project file or requested TOML is invalid, the transfer limit is
/// exceeded, or the optimistic-lock digest no longer matches and the requested contents have not
/// already been applied.
pub fn prepare_project_configuration_update(
    bytes: &[u8],
    maximum_bytes: usize,
    expected_digest: &[u8],
    contents: &str,
) -> Result<ProjectConfigurationUpdate, ProjectFileError> {
    let sections = parse_cache_sections(bytes, maximum_bytes).map_err(ProjectFileError::from)?;
    require_full_project(&sections)?;
    let mut manifest =
        decode_manifest_section(&sections.manifest, sections.identity.project_revision)
            .map_err(ProjectFileError::from)?;
    if project_identity(&manifest) != sections.identity {
        return Err(ProjectFileError::from(
            "project file identity does not match its embedded manifest".to_owned(),
        ));
    }
    apply_journal(&mut manifest, &sections.configuration_journal)
        .map_err(ProjectFileError::from)?;
    let current = configuration_digest(&manifest).map_err(ProjectFileError::from)?;
    let requested_source = era_config::normalize_line_endings(contents);
    let requested_digest = *blake3::hash(requested_source.as_bytes()).as_bytes();
    let expected_matches = match current {
        Some(digest) => expected_digest == digest.as_slice(),
        None => expected_digest.is_empty(),
    };
    if !expected_matches && current != Some(requested_digest) {
        return Err(ProjectFileError::from(
            "reraconfig.toml was modified by another process".to_owned(),
        ));
    }
    let (append, source_digest) =
        encode_record(current, contents).map_err(ProjectFileError::from)?;
    let append = if current == Some(source_digest) {
        Vec::new()
    } else {
        replace_configuration(&mut manifest, &requested_source, source_digest);
        append
    };
    if sections
        .configuration_journal
        .valid_end
        .checked_add(append.len())
        .is_none_or(|length| length > maximum_bytes)
    {
        return Err(ProjectFileError::from(
            "project configuration update exceeds the transfer limit".to_owned(),
        ));
    }
    Ok(ProjectConfigurationUpdate {
        truncate_to: u64::try_from(sections.configuration_journal.valid_end)
            .map_err(|_| ProjectFileError::from("project file is too large".to_owned()))?,
        append,
        identity: project_identity(&manifest),
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
    let ParsedContainerHeader {
        kind,
        version,
        identity,
        key,
        function_section_count,
        source_section_count,
        mut cursor,
    } = parse_container_header(bytes, maximum_bytes)?;
    let metadata = read_section(bytes, &mut cursor, bytes.len())?;
    let globals = read_section(bytes, &mut cursor, bytes.len())?;
    let incremental = read_section(bytes, &mut cursor, bytes.len())?;
    let project_data = read_section(bytes, &mut cursor, bytes.len())?;
    let sources = read_section(bytes, &mut cursor, bytes.len())?;
    let fingerprints = read_section(bytes, &mut cursor, bytes.len())?;
    let manifest = read_section(bytes, &mut cursor, bytes.len())?;
    let snapshot = read_section(bytes, &mut cursor, bytes.len())?;
    let diagnostics = read_section(bytes, &mut cursor, bytes.len())?;
    let functions = read_section_list(bytes, &mut cursor, function_section_count)?;
    let source_entries = read_section_list(bytes, &mut cursor, source_section_count)?;
    let journal_start = cursor
        .checked_add(32)
        .ok_or("compiled project cache digest offset overflows")?;
    let configuration_journal = parse_configuration_journal(bytes, cursor)?;
    if kind == ProjectContainerKind::CompiledCache && bytes.len() != journal_start {
        return Err("compiled project cache cannot contain a configuration journal".into());
    }
    let fixed_sections = [
        &metadata,
        &globals,
        &incremental,
        &project_data,
        &sources,
        &fingerprints,
        &manifest,
        &snapshot,
        &diagnostics,
    ];
    let decoded_bytes = fixed_sections
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
        kind,
        version,
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
        configuration_journal,
    })
}

struct ParsedContainerHeader {
    kind: ProjectContainerKind,
    version: u8,
    identity: ProjectIdentity,
    key: [u8; 32],
    function_section_count: usize,
    source_section_count: usize,
    cursor: usize,
}

fn parse_container_header(
    bytes: &[u8],
    maximum_bytes: usize,
) -> Result<ParsedContainerHeader, String> {
    if bytes.len() > maximum_bytes {
        return Err("compiled project cache exceeds the transfer limit".into());
    }
    let magic_length = PROJECT_MAGIC.len();
    let minimum = magic_length + 1 + 8 + 32 + 32 + 4 + 4 + 9 * 16 + 32;
    if bytes.len() < minimum {
        return Err("project file has an invalid header".into());
    }
    let kind = match &bytes[..magic_length] {
        magic if magic == PROJECT_MAGIC => ProjectContainerKind::FullProject,
        magic if magic == CACHE_MAGIC => ProjectContainerKind::CompiledCache,
        _ => return Err("project file has an invalid header".into()),
    };
    let mut cursor = magic_length;
    let version = *bytes
        .get(cursor)
        .ok_or("project file version is truncated")?;
    cursor += 1;
    if !matches!(
        (kind, version),
        (
            ProjectContainerKind::FullProject,
            LEGACY_PROJECT_VERSION | VERSION
        ) | (ProjectContainerKind::CompiledCache, VERSION)
    ) {
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
    Ok(ParsedContainerHeader {
        kind,
        version,
        identity,
        key,
        function_section_count,
        source_section_count,
        cursor,
    })
}

fn read_section_list<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    count: usize,
) -> Result<Vec<EncodedSectionRef<'a>>, String> {
    let mut sections = Vec::with_capacity(count);
    for _ in 0..count {
        sections.push(read_section(bytes, cursor, bytes.len())?);
    }
    Ok(sections)
}

fn require_full_project(sections: &CompiledCacheSections<'_>) -> Result<(), ProjectFileError> {
    if sections.kind != ProjectContainerKind::FullProject {
        return Err(ProjectFileError::from(
            "compiled project caches are not portable project files".to_owned(),
        ));
    }
    Ok(())
}

fn parse_configuration_journal(
    bytes: &[u8],
    digest_offset: usize,
) -> Result<ConfigurationJournal<'_>, String> {
    let digest_end = digest_offset
        .checked_add(32)
        .ok_or("compiled project cache digest offset overflows")?;
    let digest = bytes
        .get(digest_offset..digest_end)
        .ok_or("compiled project cache digest is truncated")?;
    if blake3::hash(&bytes[..digest_offset]).as_bytes() != digest {
        return Err("compiled project cache digest mismatch".into());
    }
    parse_journal(bytes, digest_end)
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
    let (diagnostics, mut manifest, metadata, globals) = primary?;
    if project_identity(&manifest) != sections.identity {
        return Err("project file identity does not match its embedded manifest".into());
    }
    apply_journal(&mut manifest, &sections.configuration_journal)?;
    let (incremental_cache_keys, project_data, fingerprints, function_chunks) = secondary?;
    let mut functions = Vec::with_capacity(function_chunks.iter().map(Vec::len).sum());
    for mut chunk in function_chunks {
        functions.append(&mut chunk);
    }
    let ((sources, snapshot), source_chunks) = rayon::join(
        || {
            rayon::join(
                || {
                    if sections.kind == ProjectContainerKind::CompiledCache
                        && sections.version == VERSION
                    {
                        decode_compact_source_record_section(&sections.sources, &manifest)
                    } else {
                        decode_source_record_section(&sections.sources, &manifest)
                    }
                },
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
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        rmp_serde::encode::write(writer, value).map_err(|error| error.to_string())
    })
}

fn decode_manifest_section(
    section: &EncodedSectionRef<'_>,
    project_revision: u64,
) -> Result<ProjectManifest, String> {
    decode_raw_section(section, |reader| {
        let mut magic = [0_u8; 4];
        reader
            .read_exact(&mut magic)
            .map_err(|error| error.to_string())?;
        let compact = match &magic {
            value if value == MANIFEST_SECTION_MAGIC => false,
            value if value == COMPACT_MANIFEST_SECTION_MAGIC => true,
            _ => return Err("project manifest has invalid magic".into()),
        };
        let count = read_count(reader, section.decoded_length, "project manifest file")?;
        let mut files = Vec::new();
        let mut paths = BTreeSet::new();
        files
            .try_reserve_exact(count)
            .map_err(|_| "project manifest allocation failed")?;
        for _ in 0..count {
            let relative_path = String::from_utf8(read_bytes(reader, section.decoded_length)?)
                .map_err(|_| "project manifest path is not UTF-8")?;
            let normalized_path =
                validate_relative_path(&relative_path).map_err(|error| error.to_string())?;
            if !paths.insert(normalized_path.to_lowercase()) {
                return Err("project manifest contains duplicate paths".into());
            }
            let mut tags = [0_u8; 3];
            reader
                .read_exact(&mut tags)
                .map_err(|error| error.to_string())?;
            let category = decode_file_category(tags[0])?;
            let (payload, content_hash) = if compact {
                decode_compact_manifest_payload(reader, section.decoded_length, category, tags)?
            } else {
                decode_full_manifest_payload(reader, section.decoded_length, tags)?
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

fn decode_compact_manifest_payload(
    reader: &mut dyn std::io::Read,
    decoded_length: u64,
    category: FileCategory,
    tags: [u8; 3],
) -> Result<(FilePayload, Option<ProtocolBytes>), String> {
    let mut hash = [0_u8; blake3::OUT_LEN];
    reader
        .read_exact(&mut hash)
        .map_err(|error| error.to_string())?;
    let omitted = match tags[2] {
        0 => false,
        1 => true,
        _ => return Err("project manifest omission tag is invalid".into()),
    };
    let expected_omitted = !matches!(
        category,
        FileCategory::Configuration | FileCategory::ResourceManifest
    );
    if omitted != expected_omitted {
        return Err("project cache manifest violates its payload omission policy".into());
    }
    let expected_bytes = category == FileCategory::Resource;
    if (tags[1] == 1) != expected_bytes || tags[1] > 1 {
        return Err("project cache manifest payload type disagrees with its category".into());
    }
    let payload_bytes = if omitted {
        Vec::new()
    } else {
        read_bytes(reader, decoded_length)?
    };
    let payload = decode_manifest_payload(payload_bytes, tags[1])?;
    if !omitted && manifest_payload_hash(&payload).as_bytes() != &hash {
        return Err("project cache manifest payload hash mismatch".into());
    }
    Ok((payload, Some(ProtocolBytes::new(hash.to_vec()))))
}

fn decode_full_manifest_payload(
    reader: &mut dyn std::io::Read,
    decoded_length: u64,
    tags: [u8; 3],
) -> Result<(FilePayload, Option<ProtocolBytes>), String> {
    let payload = decode_manifest_payload(read_bytes(reader, decoded_length)?, tags[2])?;
    let content_hash = match tags[1] {
        0 => None,
        1 => Some(ProtocolBytes::new(
            manifest_payload_hash(&payload).as_bytes().to_vec(),
        )),
        _ => return Err("project manifest hash-presence tag is invalid".into()),
    };
    Ok((payload, content_hash))
}

fn decode_manifest_payload(bytes: Vec<u8>, tag: u8) -> Result<FilePayload, String> {
    match tag {
        0 => String::from_utf8(bytes)
            .map(FilePayload::Utf8)
            .map_err(|_| "project manifest text payload is not UTF-8".into()),
        1 => Ok(FilePayload::Bytes(ProtocolBytes::new(bytes))),
        _ => Err("project manifest payload tag is invalid".into()),
    }
}

fn manifest_payload_hash(payload: &FilePayload) -> blake3::Hash {
    match payload {
        FilePayload::Utf8(text) => blake3::hash(text.as_bytes()),
        FilePayload::Bytes(bytes) => blake3::hash(bytes.as_slice()),
        FilePayload::IoError(_) => unreachable!(),
    }
}

fn encode_compact_source_record_section(
    sources: &[SourceRecord],
    manifest: &ProjectManifest,
    kind: ProjectContainerKind,
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
    let records = sources
        .iter()
        .map(|source| -> Result<(usize, &SourceRecord), String> {
            let index = *manifest_indices
                .get(&source.relative_path)
                .ok_or_else(|| "bytecode source is missing from the project manifest".to_owned())?;
            let file = &manifest.files[index];
            if source_record_from_file(file)? != *source {
                return Err("bytecode source differs from the project manifest".into());
            }
            Ok((index, source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
        writer
            .write_all(COMPACT_SOURCE_RECORD_SECTION_MAGIC)
            .map_err(|error| error.to_string())?;
        write_varint(
            writer,
            u64::try_from(records.len()).map_err(|_| "too many bytecode sources")?,
        )?;
        for &(index, source) in &records {
            write_varint(
                writer,
                u64::try_from(index).map_err(|_| "manifest file index is too large")?,
            )?;
            write_varint(writer, source.byte_len)?;
            write_varint(
                writer,
                u64::try_from(source.line_starts.len())
                    .map_err(|_| "source record has too many lines")?,
            )?;
            let mut previous = 0_u64;
            for &line_start in &source.line_starts {
                let delta = line_start
                    .checked_sub(previous)
                    .ok_or("source record line starts are not ordered")?;
                write_varint(writer, delta)?;
                previous = line_start;
            }
        }
        Ok(())
    })
}

fn decode_compact_source_record_section(
    section: &EncodedSectionRef<'_>,
    manifest: &ProjectManifest,
) -> Result<Vec<SourceRecord>, String> {
    decode_raw_section(section, |reader| {
        expect_magic(
            reader,
            *COMPACT_SOURCE_RECORD_SECTION_MAGIC,
            "compact source record",
        )?;
        let count = read_count(reader, section.decoded_length, "compact source record")?;
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(count)
            .map_err(|_| "compact source record allocation failed")?;
        for _ in 0..count {
            let index = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "manifest source index is not addressable")?;
            let file = manifest
                .files
                .get(index)
                .ok_or("manifest source index is out of range")?;
            if !matches!(
                file.category,
                FileCategory::Csv | FileCategory::Erh | FileCategory::Erb
            ) {
                return Err("compact source record refers to a non-source manifest file".into());
            }
            let hash: [u8; 32] = file
                .content_hash
                .as_ref()
                .ok_or("compact source record is missing its manifest hash")?
                .as_slice()
                .try_into()
                .map_err(|_| "compact source record manifest hash is not 32 bytes")?;
            let byte_len = read_stream_varint(reader)?;
            let line_count = usize::try_from(read_stream_varint(reader)?)
                .map_err(|_| "compact source line count is not addressable")?;
            if line_count == 0
                || u64::try_from(line_count).unwrap_or(u64::MAX) > byte_len.saturating_add(1)
            {
                return Err("compact source line count is invalid".into());
            }
            let mut line_starts = Vec::new();
            line_starts
                .try_reserve_exact(line_count)
                .map_err(|_| "compact source line allocation failed")?;
            let mut previous = 0_u64;
            for line_index in 0..line_count {
                let delta = read_stream_varint(reader)?;
                if (line_index == 0 && delta != 0) || (line_index != 0 && delta == 0) {
                    return Err("compact source line deltas are invalid".into());
                }
                previous = previous
                    .checked_add(delta)
                    .ok_or("compact source line delta overflows")?;
                if previous > byte_len {
                    return Err("compact source line start exceeds its byte length".into());
                }
                line_starts.push(previous);
            }
            sources.push(SourceRecord {
                relative_path: validate_relative_path(&file.relative_path)
                    .map_err(|error| error.to_string())?,
                content_hash: Digest(hash),
                byte_len,
                line_starts,
            });
        }
        Ok(sources)
    })
}

fn encode_source_record_section(
    sources: &[SourceRecord],
    manifest: &ProjectManifest,
    kind: ProjectContainerKind,
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
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
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
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
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
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
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
    kind: ProjectContainerKind,
    cancelled: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let group_count = entries
        .windows(2)
        .filter(|pair| pair[0].function != pair[1].function)
        .count()
        + usize::from(!entries.is_empty());
    encode_raw_section(kind.compression_level(), cancelled, |writer| {
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
