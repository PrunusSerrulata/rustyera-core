pub(crate) fn encode_project_configuration_journal_record(
    previous_digest: [u8; 32],
    source: &str,
) -> Result<Vec<u8>, String> {
    encode_record(Some(previous_digest), source).map(|(record, _)| record)
}

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
const PROFILELESS_PROJECT_VERSION: u8 = 8;
const PROFILED_PROJECT_VERSION: u8 = 9;
const ARITHMETIC_PROJECT_VERSION: u8 = 10;
const CALL_PROJECT_VERSION: u8 = 11;
/// Last source-extractable project container before Batch 2 data APIs changed bytecode.
/// Compiled caches at this version are deliberately rebuilt instead of decoded.
const DATA_PROJECT_VERSION: u8 = 12;
const VERSION: u8 = 14;
const PROJECT_COMPRESSION_LEVEL: i32 = 3;
const CACHE_COMPRESSION_LEVEL: i32 = 1;
// This is also the cooperative WASM encoder's largest function/source partition count. Snake TW
// has enough bytecode for a 32-way function section to monopolize the Runtime Worker beyond the
// frontend watchdog interval. Keep the canonical native/WASM layout identical while bounding each
// cooperative serialization slice more tightly.
const TARGET_PARALLEL_SECTIONS: usize = 256;
const FIXED_SECTION_COUNT: usize = 9;
const MANIFEST_SECTION_INDEX: usize = 6;
const MAXIMUM_DECODED_PAYLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SOURCE_SECTION_MAGIC: &[u8; 4] = b"RSM2";
const DIGEST_SECTION_MAGIC: &[u8; 4] = b"RDI2";
const LEGACY_MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF2";
const LEGACY_COMPACT_MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF3";
const MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF4";
const COMPACT_MANIFEST_SECTION_MAGIC: &[u8; 4] = b"RMF5";
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
    runtime_builtins: &'a [erabasic_bytecode::RuntimeBuiltinSymbol],
    runtime_variables: &'a [erabasic_bytecode::RuntimeVariableSymbol],
    runtime_native_authorizations: &'a [erabasic_bytecode::RuntimeNativeAuthorization],
    runtime_host_authorizations: &'a [erabasic_bytecode::RuntimeHostAuthorization],
    runtime_staged_authorizations: &'a [erabasic_bytecode::RuntimeStagedAuthorization],
    native_imports: &'a [NativeImport],
    host_imports: &'a [HostImport],
    event_groups: &'a [BytecodeEventGroup],
}

#[derive(Deserialize)]
struct CompiledCacheMetadata {
    manifest: ArtifactManifest,
    call_compatibility: BytecodeCallCompatibility,
    runtime_builtins: Vec<erabasic_bytecode::RuntimeBuiltinSymbol>,
    runtime_variables: Vec<erabasic_bytecode::RuntimeVariableSymbol>,
    runtime_native_authorizations: Vec<erabasic_bytecode::RuntimeNativeAuthorization>,
    runtime_host_authorizations: Vec<erabasic_bytecode::RuntimeHostAuthorization>,
    runtime_staged_authorizations: Vec<erabasic_bytecode::RuntimeStagedAuthorization>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
    event_groups: Vec<BytecodeEventGroup>,
}

struct EncodedSectionRef<'a> {
    decoded_length: u64,
    compressed: &'a [u8],
}

// The fixed container header owns only source identity; v9 policy identity is in the
// checksummed manifest and must also agree with the artifact before executable reuse.
struct ProjectSourceIdentity {
    project_revision: u64,
    source_digest: ProtocolBytes,
}

impl ProjectSourceIdentity {
    fn matches(&self, manifest: &ProjectManifest) -> bool {
        let actual = project_identity(manifest);
        self.project_revision == actual.project_revision
            && self.source_digest == actual.source_digest
    }
}

struct CompiledCacheSections<'a> {
    kind: ProjectContainerKind,
    version: u8,
    identity: ProjectSourceIdentity,
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
    configuration_journal: ConfigurationJournal,
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
    pub(crate) fn for_full_project_export(
        snapshot: &NormalizedProjectSnapshot,
        project_identity: [u8; 32],
        configuration: era_config::ConfigStore,
    ) -> Self {
        Self {
            project_identity,
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
            configuration_profile: ConfigurationClientProfile::Reference,
            configuration: configuration.clone(),
            editable_configuration: configuration,
            extensions: snapshot.extensions.clone(),
        }
    }

    fn into_snapshot(self, manifest: ProjectManifest) -> Result<NormalizedProjectSnapshot, String> {
        let resolved = crate::compatibility::resolve_manifest_compatibility(&manifest)
            .map_err(|diagnostic| diagnostic.message)?;
        if resolved.0 != manifest.compatibility
            || self.configuration.compatibility_profile() != manifest.compatibility.profile
        {
            return Err("cached configuration compatibility differs from manifest".into());
        }
        let resource_graph = self.resource_graph;
        let configuration_source_digest =
            crate::project::project_configuration_source_digest(&manifest.files);
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
            configuration_source_digest,
            generated_configuration_source: None,
            extensions: self.extensions,
        })
    }
}

