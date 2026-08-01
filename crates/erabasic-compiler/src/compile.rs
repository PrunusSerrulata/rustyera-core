use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use erabasic_analyzer::AnalyzedProject;
use erabasic_bytecode::{
    ArtifactManifest, BytecodeArtifact, BytecodeEventGroup, BytecodeGlobal, BytecodePatch, Digest,
    HostImport, NativeImport, SourceMap, SourceMapEntry, SourceRecord, SymbolKey,
};
use erabasic_hir::Function;
use erabasic_validator::{ValidationContext, validate_compiler_output, validate_hir};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    CompilerDiagnostic, CompilerDiagnosticCode, CompilerOptions, HostRegistry,
    lowering::{LoweredFunction, LoweredSourceMapEntry, LoweringContext, lower_function},
};

mod artifact;
mod incremental;

use artifact::{canonical_digest, event_groups, function_keys, globals, variable_keys};
use incremental::{
    PreviousArtifactIndex, cached_function, compact_cached_function, create_incremental_patch,
    materialize_cached_function, materialized_function,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct CachedFunction {
    cache_key: Digest,
    function_key: SymbolKey,
    function: Option<erabasic_bytecode::BytecodeFunction>,
    source_entries: Vec<LoweredSourceMapEntry>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
}

enum FunctionBuild {
    Cached(MaterializedFunction),
    Lowered(LoweredFunction),
}

struct MaterializedFunction {
    cache_key: Digest,
    function: erabasic_bytecode::BytecodeFunction,
    source_entries: Vec<LoweredSourceMapEntry>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IncrementalBase {
    manifest: ArtifactManifest,
    metadata: Option<Box<IncrementalMetadata>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct IncrementalMetadata {
    call_compatibility: erabasic_bytecode::BytecodeCallCompatibility,
    project_data: erabasic_data::ProjectData,
    globals: Vec<BytecodeGlobal>,
    native_imports: Vec<NativeImport>,
    host_imports: Vec<HostImport>,
    event_groups: Vec<BytecodeEventGroup>,
}

impl IncrementalBase {
    fn from_artifact(artifact: &BytecodeArtifact, include_metadata: bool) -> Self {
        Self {
            manifest: artifact.manifest.clone(),
            metadata: include_metadata.then(|| {
                Box::new(IncrementalMetadata {
                    call_compatibility: artifact.call_compatibility,
                    project_data: artifact.project_data.clone(),
                    globals: artifact.globals.clone(),
                    native_imports: artifact.native_imports.clone(),
                    host_imports: artifact.host_imports.clone(),
                    event_groups: artifact.event_groups.clone(),
                })
            }),
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

    /// Discard function payloads that already exist in the active artifact.
    ///
    /// Runtime callers retain that artifact independently and can supply it to
    /// [`compile_project_with_artifact`] on the next reload. Keeping only cache
    /// keys avoids a second resident copy of all code and source-map records.
    pub fn compact(&mut self) {
        for cached in self.functions.values_mut() {
            cached.function = None;
            cached.source_entries.clear();
            cached.native_imports.clear();
            cached.host_imports.clear();
        }
        if let Some(base) = &mut self.base {
            base.metadata = None;
        }
    }

    /// Return compact cache keys in the artifact's canonical function order.
    ///
    /// Project files already contain the complete artifact, so serializing the
    /// symbol-keyed map and its duplicate function keys only wastes space.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is not the compact cache for the exact
    /// supplied artifact and current compiler ABI.
    pub fn compact_cache_keys(&self, artifact: &BytecodeArtifact) -> Result<Vec<Digest>, String> {
        if self.compiler_abi != erabasic_bytecode::COMPILER_ABI_VERSION {
            return Err("incremental cache compiler ABI differs from the artifact".into());
        }
        if self.base_artifact_id() != Some(artifact.manifest.artifact_id) {
            return Err("incremental cache base differs from the artifact".into());
        }
        if self.functions.len() != artifact.functions.len() {
            return Err("incremental cache function count differs from the artifact".into());
        }
        artifact
            .functions
            .iter()
            .map(|function| {
                let cached = self.functions.get(&function.key).ok_or_else(|| {
                    "incremental cache is missing an artifact function".to_owned()
                })?;
                if cached.function_key != function.key {
                    return Err("incremental cache function key is inconsistent".into());
                }
                Ok(cached.cache_key)
            })
            .collect()
    }

    /// Rebuild a compact incremental cache from canonical artifact function keys.
    ///
    /// # Errors
    ///
    /// Returns an error when the cache-key count differs from the artifact's
    /// canonical function count.
    pub fn from_compact_cache_keys(
        artifact: &BytecodeArtifact,
        cache_keys: Vec<Digest>,
    ) -> Result<Self, String> {
        if cache_keys.len() != artifact.functions.len() {
            return Err("incremental cache key count differs from the artifact".into());
        }
        let functions = artifact
            .functions
            .iter()
            .zip(cache_keys)
            .map(|(function, cache_key)| {
                (
                    function.key,
                    CachedFunction {
                        cache_key,
                        function_key: function.key,
                        function: None,
                        source_entries: Vec::new(),
                        native_imports: Vec::new(),
                        host_imports: Vec::new(),
                    },
                )
            })
            .collect();
        Ok(Self {
            compiler_abi: erabasic_bytecode::COMPILER_ABI_VERSION,
            functions,
            base: Some(IncrementalBase::from_artifact(artifact, false)),
        })
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompileProgress {
    pub completed: usize,
    pub total: usize,
}

#[cfg(not(target_arch = "wasm32"))]
pub trait CompileProgressCallback: Fn(CompileProgress) + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T> CompileProgressCallback for T where T: Fn(CompileProgress) + Sync {}

#[cfg(target_arch = "wasm32")]
pub trait CompileProgressCallback: Fn(CompileProgress) {}

#[cfg(target_arch = "wasm32")]
impl<T> CompileProgressCallback for T where T: Fn(CompileProgress) {}

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
    compile_project_inner(project, options, host_registry, previous, None, false, None)
}

/// Compile with an exact previous artifact backing a compact incremental cache.
///
/// Runtime owners use this entry point because they already retain the executable
/// artifact. The returned cache is compact and therefore must again be paired with
/// its exact artifact on the next incremental build.
#[must_use]
pub fn compile_project_with_artifact(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> CompileReport {
    compile_project_inner(
        project,
        options,
        host_registry,
        previous,
        previous_artifact,
        true,
        None,
    )
}

#[must_use]
pub fn compile_project_with_artifact_and_progress(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> CompileReport {
    compile_project_inner(
        project,
        options,
        host_registry,
        previous,
        previous_artifact,
        true,
        Some(progress),
    )
}

#[allow(clippy::too_many_lines)]
fn compile_project_inner(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    compact_cache: bool,
    progress: Option<&dyn CompileProgressCallback>,
) -> CompileReport {
    let total_functions = project.program.functions.len();
    if let Some(progress) = progress {
        progress(CompileProgress {
            completed: 0,
            total: total_functions,
        });
    }
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

    let compiler_options = canonical_digest("rustyera.compiler.options.v2", &options.optimization);
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
    let shared_dependencies = canonical_digest(
        "rustyera.compiler.shared-dependencies.v2",
        &(
            &project.program.variables,
            host_registry,
            options.optimization,
        ),
    );
    let previous_functions = previous
        .filter(|state| state.compiler_abi == erabasic_bytecode::COMPILER_ABI_VERSION)
        .map(|state| &state.functions);
    let previous_artifact = previous_artifact.filter(|artifact| {
        previous.and_then(IncrementalState::base_artifact_id) == Some(artifact.manifest.artifact_id)
    });
    let previous_artifact_index = previous_artifact.map(PreviousArtifactIndex::new);
    let completed_functions = AtomicUsize::new(0);
    let reported_percent = AtomicUsize::new(0);
    let progress_callback_lock = Mutex::new(());
    let compile_one = |function: &Function| {
        let build = {
            let key = function_keys[&function.id];
            let function_digest = canonical_digest("rustyera.compiler.hir-function.v3", function);
            let cache_key = Digest::hash(
                "rustyera.compiler.function.v3",
                &[
                    &function_digest.0,
                    &shared_dependencies.0,
                    &compiler_options.0,
                ],
            );
            if let Some(entry) = previous_functions
                .and_then(|functions| functions.get(&key))
                .filter(|entry| entry.cache_key == cache_key)
                .and_then(|entry| {
                    materialize_cached_function(entry, previous_artifact_index.as_ref())
                })
            {
                FunctionBuild::Cached(entry)
            } else {
                FunctionBuild::Lowered(lower_function(function, key, cache_key, &context))
            }
        };
        let completed = completed_functions.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(progress) = progress {
            let percent = completed
                .saturating_mul(100)
                .checked_div(total_functions)
                .unwrap_or(100);
            let _guard = progress_callback_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = reported_percent.load(Ordering::Relaxed);
            if percent > previous || completed == total_functions {
                reported_percent.store(percent, Ordering::Relaxed);
                progress(CompileProgress {
                    completed,
                    total: total_functions,
                });
            }
        }
        build
    };
    let compile_functions = || {
        #[cfg(not(target_arch = "wasm32"))]
        {
            project
                .program
                .functions
                .par_iter()
                .map(compile_one)
                .collect::<Vec<_>>()
        }
        #[cfg(target_arch = "wasm32")]
        {
            project
                .program
                .functions
                .iter()
                .map(compile_one)
                .collect::<Vec<_>>()
        }
    };
    // Cache hashing and lowering are both function-local. Running them in one
    // indexed parallel iterator preserves deterministic input order while
    // avoiding a serial hashing pass before worker threads can start lowering.
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(target_arch = "wasm32")]
    let function_builds = compile_functions();
    let mut materialized = Vec::with_capacity(function_builds.len());
    let mut lowered_count = 0usize;
    let mut diagnostics = Vec::new();
    for result in function_builds {
        let entry = match result {
            FunctionBuild::Cached(entry) => entry,
            FunctionBuild::Lowered(result) => {
                lowered_count += 1;
                diagnostics.extend(result.diagnostics.clone());
                materialized_function(result)
            }
        };
        materialized.push(entry);
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
    let source_entry_count = materialized
        .iter()
        .map(|entry| entry.source_entries.len())
        .sum();
    let mut lowered_source_entries = Vec::with_capacity(source_entry_count);
    let mut functions = Vec::with_capacity(materialized.len());
    let mut cached = BTreeMap::new();
    materialized.sort_by_key(|entry| entry.function.key);
    for entry in materialized {
        let key = entry.function.key;
        let cached_entry = if compact_cache {
            compact_cached_function(&entry)
        } else {
            cached_function(&entry)
        };
        for import in entry.native_imports {
            native_imports.insert(import.import.key, import);
        }
        for import in entry.host_imports {
            host_imports.insert(import.import.key, import);
        }
        lowered_source_entries.extend(entry.source_entries);
        functions.push(entry.function);
        cached.insert(key, cached_entry);
    }
    let mut fingerprint_order = lowered_source_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.statement_fingerprint, index))
        .collect::<Vec<_>>();
    fingerprint_order.sort_unstable();
    let mut statement_fingerprints = Vec::new();
    let mut fingerprint_indices = vec![0_u32; lowered_source_entries.len()];
    for (fingerprint, entry_index) in fingerprint_order {
        let fingerprint_index = if statement_fingerprints.last() == Some(&fingerprint) {
            statement_fingerprints.len().saturating_sub(1)
        } else {
            statement_fingerprints.push(fingerprint);
            statement_fingerprints.len().saturating_sub(1)
        };
        fingerprint_indices[entry_index] = u32::try_from(fingerprint_index).unwrap_or(u32::MAX);
    }
    let source_entries = lowered_source_entries
        .into_iter()
        .zip(fingerprint_indices)
        .map(|(entry, statement_fingerprint)| SourceMapEntry {
            function: entry.function,
            code_start: entry.code_start,
            code_end: entry.code_end,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            statement_fingerprint,
            origin_chain: entry.origin_chain,
            source_index: entry.source_index,
        })
        .collect();
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
        functions,
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
            statement_fingerprints,
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
    let patch =
        previous.and_then(|base| create_incremental_patch(base, previous_artifact, &artifact));
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
        base: Some(IncrementalBase::from_artifact(&artifact, !compact_cache)),
    };
    CompileReport {
        artifact: Some(artifact),
        patch,
        incremental_state,
        diagnostics,
        stats,
    }
}
