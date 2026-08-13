#[allow(clippy::wildcard_imports)]
use super::*;

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

pub(super) fn encode_manifest_section(
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

pub(super) fn encode_project_file_header(
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
    let project_revision = container_project_revision(kind, identity.project_revision);
    output.extend_from_slice(&project_revision.to_le_bytes());
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

fn container_project_revision(kind: ProjectContainerKind, project_revision: u64) -> u64 {
    match kind {
        ProjectContainerKind::CompiledCache => COMPILED_CACHE_PROJECT_REVISION,
        ProjectContainerKind::FullProject => project_revision,
    }
}
