use std::collections::BTreeMap;

use erabasic_analyzer::AnalyzedProject;
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeConstant, BytecodeEventEntry, BytecodeEventGroup,
    BytecodeGlobal, BytecodePatch, BytecodePersistence, BytecodeStorage, BytecodeType, Digest,
    HostImport, NativeImport, SourceMap, SourceRecord, SymbolKey,
};
use erabasic_data::{Persistence, StorageScope};
use erabasic_hir::{
    ConstantValue, Function, FunctionId, FunctionKind, SemanticType, Variable, VariableId,
    VariableScope,
};
use erabasic_validator::{ValidationContext, validate_compiler_output, validate_hir};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    CompilerDiagnostic, CompilerDiagnosticCode, CompilerOptions, HostRegistry,
    lowering::{LoweredFunction, LoweringContext, bytecode_type, lower_function},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachedFunction {
    cache_key: Digest,
    function: erabasic_bytecode::BytecodeFunction,
    source_entries: Vec<erabasic_bytecode::SourceMapEntry>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
}

enum FunctionBuild {
    Cached(CachedFunction),
    Lowered(LoweredFunction),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IncrementalBase {
    manifest: ArtifactManifest,
    call_compatibility: erabasic_bytecode::BytecodeCallCompatibility,
    project_data: erabasic_data::ProjectData,
    globals: Vec<BytecodeGlobal>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
    event_groups: Vec<BytecodeEventGroup>,
}

impl IncrementalBase {
    fn from_artifact(artifact: &BytecodeArtifact) -> Self {
        Self {
            manifest: artifact.manifest.clone(),
            call_compatibility: artifact.call_compatibility,
            project_data: artifact.project_data.clone(),
            globals: artifact.globals.clone(),
            native_imports: artifact.native_imports.clone(),
            host_imports: artifact.host_imports.clone(),
            event_groups: artifact.event_groups.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncrementalState {
    compiler_abi: u32,
    functions: BTreeMap<SymbolKey, CachedFunction>,
    base: Option<IncrementalBase>,
}

impl IncrementalState {
    #[must_use]
    pub fn cached_function_count(&self) -> usize {
        self.functions.len()
    }

    #[must_use]
    pub fn base_artifact_id(&self) -> Option<Digest> {
        self.base.as_ref().map(|base| base.manifest.artifact_id)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileStats {
    pub total_functions: usize,
    pub compiled_functions: usize,
    pub reused_functions: usize,
    pub patch_functions: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompileReport {
    pub artifact: Option<BytecodeArtifact>,
    pub patch: Option<BytecodePatch>,
    pub incremental_state: IncrementalState,
    pub diagnostics: Vec<CompilerDiagnostic>,
    pub stats: CompileStats,
}

#[must_use]
#[allow(clippy::too_many_lines)]
/// Compile one analyzed, in-memory project into a self-contained artifact.
///
/// # Panics
///
/// Panics only if the crate's own fixed, Serde-derived identity tuples stop being
/// serializable. User-provided source and project values are reported as diagnostics.
pub fn compile_project(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
) -> CompileReport {
    let hir_report = validate_hir(&project.program, &project.data);
    if !hir_report.is_valid() {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics: hir_report
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    CompilerDiagnostic::new(CompilerDiagnosticCode::InvalidHir, diagnostic.message)
                })
                .collect(),
            stats: CompileStats::default(),
        };
    }

    let options_bytes = serde_json::to_vec(&options.optimization)
        .expect("compiler semantic options are serializable");
    let compiler_options = Digest::hash("rustyera.compiler.options.v1", &[&options_bytes]);
    let function_keys = function_keys(&project.program.functions, &project.program.sources);
    let variable_keys = variable_keys(&project.program.variables, &function_keys);
    let functions_by_id = project
        .program
        .functions
        .iter()
        .map(|function| (function.id, function))
        .collect();
    let source_indices = project
        .program
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| (source.id, u32::try_from(index).unwrap_or(u32::MAX)))
        .collect();
    let context = LoweringContext {
        program: &project.program,
        function_keys: &function_keys,
        functions_by_id: &functions_by_id,
        variable_keys: &variable_keys,
        source_indices: &source_indices,
        host_registry,
    };
    let shared_dependencies = serde_json::to_vec(&(
        &project.program.variables,
        host_registry,
        options.optimization,
    ))
    .expect("compiler dependencies are serializable");
    // Hash the project-wide block once. Feeding it into BLAKE3 for every function
    // made cold compilation O(function count × static-data size) on large games.
    let shared_dependencies = Digest::hash(
        "rustyera.compiler.shared-dependencies.v1",
        &[&shared_dependencies],
    );
    let previous_functions = previous
        .filter(|state| state.compiler_abi == erabasic_bytecode::COMPILER_ABI_VERSION)
        .map(|state| &state.functions);
    let compile_functions = || {
        project
            .program
            .functions
            .par_iter()
            .map(|function| {
                let key = function_keys[&function.id];
                let function_bytes =
                    serde_json::to_vec(function).expect("HIR function is serializable");
                let cache_key = Digest::hash(
                    "rustyera.compiler.function.v2",
                    &[&function_bytes, &shared_dependencies.0, &compiler_options.0],
                );
                if let Some(entry) = previous_functions
                    .and_then(|functions| functions.get(&key))
                    .filter(|entry| entry.cache_key == cache_key)
                {
                    FunctionBuild::Cached(entry.clone())
                } else {
                    FunctionBuild::Lowered(lower_function(function, key, cache_key, &context))
                }
            })
            .collect::<Vec<_>>()
    };
    // Cache hashing and lowering are both function-local. Running them in one
    // indexed parallel iterator preserves deterministic input order while
    // avoiding a serial hashing pass before worker threads can start lowering.
    let function_builds = if let Some(jobs) = options.jobs {
        match rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()
        {
            Ok(pool) => pool.install(compile_functions),
            Err(error) => {
                return CompileReport {
                    artifact: None,
                    patch: None,
                    incremental_state: previous.cloned().unwrap_or_default(),
                    diagnostics: vec![CompilerDiagnostic::new(
                        CompilerDiagnosticCode::Parallelism,
                        error.to_string(),
                    )],
                    stats: CompileStats::default(),
                };
            }
        }
    } else {
        compile_functions()
    };
    let mut cached = BTreeMap::new();
    let mut lowered_count = 0usize;
    let mut diagnostics = Vec::new();
    for result in function_builds {
        let entry = match result {
            FunctionBuild::Cached(entry) => entry,
            FunctionBuild::Lowered(result) => {
                lowered_count += 1;
                diagnostics.extend(result.diagnostics.clone());
                cached_function(result)
            }
        };
        cached.insert(entry.function.key, entry);
    }
    diagnostics.sort_by_key(|diagnostic| {
        diagnostic
            .location
            .map_or((u32::MAX, usize::MAX, diagnostic.code as u8), |location| {
                (
                    location.source.0,
                    location.span.start,
                    diagnostic.code as u8,
                )
            })
    });
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::CompilerDiagnosticSeverity::Error)
    {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics,
            stats: CompileStats {
                total_functions: project.program.functions.len(),
                compiled_functions: lowered_count,
                reused_functions: project.program.functions.len() - lowered_count,
                patch_functions: 0,
            },
        };
    }

