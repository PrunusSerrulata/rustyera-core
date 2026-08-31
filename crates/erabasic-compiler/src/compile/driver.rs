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
    compile_project_inner(
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        None,
        CompilePolicy {
            compact_cache: false,
            consume_owned_hir: false,
        },
        None,
    )
    .report
    .into()
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
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: false,
        },
        None,
    )
    .report
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
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: false,
        },
        Some(progress),
    )
    .report
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
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: false,
        },
        None,
    )
    .report
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
        ProjectInput::Borrowed(project),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: false,
        },
        Some(progress),
    )
    .report
}

/// Compile an owned analyzed project while moving its large data tables into the artifact.
///
/// Runtime cold loads use this path after analyzer diagnostics no longer need the HIR owner.
#[must_use]
pub fn compile_owned_validated_project_with_artifact(
    project: AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
) -> OwnedValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Owned(Box::new(project)),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: cfg!(target_arch = "wasm32"),
        },
        None,
    )
}

/// Compile an owned analyzed project with progress and without cloning artifact-owned tables.
#[must_use]
pub fn compile_owned_validated_project_with_artifact_and_progress(
    project: AnalyzedProject,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    progress: &dyn CompileProgressCallback,
) -> OwnedValidatedCompileReport {
    compile_project_inner(
        ProjectInput::Owned(Box::new(project)),
        options,
        host_registry,
        previous,
        previous_artifact,
        CompilePolicy {
            compact_cache: true,
            consume_owned_hir: cfg!(target_arch = "wasm32"),
        },
        Some(progress),
    )
}

enum ProjectInput<'a> {
    Borrowed(&'a AnalyzedProject),
    Owned(Box<AnalyzedProject>),
}

#[derive(Clone, Copy)]
struct CompilePolicy {
    compact_cache: bool,
    consume_owned_hir: bool,
}

impl ProjectInput<'_> {
    fn project(&self) -> &AnalyzedProject {
        match self {
            Self::Borrowed(project) => project,
            Self::Owned(project) => project,
        }
    }

    fn take_owned_functions(&mut self, consume: bool) -> Option<Vec<Function>> {
        if !consume {
            return None;
        }
        match self {
            Self::Borrowed(_) => None,
            Self::Owned(project) => Some(std::mem::take(&mut project.program.functions)),
        }
    }

    fn into_diagnostic_sources(self) -> (Vec<SourceId>, Vec<SourceRecord>) {
        match self {
            Self::Borrowed(_) => (Vec::new(), Vec::new()),
            Self::Owned(project) => source_records(project.program.sources),
        }
    }

    fn into_artifact_parts(self) -> (erabasic_data::ProjectData, Vec<SourceId>, Vec<SourceRecord>) {
        match self {
            Self::Borrowed(project) => (
                project.data.clone(),
                project
                    .program
                    .sources
                    .iter()
                    .map(|source| source.id)
                    .collect(),
                project.program.sources.iter().map(source_record).collect(),
            ),
            Self::Owned(project) => {
                let project = *project;
                let (source_ids, sources) = source_records(project.program.sources);
                (project.data, source_ids, sources)
            }
        }
    }
}

fn source_records(sources: Vec<erabasic_hir::SourceFile>) -> (Vec<SourceId>, Vec<SourceRecord>) {
    let mut source_ids = Vec::with_capacity(sources.len());
    let mut records = Vec::with_capacity(sources.len());
    for source in sources {
        source_ids.push(source.id);
        records.push(SourceRecord {
            relative_path: source.relative_path,
            content_hash: Digest(source.content_hash),
            byte_len: source.byte_len,
            line_starts: source.line_starts,
        });
    }
    (source_ids, records)
}

fn source_record(source: &erabasic_hir::SourceFile) -> SourceRecord {
    SourceRecord {
        relative_path: source.relative_path.clone(),
        content_hash: Digest(source.content_hash),
        byte_len: source.byte_len,
        line_starts: source.line_starts.clone(),
    }
}

