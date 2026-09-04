use std::collections::BTreeMap;

use erabasic_bytecode::{BytecodeArtifact, BytecodePatch, ImportKind, SymbolKey};

use crate::lowering::{LoweredFunction, LoweredSourceMapEntry};

use super::{CachedFunction, IncrementalState, MaterializedFunction};

pub(super) fn create_incremental_patch(
    base: &IncrementalState,
    base_artifact: Option<&BytecodeArtifact>,
    target: &BytecodeArtifact,
) -> Option<BytecodePatch> {
    let metadata = base.base.as_ref()?;
    let exact_base = base_artifact
        .filter(|artifact| artifact.manifest.artifact_id == metadata.manifest.artifact_id);
    let compact_metadata = metadata.metadata.as_deref();
    let target_keys = target
        .functions
        .iter()
        .map(|function| function.key)
        .collect::<std::collections::BTreeSet<_>>();
    Some(BytecodePatch {
        base_artifact_id: metadata.manifest.artifact_id,
        base_execution_id: metadata.manifest.program_version.execution_id,
        target_manifest: target.manifest.clone(),
        runtime_builtins: exact_base
            .map(|artifact| &artifact.runtime_builtins)
            .or_else(|| compact_metadata.map(|metadata| &metadata.runtime_builtins))
            .is_none_or(|base| base != &target.runtime_builtins)
            .then(|| target.runtime_builtins.clone()),
        runtime_variables: exact_base
            .map(|artifact| &artifact.runtime_variables)
            .or_else(|| compact_metadata.map(|metadata| &metadata.runtime_variables))
            .is_none_or(|base| base != &target.runtime_variables)
            .then(|| target.runtime_variables.clone()),
        runtime_native_authorizations: exact_base
            .map(|artifact| &artifact.runtime_native_authorizations)
            .or_else(|| compact_metadata.map(|metadata| &metadata.runtime_native_authorizations))
            .is_none_or(|base| base != &target.runtime_native_authorizations)
            .then(|| target.runtime_native_authorizations.clone()),
        runtime_host_authorizations: exact_base
            .map(|artifact| &artifact.runtime_host_authorizations)
            .or_else(|| compact_metadata.map(|metadata| &metadata.runtime_host_authorizations))
            .is_none_or(|base| base != &target.runtime_host_authorizations)
            .then(|| target.runtime_host_authorizations.clone()),
        runtime_staged_authorizations: exact_base
            .map(|artifact| &artifact.runtime_staged_authorizations)
            .or_else(|| compact_metadata.map(|metadata| &metadata.runtime_staged_authorizations))
            .is_none_or(|base| base != &target.runtime_staged_authorizations)
            .then(|| target.runtime_staged_authorizations.clone()),
        call_compatibility: exact_base
            .map(|artifact| artifact.call_compatibility)
            .or_else(|| compact_metadata.map(|metadata| metadata.call_compatibility))
            .is_none_or(|base| base != target.call_compatibility)
            .then_some(target.call_compatibility),
        project_data: exact_base
            .map(|artifact| &artifact.project_data)
            .or_else(|| compact_metadata.map(|metadata| &metadata.project_data))
            .is_none_or(|base| base != &target.project_data)
            .then(|| target.project_data.clone()),
        globals: exact_base
            .map(|artifact| &artifact.globals)
            .or_else(|| compact_metadata.map(|metadata| &metadata.globals))
            .is_none_or(|base| base != &target.globals)
            .then(|| target.globals.clone()),
        native_imports: exact_base
            .map(|artifact| &artifact.native_imports)
            .or_else(|| compact_metadata.map(|metadata| &metadata.native_imports))
            .is_none_or(|base| base != &target.native_imports)
            .then(|| target.native_imports.clone()),
        host_imports: exact_base
            .map(|artifact| &artifact.host_imports)
            .or_else(|| compact_metadata.map(|metadata| &metadata.host_imports))
            .is_none_or(|base| base != &target.host_imports)
            .then(|| target.host_imports.clone()),
        changed_functions: target
            .functions
            .iter()
            .filter(|function| {
                base.functions
                    .get(&function.key)
                    .and_then(|cached| cached_function_body(cached, base_artifact))
                    != Some(function)
            })
            .cloned()
            .collect(),
        removed_functions: base
            .functions
            .values()
            .filter(|cached| !target_keys.contains(&cached.function_key))
            .map(|cached| cached.function_key)
            .collect(),
        event_groups: exact_base
            .map(|artifact| &artifact.event_groups)
            .or_else(|| compact_metadata.map(|metadata| &metadata.event_groups))
            .is_none_or(|base| base != &target.event_groups)
            .then(|| target.event_groups.clone()),
        source_map: target.source_map.clone(),
    })
}