    let mut native_imports = BTreeMap::new();
    let mut host_imports = BTreeMap::new();
    let mut source_entries = Vec::new();
    for entry in cached.values() {
        for import in &entry.native_imports {
            native_imports.insert(import.import.key, import.clone());
        }
        for import in &entry.host_imports {
            host_imports.insert(import.import.key, import.clone());
        }
        source_entries.extend(entry.source_entries.clone());
    }
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(compiler_options),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility {
            allow_event_as_normal: project.program.call_compatibility.allow_event_as_normal,
            allow_omitted_arguments: project.program.call_compatibility.allow_omitted_arguments,
            auto_convert_integer_to_string: project
                .program
                .call_compatibility
                .auto_convert_integer_to_string,
        },
        project_data: project.data.clone(),
        globals: globals(&project.program.variables, &variable_keys, &function_keys),
        native_imports: native_imports.into_values().collect(),
        host_imports: host_imports.into_values().collect(),
        functions: cached
            .values()
            .map(|entry| entry.function.clone())
            .collect(),
        event_groups: event_groups(&project.program.functions, &function_keys),
        source_map: SourceMap {
            sources: project
                .program
                .sources
                .iter()
                .map(|source| SourceRecord {
                    relative_path: source.relative_path.clone(),
                    content_hash: Digest(source.content_hash),
                    byte_len: source.byte_len,
                    line_starts: source.line_starts.clone(),
                })
                .collect(),
            entries: source_entries,
        },
    };
    // Compiler output has no identity to verify yet. Validate its structure in
    // place, then serialize the complete artifact only once to assign final IDs.
    // Untrusted decoded artifacts continue to use the validator's identity-checking path.
    let validation_context = ValidationContext::for_artifact(&artifact);
    let validation = validate_compiler_output(artifact, &validation_context);
    if !validation.is_valid() {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics: validation
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    let context = match (diagnostic.function, diagnostic.instruction) {
                        (Some(function), Some(instruction)) => {
                            format!("function {function}, instruction {instruction}: ")
                        }
                        (Some(function), None) => format!("function {function}: "),
                        (None, Some(instruction)) => format!("instruction {instruction}: "),
                        (None, None) => String::new(),
                    };
                    CompilerDiagnostic::new(
                        CompilerDiagnosticCode::Validation,
                        format!("{context}{}", diagnostic.message),
                    )
                })
                .collect(),
            stats: CompileStats::default(),
        };
    }
    let mut artifact = validation
        .value
        .expect("a valid compiler artifact is returned by the validator")
        .into_inner();
    if let Err(error) = artifact.refresh_ids() {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics: vec![CompilerDiagnostic::new(
                CompilerDiagnosticCode::Encoding,
                error.to_string(),
            )],
            stats: CompileStats::default(),
        };
    }
    let patch = previous.and_then(|base| create_incremental_patch(base, &artifact));
    let patch_functions = patch
        .as_ref()
        .map_or(0, |patch| patch.changed_functions.len());
    let stats = CompileStats {
        total_functions: project.program.functions.len(),
        compiled_functions: lowered_count,
        reused_functions: project.program.functions.len() - lowered_count,
        patch_functions,
    };
    let incremental_state = IncrementalState {
        compiler_abi: erabasic_bytecode::COMPILER_ABI_VERSION,
        functions: cached,
        // Function bodies and source entries already live in `functions`; retain only the
        // remaining fields needed to compare the next build instead of cloning the artifact.
        base: Some(IncrementalBase::from_artifact(&artifact)),
    };
    CompileReport {
        artifact: Some(artifact),
        patch,
        incremental_state,
        diagnostics,
        stats,
    }
}