#[allow(clippy::too_many_lines)]
fn compile_project_inner(
    project: ProjectInput<'_>,
    options: &CompilerOptions,
    host_registry: &HostRegistry,
    previous: Option<&IncrementalState>,
    previous_artifact: Option<&BytecodeArtifact>,
    policy: CompilePolicy,
    progress: Option<&dyn CompileProgressCallback>,
) -> OwnedValidatedCompileReport {
    let CompilePolicy {
        compact_cache,
        consume_owned_hir,
    } = policy;
    let mut project = project;
    let (total_functions, total_variables, total_sources) = {
        let program = &project.project().program;
        (
            program.functions.len(),
            program.variables.len(),
            program.sources.len(),
        )
    };
    // Compiling includes stable-key/signature indexing and call-dependency preparation before
    // bytecode lowering. Count those real work units so large projects keep reporting progress
    // instead of appearing stalled before the first function body is emitted.
    let total_compile_work = total_functions
        .saturating_mul(5)
        .saturating_add(total_variables.saturating_mul(2))
        .saturating_add(total_sources);
    let compiling_progress = CompileProgressCounter::new(
        CompileProgressStage::Compiling,
        total_compile_work,
        progress,
    );
    let hir_report = {
        let project_ref = project.project();
        validate_hir(&project_ref.program, &project_ref.data)
    };
    if !hir_report.is_valid() {
        let (source_ids, diagnostic_sources) = project.into_diagnostic_sources();
        return OwnedValidatedCompileReport {
            source_ids,
            diagnostic_sources,
            report: ValidatedCompileReport {
                artifact: None,
                patch: None,
                incremental_state: previous.cloned().unwrap_or_default(),
                diagnostics: hir_report
                    .diagnostics
                    .into_iter()
                    .map(|diagnostic| {
                        CompilerDiagnostic::new(
                            CompilerDiagnosticCode::InvalidHir,
                            diagnostic.message,
                        )
                    })
                    .collect(),
                stats: CompileStats::default(),
            },
        };
    }

    let compiler_options = canonical_digest("rustyera.compiler.options.v2", &options.optimization);
    let (function_keys, variable_keys, function_signatures, artifact_event_groups) = {
        let project_ref = project.project();
        let function_keys = function_keys(
            &project_ref.program.functions,
            &project_ref.program.sources,
            || compiling_progress.advance(),
        );
        let variable_keys = variable_keys(&project_ref.program.variables, &function_keys, || {
            compiling_progress.advance();
        });
        let mut function_signatures = Vec::with_capacity(total_functions);
        for function in &project_ref.program.functions {
            function_signatures.push(FunctionSignature::from(function));
            compiling_progress.advance();
        }
        let artifact_event_groups = event_groups(&project_ref.program.functions, &function_keys);
        (
            function_keys,
            variable_keys,
            function_signatures,
            artifact_event_groups,
        )
    };
    let owned_functions = project.take_owned_functions(consume_owned_hir);
    let project_ref = project.project();
    let (function_builds, previous_artifact) = {
        let mut functions_by_id = DenseIdIndex::new(function_signatures.len());
        for function in &function_signatures {
            functions_by_id.insert(function.id.0, function);
            compiling_progress.advance();
        }
        let mut source_indices = DenseIdIndex::new(project_ref.program.sources.len());
        for (index, source) in project_ref.program.sources.iter().enumerate() {
            source_indices.insert(source.id.0, u32::try_from(index).unwrap_or(u32::MAX));
            compiling_progress.advance();
        }
        let context = LoweringContext {
            program: LoweringProgram {
                variables: &project_ref.program.variables,
                snake_input: project_ref.program.compatibility.supports_snake_input(),
                call_compatibility: project_ref.program.call_compatibility,
            },
            function_keys: &function_keys,
            functions_by_id: &functions_by_id,
            variable_keys: &variable_keys,
            source_indices: &source_indices,
            host_registry,
        };
        let call_dependencies = super::call_dependencies::CallDependencies::new(
            &function_signatures,
            &function_keys,
            &project_ref.program.variables,
            || compiling_progress.advance(),
        );
        let variable_dependencies =
            shared_variable_dependencies(&project_ref.program.variables, || {
                compiling_progress.advance();
            });
        let shared_dependencies = canonical_digest(
            "rustyera.compiler.shared-dependencies.v5",
            &(
                &project_ref.program.compatibility,
                &project_ref.program.call_compatibility,
                variable_dependencies,
                host_registry,
                options.optimization,
            ),
        );
        let previous_functions = previous
            .filter(|state| state.compiler_abi == erabasic_bytecode::COMPILER_ABI_VERSION)
            .map(|state| &state.functions);
        let previous_artifact = previous_artifact.filter(|artifact| {
            artifact.manifest.compatibility == project_ref.program.compatibility
                && previous.and_then(IncrementalState::base_artifact_id)
                    == Some(artifact.manifest.artifact_id)
        });
        let previous_artifact_index = previous_artifact.map(PreviousArtifactIndex::new);
        let compile_one = |function: &Function| {
            let build = {
                let key = *function_keys
                    .get(function.id.0)
                    .expect("validated function IDs have stable keys");
                let function_digest =
                    canonical_digest("rustyera.compiler.hir-function.v3", function);
                let signature_dependencies = call_dependencies.for_function(function);
                let cache_key = Digest::hash(
                    "rustyera.compiler.function.v4",
                    &[
                        &function_digest.0,
                        &signature_dependencies.0,
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
        let function_builds = if let Some(functions) = owned_functions {
            // A constrained WASM cold build no longer needs a complete HIR function graph after
            // validation and signature extraction. Move and lower one function at a time so its
            // statements are released before the next bytecode body is accumulated.
            Ok(functions
                .into_iter()
                .map(|function| compile_one(&function))
                .collect::<Vec<_>>())
        } else {
            let compile_functions = || {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    project_ref
                        .program
                        .functions
                        .par_iter()
                        .map(compile_one)
                        .collect::<Vec<_>>()
                }
                #[cfg(target_arch = "wasm32")]
                {
                    project_ref
                        .program
                        .functions
                        .iter()
                        .map(compile_one)
                        .collect::<Vec<_>>()
                }
            };
            // Cache hashing and lowering are both function-local. Running them in
            // one indexed parallel iterator preserves deterministic input order.
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(jobs) = options.jobs {
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(jobs.max(1))
                        .build()
                        .map(|pool| pool.install(compile_functions))
                        .map_err(|error| error.to_string())
                } else {
                    Ok(compile_functions())
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                Ok::<_, String>(compile_functions())
            }
        };
        (function_builds, previous_artifact)
    };
    let function_builds = match function_builds {
        Ok(function_builds) => function_builds,
        Err(error) => {
            let (source_ids, diagnostic_sources) = project.into_diagnostic_sources();
            return OwnedValidatedCompileReport {
                source_ids,
                diagnostic_sources,
                report: ValidatedCompileReport {
                    artifact: None,
                    patch: None,
                    incremental_state: previous.cloned().unwrap_or_default(),
                    diagnostics: vec![CompilerDiagnostic::new(
                        CompilerDiagnosticCode::Parallelism,
                        error,
                    )],
                    stats: CompileStats::default(),
                },
            };
        }
    };
    compiling_progress.finish();
    let source_entry_count = function_builds
        .iter()
        .map(FunctionBuild::source_entry_count)
        .sum::<usize>();
    let source_entry_chunks = source_entry_count.div_ceil(65_536);
    let finalizing_total = total_functions
        .saturating_mul(2)
        .saturating_add(source_entry_chunks)
        .saturating_add(9);
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
        let (source_ids, diagnostic_sources) = project.into_diagnostic_sources();
        return OwnedValidatedCompileReport {
            source_ids,
            diagnostic_sources,
            report: ValidatedCompileReport {
                artifact: None,
                patch: None,
                incremental_state: previous.cloned().unwrap_or_default(),
                diagnostics,
                stats: CompileStats {
                    total_functions,
                    compiled_functions: lowered_count,
                    reused_functions: total_functions - lowered_count,
                    patch_functions: 0,
                },
            },
        };
    }

    // Everything below operates on bytecode-owned data. Derive the few HIR
    // summaries still needed by the artifact and then consume the analyzed
    // project so its function/variable graphs are released before source-map
    // interning and artifact hashing reach their peak.
    let call_compatibility = erabasic_bytecode::BytecodeCallCompatibility {
        user_argument_policy: project_ref.program.call_compatibility.user_argument_policy,
        allow_event_as_normal: project_ref.program.call_compatibility.allow_event_as_normal,
        allow_omitted_arguments: project_ref
            .program
            .call_compatibility
            .allow_omitted_arguments,
        auto_convert_integer_to_string: project_ref
            .program
            .call_compatibility
            .auto_convert_integer_to_string,
        allow_full_width_space: project_ref
            .program
            .call_compatibility
            .allow_full_width_space,
        debug_semicolon: project_ref.program.call_compatibility.debug_semicolon,
        ignore_triple_symbols: project_ref.program.call_compatibility.ignore_triple_symbols,
        compatible_rand: project_ref.program.call_compatibility.compatible_rand,
        system_no_target: project_ref.program.call_compatibility.system_no_target,
        ignore_case: project_ref.program.call_compatibility.ignore_case,
        before_error_throw_hooks: project_ref
            .program
            .call_compatibility
            .before_error_throw_hooks,
    };
    let runtime_variables = super::runtime_symbols::runtime_variable_symbols(
        &project_ref.program.variables,
        &variable_keys,
    );
    let artifact_globals = globals(
        &project_ref.program.variables,
        &variable_keys,
        &function_keys,
    );
    let compatibility = project.project().program.compatibility.clone();
    let (project_data, source_ids, project_sources) = project.into_artifact_parts();
    drop(variable_keys);
    drop(function_keys);

    let mut native_imports = Vec::<erabasic_bytecode::NativeImport>::new();
    let mut host_imports = Vec::<erabasic_bytecode::HostImport>::new();
    let mut native_import_indices = HashMap::new();
    let mut host_import_indices = HashMap::new();
    let source_entry_count = materialized
        .iter()
        .map(|entry| entry.source_entries.len())
        .sum();
    let mut source_entries = Vec::with_capacity(source_entry_count);
    let mut fingerprint_prefixes = Vec::with_capacity(source_entry_count);
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
        for entry in entry.source_entries {
            debug_assert!(
                entry.statement_fingerprint.0[16..]
                    .iter()
                    .all(|byte| *byte == 0)
            );
            let mut prefix = [0; 16];
            prefix.copy_from_slice(&entry.statement_fingerprint.0[..16]);
            fingerprint_prefixes.push(prefix);
            source_entries.push(SourceMapEntry {
                function: entry.function,
                code_start: entry.code_start,
                code_end: entry.code_end,
                byte_start: entry.byte_start,
                byte_end: entry.byte_end,
                statement_fingerprint: 0,
                origin_chain: entry.origin_chain,
                source_index: entry.source_index,
            });
        }
        functions.push(entry.function);
        cached.insert(key, cached_entry);
        finalizing_progress.advance();
    }
    drop(native_import_indices);
    drop(host_import_indices);
    if compatibility.supports_snake_input() {
        for import in super::runtime_symbols::runtime_input_imports(host_registry) {
            if !host_imports
                .iter()
                .any(|existing| existing.import.key == import.import.key)
            {
                host_imports.push(import);
            }
        }
    }
    native_imports.sort_unstable_by_key(|value| value.import.key);
    host_imports.sort_unstable_by_key(|value| value.import.key);
    assert!(
        u32::try_from(source_entries.len()).is_ok(),
        "source-map entry count exceeds the artifact format"
    );
    let mut fingerprint_order = (0..source_entries.len())
        .map(|index| u32::try_from(index).expect("source-map length checked above"))
        .collect::<Vec<_>>();
    finalizing_progress.checkpoint();
    // Equal fingerprints receive the same interned index, so their source-entry
    // order is irrelevant here; the original entry order is restored through the
    // index side table below.
    #[cfg(any(not(target_arch = "wasm32"), target_feature = "atomics"))]
    fingerprint_order.par_sort_unstable_by_key(|entry| fingerprint_prefixes[*entry as usize]);
    #[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
    fingerprint_order.sort_unstable_by_key(|entry| fingerprint_prefixes[*entry as usize]);
    finalizing_progress.checkpoint();
    let mut statement_fingerprints = Vec::new();
    for chunk in fingerprint_order.chunks(65_536) {
        for &entry_index in chunk {
            let prefix = fingerprint_prefixes[entry_index as usize];
            let mut fingerprint = [0; 32];
            fingerprint[..16].copy_from_slice(&prefix);
            let fingerprint = Digest(fingerprint);
            let fingerprint_index = if statement_fingerprints.last() == Some(&fingerprint) {
                statement_fingerprints.len().saturating_sub(1)
            } else {
                statement_fingerprints.push(fingerprint);
                statement_fingerprints.len().saturating_sub(1)
            };
            source_entries[entry_index as usize].statement_fingerprint =
                u32::try_from(fingerprint_index).unwrap_or(u32::MAX);
        }
        finalizing_progress.checkpoint();
    }
    drop(fingerprint_order);
    drop(fingerprint_prefixes);
    finalizing_progress.checkpoint();
    let mut expression_signatures = erabasic_analyzer::builtin_function_signatures(&compatibility);
    for signature in &mut expression_signatures {
        if signature.name == "EXISTVAR" && compatibility.supports_existvar_expression_probe() {
            signature.arguments = vec![
                erabasic_analyzer::ArgumentConstraint::String,
                erabasic_analyzer::ArgumentConstraint::Integer,
            ];
        }
    }
    let runtime_builtins = super::runtime_symbols::runtime_builtin_symbols(expression_signatures);
    let runtime_native_authorizations =
        super::runtime_symbols::runtime_native_authorizations(&runtime_builtins, host_registry);
    let runtime_staged_authorizations =
        super::runtime_symbols::runtime_staged_authorizations(&runtime_builtins, host_registry);
    let runtime_host_authorizations = super::runtime_symbols::runtime_host_authorizations(
        &runtime_builtins,
        host_registry,
        &compatibility,
    );
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest {
            compatibility,
            ..ArtifactManifest::new(compiler_options)
        },
        call_compatibility,
        runtime_builtins,
        runtime_variables,
        runtime_native_authorizations,
        runtime_host_authorizations,
        runtime_staged_authorizations,
        project_data,
        globals: artifact_globals,
        native_imports,
        host_imports,
        functions,
        event_groups: artifact_event_groups,
        source_map: SourceMap {
            sources: project_sources,
            statement_fingerprints,
            entries: source_entries,
        },
    };
    finalizing_progress.checkpoint();
    // Compiler output has no identity to verify yet. Validate its structure in
    // place, then serialize the complete artifact only once to assign final IDs.
    // Untrusted decoded artifacts continue to use the validator's identity-checking path.
    let validation_context = super::runtime_native_validation_context(&artifact, host_registry);
    let validation = validate_compiler_output(artifact, &validation_context);
    finalizing_progress.checkpoint();
    if !validation.is_valid() {
        return OwnedValidatedCompileReport {
            source_ids: Vec::new(),
            diagnostic_sources: Vec::new(),
            report: ValidatedCompileReport {
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
            },
        };
    }
    let artifact = validation
        .value
        .expect("a valid compiler artifact is returned by the validator")
        .refresh_ids();
    let artifact = match artifact {
        Ok(artifact) => artifact,
        Err(error) => {
            return OwnedValidatedCompileReport {
                source_ids: Vec::new(),
                diagnostic_sources: Vec::new(),
                report: ValidatedCompileReport {
                    artifact: None,
                    patch: None,
                    incremental_state: previous.cloned().unwrap_or_default(),
                    diagnostics: vec![CompilerDiagnostic::new(
                        CompilerDiagnosticCode::Encoding,
                        error,
                    )],
                    stats: CompileStats::default(),
                },
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
        total_functions,
        compiled_functions: lowered_count,
        reused_functions: total_functions - lowered_count,
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
    OwnedValidatedCompileReport {
        source_ids,
        diagnostic_sources: Vec::new(),
        report: ValidatedCompileReport {
            artifact: Some(artifact),
            patch,
            incremental_state,
            diagnostics,
            stats,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_host_registry;
    use erabasic_analyzer::{
        AnalysisInput, AnalyzerOptions, ExtensionRegistry, ProjectSource, SourcePayload,
        analyze_project,
    };
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};

    fn analyzed(source: &str) -> AnalyzedProject {
        let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
            .data
            .expect("default project data");
        let report = analyze_project(
            AnalysisInput {
                project_data,
                sources: vec![ProjectSource {
                    relative_path: "main.erb".into(),
                    payload: SourcePayload::Utf8(source.into()),
                }],
            },
            &AnalyzerOptions::analysis_mode(),
            &ExtensionRegistry::default(),
        );
        assert!(report.project.is_some(), "{:#?}", report.diagnostics);
        report.project.expect("analyzed project")
    }

    #[test]
    fn consumed_owned_hir_matches_the_borrowed_compile() {
        let source = "@SYSTEM_TITLE\nCALL HELPER\nRETURN\n\
                      @HELPER(ARG = 2)\nRESULT = ARG\nRETURN\n";
        let borrowed_project = analyzed(source);
        let expected = compile_validated_project_with_artifact(
            &borrowed_project,
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
            None,
        );
        let consumed = compile_project_inner(
            ProjectInput::Owned(Box::new(analyzed(source))),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
            None,
            CompilePolicy {
                compact_cache: true,
                consume_owned_hir: true,
            },
            None,
        );

        assert_eq!(consumed.report, expected);
        assert_eq!(consumed.source_ids, [SourceId(0)]);
        assert!(consumed.diagnostic_sources.is_empty());
    }

    #[test]
    fn compilation_preparation_reports_intermediate_work() {
        let events = std::sync::Mutex::new(Vec::new());
        let callback = |progress| events.lock().unwrap().push(progress);
        let project = analyzed("@SYSTEM_TITLE\nCALL HELPER\nRETURN\n@HELPER\nRETURN\n");
        let expected_work = project
            .program
            .functions
            .len()
            .saturating_mul(5)
            .saturating_add(project.program.variables.len().saturating_mul(2))
            .saturating_add(project.program.sources.len());
        let report = compile_project_inner(
            ProjectInput::Owned(Box::new(project)),
            &CompilerOptions::default(),
            &default_host_registry(),
            None,
            None,
            CompilePolicy {
                compact_cache: true,
                consume_owned_hir: true,
            },
            Some(&callback),
        );

        assert!(
            report.report.artifact.is_some(),
            "{:#?}",
            report.report.diagnostics
        );
        let events = events.into_inner().unwrap();
        let compiling = events
            .iter()
            .filter(|progress| progress.stage == CompileProgressStage::Compiling)
            .collect::<Vec<_>>();
        assert_eq!(compiling.first().unwrap().completed, 0);
        assert_eq!(compiling.last().unwrap().total, expected_work);
        assert_eq!(
            compiling.last().unwrap().completed,
            compiling.last().unwrap().total
        );
        assert!(
            compiling
                .iter()
                .any(|progress| progress.completed > 0 && progress.completed < progress.total),
            "compilation preparation did not expose intermediate progress: {compiling:?}"
        );
    }
}
