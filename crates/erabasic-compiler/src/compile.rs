use std::collections::BTreeMap;

use erabasic_analyzer::AnalyzedProject;
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeGlobal, BytecodePatch, BytecodeType, Digest,
    HostImport, NativeImport, SourceMap, SourceRecord, SymbolKey, create_patch,
};
use erabasic_hir::{Function, FunctionId, FunctionKind, SemanticType, Variable, VariableId};
use erabasic_validator::{ValidationContext, validate_bytecode, validate_hir};
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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct IncrementalState {
    compiler_abi: u32,
    functions: BTreeMap<SymbolKey, CachedFunction>,
    base_artifact: Option<BytecodeArtifact>,
}

impl IncrementalState {
    #[must_use]
    pub fn cached_function_count(&self) -> usize {
        self.functions.len()
    }

    #[must_use]
    pub fn base_artifact(&self) -> Option<&BytecodeArtifact> {
        self.base_artifact.as_ref()
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
    let mut cached = BTreeMap::new();
    let previous_functions = previous
        .filter(|state| state.compiler_abi == erabasic_bytecode::COMPILER_ABI_VERSION)
        .map(|state| &state.functions);
    let mut dirty = Vec::new();
    for function in &project.program.functions {
        let key = function_keys[&function.id];
        let function_bytes = serde_json::to_vec(function).expect("HIR function is serializable");
        let cache_key = Digest::hash(
            "rustyera.compiler.function.v1",
            &[&function_bytes, &shared_dependencies, &compiler_options.0],
        );
        if let Some(entry) = previous_functions
            .and_then(|functions| functions.get(&key))
            .filter(|entry| entry.cache_key == cache_key)
        {
            cached.insert(key, entry.clone());
        } else {
            dirty.push((function, key, cache_key));
        }
    }

    let compile_dirty = || {
        dirty
            .par_iter()
            .map(|(function, key, cache_key)| lower_function(function, *key, *cache_key, &context))
            .collect::<Vec<_>>()
    };
    let lowered = if let Some(jobs) = options.jobs {
        match rayon::ThreadPoolBuilder::new()
            .num_threads(jobs.max(1))
            .build()
        {
            Ok(pool) => pool.install(compile_dirty),
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
        compile_dirty()
    };
    let mut diagnostics = Vec::new();
    for result in lowered {
        diagnostics.extend(result.diagnostics.clone());
        let entry = cached_function(result);
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
    if !diagnostics.is_empty() {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics,
            stats: CompileStats {
                total_functions: project.program.functions.len(),
                compiled_functions: dirty.len(),
                reused_functions: project.program.functions.len() - dirty.len(),
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
    let mut artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(compiler_options),
        project_data: project.data.clone(),
        globals: globals(&project.program.variables, &variable_keys),
        native_imports: native_imports.into_values().collect(),
        host_imports: host_imports.into_values().collect(),
        functions: cached
            .values()
            .map(|entry| entry.function.clone())
            .collect(),
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
    let validation = validate_bytecode(
        artifact.clone().into_unvalidated(),
        &ValidationContext::for_artifact(&artifact),
    );
    if !validation.is_valid() {
        return CompileReport {
            artifact: None,
            patch: None,
            incremental_state: previous.cloned().unwrap_or_default(),
            diagnostics: validation
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    CompilerDiagnostic::new(CompilerDiagnosticCode::Validation, diagnostic.message)
                })
                .collect(),
            stats: CompileStats::default(),
        };
    }
    let base = previous.and_then(|state| state.base_artifact.as_ref());
    let patch = base.map(|base| create_patch(base, &artifact));
    let patch_functions = patch
        .as_ref()
        .map_or(0, |patch| patch.changed_functions.len());
    let stats = CompileStats {
        total_functions: project.program.functions.len(),
        compiled_functions: dirty.len(),
        reused_functions: project.program.functions.len() - dirty.len(),
        patch_functions,
    };
    let incremental_state = IncrementalState {
        compiler_abi: erabasic_bytecode::COMPILER_ABI_VERSION,
        functions: cached,
        base_artifact: Some(artifact.clone()),
    };
    CompileReport {
        artifact: Some(artifact),
        patch,
        incremental_state,
        diagnostics,
        stats,
    }
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
            let identity = serde_json::to_vec(&(
                variable.name.to_ascii_uppercase(),
                variable.scope,
                owner,
                variable.value_type,
                &variable.dimensions,
            ))
            .expect("variable identity is serializable");
            (
                variable.id,
                SymbolKey::derive("rustyera.bytecode.variable.v1", &identity),
            )
        })
        .collect()
}

fn globals(variables: &[Variable], keys: &BTreeMap<VariableId, SymbolKey>) -> Vec<BytecodeGlobal> {
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
            })
        })
        .collect()
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