fn create_incremental_patch(
    base: &IncrementalState,
    target: &BytecodeArtifact,
) -> Option<BytecodePatch> {
    let metadata = base.base.as_ref()?;
    let target_keys = target
        .functions
        .iter()
        .map(|function| function.key)
        .collect::<std::collections::BTreeSet<_>>();
    Some(BytecodePatch {
        base_artifact_id: metadata.manifest.artifact_id,
        base_execution_id: metadata.manifest.program_version.execution_id,
        target_manifest: target.manifest.clone(),
        call_compatibility: (metadata.call_compatibility != target.call_compatibility)
            .then_some(target.call_compatibility),
        project_data: (metadata.project_data != target.project_data)
            .then(|| target.project_data.clone()),
        globals: (metadata.globals != target.globals).then(|| target.globals.clone()),
        native_imports: (metadata.native_imports != target.native_imports)
            .then(|| target.native_imports.clone()),
        host_imports: (metadata.host_imports != target.host_imports)
            .then(|| target.host_imports.clone()),
        changed_functions: target
            .functions
            .iter()
            .filter(|function| {
                base.functions
                    .get(&function.key)
                    .map(|cached| &cached.function)
                    != Some(function)
            })
            .cloned()
            .collect(),
        removed_functions: base
            .functions
            .values()
            .filter(|cached| !target_keys.contains(&cached.function.key))
            .map(|cached| cached.function.key)
            .collect(),
        event_groups: (metadata.event_groups != target.event_groups)
            .then(|| target.event_groups.clone()),
        source_map: target.source_map.clone(),
    })
}

fn event_groups(
    functions: &[Function],
    keys: &BTreeMap<FunctionId, SymbolKey>,
) -> Vec<BytecodeEventGroup> {
    let mut groups: BTreeMap<String, Vec<&Function>> = BTreeMap::new();
    for function in functions
        .iter()
        .filter(|function| function.kind == FunctionKind::Event)
    {
        groups
            .entry(function.name.to_ascii_uppercase())
            .or_default()
            .push(function);
    }
    groups
        .into_iter()
        .map(|(name, mut members)| {
            members.sort_by_key(|function| function.definition_order);
            let mut group = BytecodeEventGroup {
                name,
                only: Vec::new(),
                priority: Vec::new(),
                normal: Vec::new(),
                later: Vec::new(),
            };
            for function in members {
                let Some(function_key) = keys.get(&function.id).copied() else {
                    continue;
                };
                let entry = BytecodeEventEntry {
                    function: function_key,
                    single: function.event_attributes.single,
                };
                if function.event_attributes.only {
                    group.only.push(entry);
                }
                if function.event_attributes.priority {
                    group.priority.push(entry);
                }
                if function.event_attributes.later {
                    group.later.push(entry);
                }
                if !function.event_attributes.priority && !function.event_attributes.later {
                    group.normal.push(entry);
                }
            }
            group
        })
        .collect()
}

fn cached_function(result: LoweredFunction) -> CachedFunction {
    CachedFunction {
        cache_key: result.cache_key,
        function: result.function,
        source_entries: result.source_entries,
        native_imports: result.native_imports,
        host_imports: result.host_imports,
    }
}