pub(super) fn materialized_function(result: LoweredFunction) -> MaterializedFunction {
    MaterializedFunction {
        cache_key: result.cache_key,
        function: result.function,
        source_entries: result.source_entries,
        native_imports: result.native_imports,
        host_imports: result.host_imports,
    }
}

pub(super) fn cached_function(result: &MaterializedFunction) -> CachedFunction {
    CachedFunction {
        cache_key: result.cache_key,
        function_key: result.function.key,
        function: Some(result.function.clone()),
        source_entries: result.source_entries.clone(),
        native_imports: result.native_imports.clone(),
        host_imports: result.host_imports.clone(),
    }
}

pub(super) fn compact_cached_function(result: &MaterializedFunction) -> CachedFunction {
    CachedFunction {
        cache_key: result.cache_key,
        function_key: result.function.key,
        function: None,
        source_entries: Vec::new(),
        native_imports: Vec::new(),
        host_imports: Vec::new(),
    }
}

fn cached_function_body<'a>(
    cached: &'a CachedFunction,
    artifact: Option<&'a BytecodeArtifact>,
) -> Option<&'a erabasic_bytecode::BytecodeFunction> {
    cached.function.as_ref().or_else(|| {
        artifact?
            .functions
            .iter()
            .find(|function| function.key == cached.function_key)
    })
}

pub(super) struct PreviousArtifactIndex<'a> {
    artifact: &'a BytecodeArtifact,
    functions: BTreeMap<SymbolKey, usize>,
    source_ranges: BTreeMap<SymbolKey, std::ops::Range<usize>>,
    native_imports: BTreeMap<SymbolKey, usize>,
    host_imports: BTreeMap<SymbolKey, usize>,
}

impl<'a> PreviousArtifactIndex<'a> {
    pub(super) fn new(artifact: &'a BytecodeArtifact) -> Self {
        let functions = artifact
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.key, index))
            .collect();
        let mut source_ranges = BTreeMap::<SymbolKey, std::ops::Range<usize>>::new();
        for (index, entry) in artifact.source_map.entries.iter().enumerate() {
            source_ranges
                .entry(entry.function)
                .and_modify(|range| range.end = index + 1)
                .or_insert(index..index + 1);
        }
        let native_imports = artifact
            .native_imports
            .iter()
            .enumerate()
            .map(|(index, import)| (import.import.key, index))
            .collect();
        let host_imports = artifact
            .host_imports
            .iter()
            .enumerate()
            .map(|(index, import)| (import.import.key, index))
            .collect();
        Self {
            artifact,
            functions,
            source_ranges,
            native_imports,
            host_imports,
        }
    }
}

pub(super) fn materialize_cached_function(
    cached: &CachedFunction,
    previous: Option<&PreviousArtifactIndex<'_>>,
) -> Option<MaterializedFunction> {
    if let Some(function) = &cached.function {
        return Some(MaterializedFunction {
            cache_key: cached.cache_key,
            function: function.clone(),
            source_entries: cached.source_entries.clone(),
            native_imports: cached.native_imports.clone(),
            host_imports: cached.host_imports.clone(),
        });
    }
    let previous = previous?;
    let function = previous
        .artifact
        .functions
        .get(*previous.functions.get(&cached.function_key)?)?
        .clone();
    let source_entries = previous
        .source_ranges
        .get(&cached.function_key)
        .map_or_else(
            || Some(Vec::new()),
            |range| {
                previous.artifact.source_map.entries[range.clone()]
                    .iter()
                    .map(|entry| {
                        Some(LoweredSourceMapEntry {
                            function: entry.function,
                            code_start: entry.code_start,
                            code_end: entry.code_end,
                            source_index: entry.source_index,
                            byte_start: entry.byte_start,
                            byte_end: entry.byte_end,
                            statement_fingerprint: previous
                                .artifact
                                .source_map
                                .statement_fingerprint(entry)?,
                            origin_chain: entry.origin_chain.clone(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            },
        )?;
    let mut native_imports = Vec::new();
    let mut host_imports = Vec::new();
    for import in &function.imports {
        match import.kind {
            ImportKind::Native => native_imports.push(
                previous
                    .artifact
                    .native_imports
                    .get(*previous.native_imports.get(&import.key)?)?
                    .clone(),
            ),
            ImportKind::Host => host_imports.push(
                previous
                    .artifact
                    .host_imports
                    .get(*previous.host_imports.get(&import.key)?)?
                    .clone(),
            ),
            ImportKind::Function => {}
        }
    }
    Some(MaterializedFunction {
        cache_key: cached.cache_key,
        function,
        source_entries,
        native_imports,
        host_imports,
    })
}
