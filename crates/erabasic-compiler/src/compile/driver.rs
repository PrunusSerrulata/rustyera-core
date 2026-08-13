#[allow(clippy::wildcard_imports)]
use super::*;
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
    compile_project_inner(project, options, host_registry, previous, None, false, None).into()
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
    .into()
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
    .into()
}

/// Compile for a runtime that must preserve the compiler's validation provenance.
#[must_use]
pub fn compile_validated_project_with_artifact(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> ValidatedCompileReport {
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

/// Compile with progress while preserving validator provenance for the runtime.
#[must_use]
pub fn compile_validated_project_with_artifact_and_progress(
    project: &AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> ValidatedCompileReport {
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
) -> ValidatedCompileReport {
    let total_functions = project.program.functions.len();
    let compiling_progress =
        CompileProgressCounter::new(CompileProgressStage::Compiling, total_functions, progress);
    let hir_report = validate_hir(&project.program, &project.data);
    if !hir_report.is_valid() {
        return ValidatedCompileReport {
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
    let mut functions_by_id = DenseIdIndex::new(project.program.functions.len());
    for function in &project.program.functions {
        functions_by_id.insert(function.id.0, function);
    }
    let mut source_indices = DenseIdIndex::new(project.program.sources.len());
    for (index, source) in project.program.sources.iter().enumerate() {
        source_indices.insert(source.id.0, u32::try_from(index).unwrap_or(u32::MAX));
    }
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
    let compile_one = |function: &Function| {
        let build = {
            let key = *function_keys
                .get(function.id.0)
                .expect("validated function IDs have stable keys");
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
        compiling_progress.advance();
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
                return ValidatedCompileReport {
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
    compiling_progress.finish();
    let source_entry_count = function_builds
        .iter()
        .map(FunctionBuild::source_entry_count)
        .sum::<usize>();
    let source_entry_chunks = source_entry_count.div_ceil(65_536);
    let finalizing_total = total_functions
        .saturating_mul(2)
        .saturating_add(source_entry_chunks.saturating_mul(3))
        .saturating_add(8);
    let finalizing_progress =
        CompileProgressCounter::new(CompileProgressStage::Finalizing, finalizing_total, progress);
    let mut materialized = Vec::with_capacity(function_builds.len());
    let mut lowered_count = 0usize;
    let mut diagnostics = Vec::new();
    for result in function_builds {
        let entry = match result {
            FunctionBuild::Cached(entry) => entry,
            FunctionBuild::Lowered(mut result) => {
                lowered_count += 1;
                diagnostics.append(&mut result.diagnostics);
                materialized_function(result)
            }
        };
        materialized.push(entry);
        finalizing_progress.advance();
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
    finalizing_progress.checkpoint();
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::CompilerDiagnosticSeverity::Error)
    {
        return ValidatedCompileReport {
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

    let mut native_imports = Vec::<erabasic_bytecode::NativeImport>::new();
    let mut host_imports = Vec::<erabasic_bytecode::HostImport>::new();
    let mut native_import_indices = HashMap::new();
    let mut host_import_indices = HashMap::new();
    let source_entry_count = materialized
        .iter()
        .map(|entry| entry.source_entries.len())
        .sum();
    let mut lowered_source_entries = Vec::with_capacity(source_entry_count);
    let mut functions = Vec::with_capacity(materialized.len());
    // Function identities include a deterministic ordinal and are therefore
    // unique. Their canonical output order does not need stable sorting.
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    materialized.par_sort_unstable_by_key(|entry| entry.function.key);
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    materialized.sort_unstable_by_key(|entry| entry.function.key);
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    let cached_entries = materialized
        .par_iter()
        .map(|entry| {
            (
                entry.function.key,
                if compact_cache {
                    compact_cached_function(entry)
                } else {
                    cached_function(entry)
                },
            )
        })
        .collect::<Vec<_>>();
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    let cached_entries = materialized
        .iter()
        .map(|entry| {
            (
                entry.function.key,
                if compact_cache {
                    compact_cached_function(entry)
                } else {
                    cached_function(entry)
                },
            )
        })
        .collect::<Vec<_>>();
    let mut cached = BTreeMap::new();
    finalizing_progress.checkpoint();
    for (entry, (cached_key, cached_entry)) in materialized.into_iter().zip(cached_entries) {
        let key = entry.function.key;
        debug_assert_eq!(key, cached_key);
        for import in entry.native_imports {
            let key = import.import.key;
            if let Some(index) = native_import_indices.get(&key).copied() {
                native_imports[index] = import;
            } else {
                native_import_indices.insert(key, native_imports.len());
                native_imports.push(import);
            }
        }
        for import in entry.host_imports {
            let key = import.import.key;
            if let Some(index) = host_import_indices.get(&key).copied() {
                host_imports[index] = import;
            } else {
                host_import_indices.insert(key, host_imports.len());
                host_imports.push(import);
            }
        }
        lowered_source_entries.extend(entry.source_entries);
        functions.push(entry.function);
        cached.insert(key, cached_entry);
        finalizing_progress.advance();
    }
    native_imports.sort_unstable_by_key(|value| value.import.key);
    host_imports.sort_unstable_by_key(|value| value.import.key);
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    let mut fingerprint_order = lowered_source_entries
        .par_iter()
        .enumerate()
        .map(|(index, entry)| {
            debug_assert!(
                entry.statement_fingerprint.0[16..]
                    .iter()
                    .all(|byte| *byte == 0)
            );
            let mut prefix = [0; 16];
            prefix.copy_from_slice(&entry.statement_fingerprint.0[..16]);
            (prefix, index)
        })
        .collect::<Vec<_>>();
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    let mut fingerprint_order = lowered_source_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            debug_assert!(
                entry.statement_fingerprint.0[16..]
                    .iter()
                    .all(|byte| *byte == 0)
            );
            let mut prefix = [0; 16];
            prefix.copy_from_slice(&entry.statement_fingerprint.0[..16]);
            (prefix, index)
        })
        .collect::<Vec<_>>();
    finalizing_progress.checkpoint();
    // Equal fingerprints receive the same interned index, so their source-entry
    // order is irrelevant here; the original entry order is restored through the
    // index side table below.
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    fingerprint_order.par_sort_unstable_by_key(|entry| entry.0);
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    fingerprint_order.sort_unstable_by_key(|entry| entry.0);
    finalizing_progress.checkpoint();
    let mut statement_fingerprints = Vec::new();
    let mut fingerprint_indices = vec![0_u32; lowered_source_entries.len()];
    for chunk in fingerprint_order.chunks(65_536) {
        for &(prefix, entry_index) in chunk {
            let mut fingerprint = [0; 32];
            fingerprint[..16].copy_from_slice(&prefix);
            let fingerprint = Digest(fingerprint);
            let fingerprint_index = if statement_fingerprints.last() == Some(&fingerprint) {
                statement_fingerprints.len().saturating_sub(1)
            } else {
                statement_fingerprints.push(fingerprint);
                statement_fingerprints.len().saturating_sub(1)
            };
            fingerprint_indices[entry_index] = u32::try_from(fingerprint_index).unwrap_or(u32::MAX);
        }
        finalizing_progress.checkpoint();
    }
    drop(fingerprint_order);
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    let source_entries = lowered_source_entries
        .into_par_iter()
        .zip(fingerprint_indices.into_par_iter())
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
        .collect::<Vec<_>>();
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
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
        .collect::<Vec<_>>();
    finalizing_progress.checkpoint();
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(compiler_options),
        call_compatibility: erabasic_bytecode::BytecodeCallCompatibility {
            allow_event_as_normal: project.program.call_compatibility.allow_event_as_normal,
            allow_omitted_arguments: project.program.call_compatibility.allow_omitted_arguments,
            auto_convert_integer_to_string: project
                .program
                .call_compatibility
                .auto_convert_integer_to_string,
            allow_full_width_space: project.program.call_compatibility.allow_full_width_space,
            debug_semicolon: project.program.call_compatibility.debug_semicolon,
            ignore_triple_symbols: project.program.call_compatibility.ignore_triple_symbols,
        },
        project_data: project.data.clone(),
        globals: globals(&project.program.variables, &variable_keys, &function_keys),
        native_imports,
        host_imports,
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
    finalizing_progress.checkpoint();
    // Compiler output has no identity to verify yet. Validate its structure in
    // place, then serialize the complete artifact only once to assign final IDs.
    // Untrusted decoded artifacts continue to use the validator's identity-checking path.
    let validation_context = ValidationContext::for_artifact(&artifact);
    let validation = validate_compiler_output(artifact, &validation_context);
    finalizing_progress.checkpoint();
    if !validation.is_valid() {
        return ValidatedCompileReport {
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
    let artifact = validation
        .value
        .expect("a valid compiler artifact is returned by the validator")
        .refresh_ids();
    let artifact = match artifact {
        Ok(artifact) => artifact,
        Err(error) => {
            return ValidatedCompileReport {
                artifact: None,
                patch: None,
                incremental_state: previous.cloned().unwrap_or_default(),
                diagnostics: vec![CompilerDiagnostic::new(
                    CompilerDiagnosticCode::Encoding,
                    error,
                )],
                stats: CompileStats::default(),
            };
        }
    };
    finalizing_progress.checkpoint();
    let patch = previous
        .and_then(|base| create_incremental_patch(base, previous_artifact, artifact.artifact()));
    finalizing_progress.checkpoint();
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
        base: Some(IncrementalBase::from_artifact(
            artifact.artifact(),
            !compact_cache,
        )),
    };
    finalizing_progress.finish();
    ValidatedCompileReport {
        artifact: Some(artifact),
        patch,
        incremental_state,
        diagnostics,
        stats,
    }
}