fn function_keys(
    functions: &[Function],
    sources: &[erabasic_hir::SourceFile],
) -> BTreeMap<FunctionId, SymbolKey> {
    let paths: BTreeMap<_, _> = sources
        .iter()
        .map(|source| (source.id, source.relative_path.as_str()))
        .collect();
    let mut ordinals = BTreeMap::new();
    functions
        .iter()
        .map(|function| {
            let identity = (
                paths
                    .get(&function.location.source)
                    .copied()
                    .unwrap_or_default(),
                function.name.to_ascii_uppercase(),
                function_kind_tag(function.kind),
                function
                    .parameters
                    .iter()
                    .map(|parameter| semantic_type_tag(parameter.target.value_type))
                    .collect::<Vec<_>>(),
            );
            let ordinal = ordinals.entry(identity.clone()).or_insert(0u32);
            let bytes = serde_json::to_vec(&(identity, *ordinal))
                .expect("function identity is serializable");
            *ordinal += 1;
            (
                function.id,
                SymbolKey::derive("rustyera.bytecode.function.v1", &bytes),
            )
        })
        .collect()
}

fn variable_keys(
    variables: &[Variable],
    functions: &BTreeMap<FunctionId, SymbolKey>,
) -> BTreeMap<VariableId, SymbolKey> {
    variables
        .iter()
        .map(|variable| {
            let owner = variable
                .owner
                .and_then(|owner| functions.get(&owner).copied());
            let identity =
                serde_json::to_vec(&(variable.name.to_ascii_uppercase(), variable.scope, owner))
                    .expect("variable identity is serializable");
            (
                variable.id,
                SymbolKey::derive("rustyera.bytecode.variable.v2", &identity),
            )
        })
        .collect()
}

fn globals(
    variables: &[Variable],
    keys: &BTreeMap<VariableId, SymbolKey>,
    functions: &BTreeMap<FunctionId, SymbolKey>,
) -> Vec<BytecodeGlobal> {
    variables
        .iter()
        .filter_map(|variable| {
            Some(BytecodeGlobal {
                key: keys[&variable.id],
                name: variable.name.clone(),
                value_type: bytecode_type(variable.value_type)?,
                dimensions: variable
                    .dimensions
                    .iter()
                    .map(|dimension| *dimension as u64)
                    .collect(),
                mutable: variable.mutable,
                storage: variable_storage(variable),
                persistence: persistence(variable.persistence),
                initial_values: variable
                    .initial_values
                    .iter()
                    .map(|value| match value {
                        ConstantValue::Integer(value) => BytecodeConstant::Integer(*value),
                        ConstantValue::String(value) => BytecodeConstant::String(value.clone()),
                    })
                    .collect(),
                owner: variable
                    .owner
                    .and_then(|owner| functions.get(&owner).copied()),
            })
        })
        .collect()
}

fn variable_storage(variable: &Variable) -> BytecodeStorage {
    if matches!(
        variable.scope,
        VariableScope::EraFunction | VariableScope::Function | VariableScope::Parameter
    ) {
        if variable.scope == VariableScope::EraFunction {
            return BytecodeStorage::FunctionPersistent;
        }
        return if variable.static_lifetime {
            BytecodeStorage::FunctionStatic
        } else {
            BytecodeStorage::FunctionLocal
        };
    }
    match variable.storage {
        StorageScope::Normal | StorageScope::Global | StorageScope::Local => {
            BytecodeStorage::Project
        }
        StorageScope::Character => BytecodeStorage::Character,
        StorageScope::Constant => BytecodeStorage::Constant,
        StorageScope::Calculated => BytecodeStorage::Calculated,
    }
}

const fn persistence(value: Persistence) -> BytecodePersistence {
    match value {
        Persistence::None => BytecodePersistence::None,
        Persistence::GameSave => BytecodePersistence::GameSave,
        Persistence::GlobalSave => BytecodePersistence::GlobalSave,
        Persistence::ExtendedSave => BytecodePersistence::ExtendedSave,
    }
}

const fn function_kind_tag(kind: FunctionKind) -> u8 {
    match kind {
        FunctionKind::Normal => 0,
        FunctionKind::Event => 1,
        FunctionKind::System => 2,
        FunctionKind::Method => 3,
    }
}

const fn semantic_type_tag(value_type: SemanticType) -> u8 {
    match value_type {
        SemanticType::Integer => 0,
        SemanticType::String => 1,
        SemanticType::Void => 2,
        SemanticType::Error => 3,
    }
}

#[allow(dead_code)]
const fn type_for_semantic(value_type: SemanticType) -> Option<BytecodeType> {
    match value_type {
        SemanticType::Integer => Some(BytecodeType::Integer),
        SemanticType::String => Some(BytecodeType::String),
        SemanticType::Void | SemanticType::Error => None,
    }
}
