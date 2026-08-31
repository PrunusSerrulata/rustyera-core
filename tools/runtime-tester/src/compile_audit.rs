use super::*;

pub(super) fn supervise() -> Result<(), Box<dyn std::error::Error>> {
    if env::var_os("ERA_AUDIT_BUDGET_SECONDS").is_none() {
        return Err(
            "compile requires explicit ERA_AUDIT_BUDGET_SECONDS; the 3600-second default is disabled for this audit"
                .into(),
        );
    }
    watchdog::supervise("compile", audit_compile)
}

fn audit_compile() -> Result<(), Box<dyn std::error::Error>> {
    run(true)
}

fn frozen_snake_csv_options(
    compatibility: &erabasic_compat::CompatibilityIdentity,
) -> erabasic_csv::CsvLoadOptions {
    erabasic_csv::CsvLoadOptions {
        compatibility: compatibility.clone(),
        ignore_case: true,
        use_rename_file: true,
        use_replace_file: true,
        search_subdirectories: true,
        sort_with_filename: true,
        compatible_call_name: true,
        compatible_sp_character: false,
        use_erd: true,
        debug_mode: false,
        allow_full_width_space: false,
        continuation_separator: " ".into(),
        current_emuera_version: "1.824.0.0".into(),
    }
}

fn frozen_snake_analyzer_options(
    compatibility: &erabasic_compat::CompatibilityIdentity,
) -> erabasic_analyzer::AnalyzerOptions {
    erabasic_analyzer::AnalyzerOptions {
        compatibility: compatibility.clone(),
        ignore_case: true,
        sort_with_filename: true,
        allow_function_overloading: true,
        warn_function_overloading: true,
        display_warning_level: 0,
        ignore_uncalled_functions: true,
        function_not_found: erabasic_analyzer::WarningPolicy::Display,
        function_not_called: erabasic_analyzer::WarningPolicy::OncePerFile,
        compatible_function_argument_auto_convert: true,
        compatible_function_argument_optional: false,
        strict_user_call_arguments: false,
        disable_before_error_throw: true,
        compatible_call_event: false,
        system_save_in_binary: true,
        use_erd: true,
        varsize_dimension_is_one_based: false,
        default_foreground_color: 0x00c0_c0c0,
        analysis_mode: false,
        debug_mode: false,
        allow_full_width_space: false,
        debug_semicolon: false,
        ignore_triple_symbols: false,
        compatible_rand: false,
        system_no_target: false,
        continuation_separator: " ".into(),
    }
}

struct CompileAuditObservation {
    project: String,
    profile: String,
    configuration_digest: String,
    input_identity: String,
    input_count: usize,
}

impl CompileAuditObservation {
    fn new(
        project: &Path,
        compatibility: &erabasic_compat::CompatibilityIdentity,
        csv: &erabasic_csv::CsvLoadOptions,
        analyzer: &erabasic_analyzer::AnalyzerOptions,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let configuration = serde_json::to_vec(&(csv, analyzer))?;
        Ok(Self {
            project: project.to_string_lossy().into_owned(),
            profile: compatibility.profile.to_string(),
            configuration_digest: blake3::hash(&configuration).to_hex().to_string(),
            input_identity: "inventory_incomplete".into(),
            input_count: 0,
        })
    }

    fn bind_paths(&mut self, paths: &[String]) {
        let mut identity = blake3::Hasher::new();
        for path in paths {
            identity.update(path.as_bytes());
            identity.update(&[0]);
        }
        self.input_identity = identity.finalize().to_hex().to_string();
        self.input_count = paths.len();
    }

    fn bind_sources(&mut self, digest: blake3::Hash) {
        self.input_identity = digest.to_hex().to_string();
    }

    fn publish(
        &self,
        phase: &str,
        pending: serde_json::Value,
        completed: usize,
        total: usize,
        first_diagnostic: serde_json::Value,
        results: serde_json::Value,
    ) {
        watchdog::publish_or_exit(serde_json::json!({
            "phase": phase,
            "pending": pending,
            "completed": completed,
            "total": total,
            "project": self.project,
            "profile": self.profile,
            "configurationDigest": self.configuration_digest,
            "inputIdentity": self.input_identity,
            "inputCount": self.input_count,
            "firstDiagnostic": first_diagnostic,
            "results": results,
            "lastFullResponse": null
        }));
    }

    fn fail(
        &self,
        stage: &str,
        diagnostics: serde_json::Value,
        message: impl Into<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let message = message.into();
        let diagnostics = diagnostic_batch(diagnostics);
        let first_diagnostic = diagnostics.first().cloned().unwrap_or_default();
        self.publish(
            "failed",
            serde_json::json!({"stage": stage}),
            0,
            0,
            first_diagnostic,
            serde_json::json!({
                "failure": &message,
                "errorCount": diagnostics.len(),
                "diagnostics": diagnostics
            }),
        );
        Err(message.into())
    }
}

fn diagnostic_batch(diagnostics: serde_json::Value) -> Vec<serde_json::Value> {
    match diagnostics {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(diagnostics) => diagnostics,
        diagnostic => vec![diagnostic],
    }
}

pub(super) fn run(compile: bool) -> Result<(), Box<dyn std::error::Error>> {
    let total_started = std::time::Instant::now();
    let root = project_argument(2);
    let compatibility = env::args().nth(3).map_or_else(
        || Ok(erabasic_compat::CompatibilityIdentity::default()),
        |profile| {
            profile
                .parse::<erabasic_compat::CompatibilityProfileId>()
                .map(erabasic_compat::CompatibilityIdentity::for_profile)
        },
    )?;
    let csv_options = frozen_snake_csv_options(&compatibility);
    let options = frozen_snake_analyzer_options(&compatibility);
    let mut observation =
        CompileAuditObservation::new(&root, &compatibility, &csv_options, &options)?;
    observation.publish(
        "inventory_directory",
        serde_json::json!({"path": root}),
        0,
        0,
        serde_json::Value::Null,
        serde_json::json!({}),
    );
    let paths = match try_collect_project_files(&root, &mut |path, completed| {
        observation.publish(
            "inventory_directory",
            serde_json::json!({"path": path}),
            completed,
            0,
            serde_json::Value::Null,
            serde_json::json!({}),
        );
    }) {
        Ok(paths) => paths,
        Err(error) => {
            return observation.fail(
                "inventory_directory",
                serde_json::json!({
                    "code": "project_inventory",
                    "path": root,
                    "message": error.to_string()
                }),
                format!("cannot inventory project {}: {error}", root.display()),
            );
        }
    };
    observation.bind_paths(&paths);
    let inputs = project_inputs::ProjectInputs::new(&root, &paths);
    let mut csv_files = erabasic_csv::ProjectFiles::default();
    let mut sources = Vec::new();
    let mut source_identity = blake3::Hasher::new();
    for (index, relative) in paths.iter().enumerate() {
        observation.publish(
            "coverage_read_input",
            serde_json::json!({"path": relative}),
            index,
            paths.len(),
            serde_json::Value::Null,
            serde_json::json!({}),
        );
        let Some(category) = inputs.classify(relative) else {
            continue;
        };
        let data_root = inputs.data_root(relative, category);
        if data_root.is_none() && !matches!(category, FileCategory::Erb | FileCategory::Erh) {
            continue;
        }
        let text = match read_submitted_text(root.join(relative), category) {
            Ok(text) => text,
            Err(error) => {
                return observation.fail(
                    "read_input",
                    serde_json::json!({
                        "code": "project_input_io",
                        "path": relative,
                        "message": error.to_string()
                    }),
                    format!("cannot read project input {relative}: {error}"),
                );
            }
        };
        source_identity.update(format!("{category:?}").as_bytes());
        source_identity.update(&[0]);
        source_identity.update(relative.as_bytes());
        source_identity.update(&[0]);
        source_identity.update(text.as_bytes());
        source_identity.update(&[0]);
        if let Some(data_root) = data_root {
            let file = erabasic_csv::FrontendFile {
                relative_path: data_root.relative_path(relative),
                source_path: Some(relative.clone()),
                payload: erabasic_csv::FilePayload::Utf8(text),
            };
            match data_root {
                project_inputs::DataRoot::Csv => csv_files.csv.push(file),
                project_inputs::DataRoot::Erb => csv_files.erb.push(file),
            }
        } else {
            sources.push(erabasic_analyzer::ProjectSource {
                relative_path: relative.clone(),
                payload: erabasic_analyzer::SourcePayload::Utf8(text),
            });
        }
    }
    observation.bind_sources(source_identity.finalize());
    observation.publish(
        "coverage_read_input",
        serde_json::Value::Null,
        paths.len(),
        paths.len(),
        serde_json::Value::Null,
        serde_json::json!({
            "csvInputs": csv_files.csv.len() + csv_files.erb.len(),
            "sourceInputs": sources.len()
        }),
    );
    let files_elapsed = total_started.elapsed();
    let csv_started = std::time::Instant::now();
    observation.publish(
        "csv_load",
        "load_project".into(),
        0,
        csv_files.csv.len() + csv_files.erb.len(),
        serde_json::Value::Null,
        serde_json::json!({}),
    );
    let csv = erabasic_csv::load_project(&csv_files, &csv_options);
    let csv_elapsed = csv_started.elapsed();
    println!("csv_diagnostics={}", csv.diagnostics.len());
    let csv_errors = csv
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                erabasic_csv::CsvDiagnosticSeverity::Error
                    | erabasic_csv::CsvDiagnosticSeverity::Fatal
            )
        })
        .count();
    if csv_errors != 0 {
        let errors = csv
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.severity,
                    erabasic_csv::CsvDiagnosticSeverity::Error
                        | erabasic_csv::CsvDiagnosticSeverity::Fatal
                )
            })
            .collect::<Vec<_>>();
        return observation.fail(
            "csv_load",
            serde_json::to_value(errors)?,
            format!("CSV loading reported {csv_errors} errors"),
        );
    }
    let Some(project_data) = csv.data else {
        return observation.fail(
            "csv_load",
            serde_json::json!({"code": "missing_project_data"}),
            "CSV loading did not produce project data",
        );
    };
    observation.publish(
        "csv_load",
        serde_json::Value::Null,
        csv_files.csv.len() + csv_files.erb.len(),
        csv_files.csv.len() + csv_files.erb.len(),
        serde_json::Value::Null,
        serde_json::json!({"diagnostics": csv.diagnostics.len(), "errors": 0}),
    );
    let analyze_started = std::time::Instant::now();
    let source_count = sources.len();
    observation.publish(
        "analysis",
        "analyze_project".into(),
        0,
        source_count,
        serde_json::Value::Null,
        serde_json::json!({}),
    );
    let progress = |progress: erabasic_analyzer::AnalysisProgress| {
        observation.publish(
            "analysis",
            serde_json::json!({
                "operation": "analyze_project",
                "projectProgress": {
                "stage": format!("{:?}", progress.stage),
                "completed": progress.completed,
                "total": progress.total
                }
            }),
            progress.completed,
            progress.total,
            serde_json::Value::Null,
            serde_json::json!({}),
        );
    };
    let report = erabasic_analyzer::analyze_project_with_progress(
        erabasic_analyzer::AnalysisInput {
            project_data,
            sources,
        },
        &options,
        &Default::default(),
        &progress,
    );
    let analyze_elapsed = analyze_started.elapsed();
    observation.publish(
        "analysis_returned",
        serde_json::Value::Null,
        source_count,
        source_count,
        serde_json::Value::Null,
        serde_json::json!({
            "diagnostics": report.diagnostics.len(),
            "project": report.project.is_some()
        }),
    );
    println!(
        "files_ms={} csv_ms={} analyze_ms={}",
        files_elapsed.as_millis(),
        csv_elapsed.as_millis(),
        analyze_elapsed.as_millis()
    );
    let errors = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                erabasic_analyzer::AnalyzerDiagnosticSeverity::Error
                    | erabasic_analyzer::AnalyzerDiagnosticSeverity::Fatal
            )
        })
        .collect::<Vec<_>>();
    let mut by_code = std::collections::BTreeMap::new();
    for diagnostic in &errors {
        *by_code
            .entry(format!("{:?}", diagnostic.code))
            .or_insert(0usize) += 1;
    }
    println!(
        "analyzer_diagnostics={} errors={} by_code={by_code:?}",
        report.diagnostics.len(),
        errors.len()
    );
    for diagnostic in errors.iter().take(300) {
        let source = diagnostic.source.as_ref();
        println!(
            "{:?}\t{}:{}\t{}",
            diagnostic.code,
            source.map_or("", |s| s.relative_path.as_str()),
            source.map_or(0, |s| s.physical_line),
            diagnostic.message
        );
    }
    if !errors.is_empty() {
        return observation.fail(
            "analysis",
            serde_json::to_value(&errors)?,
            format!("analysis reported {} errors", errors.len()),
        );
    }
    if compile {
        let Some(project) = report.project else {
            return observation.fail(
                "analysis",
                serde_json::json!({"code": "missing_analyzed_project"}),
                "analysis did not produce a project",
            );
        };
        let started = std::time::Instant::now();
        observation.publish(
            "hir_validation",
            "validate_hir".into(),
            0,
            project.program.functions.len(),
            serde_json::Value::Null,
            serde_json::json!({}),
        );
        let validation = erabasic_validator::validate_hir(&project.program, &project.data);
        println!(
            "hir_validation_ms={} valid={} diagnostics={}",
            started.elapsed().as_millis(),
            validation.is_valid(),
            validation.diagnostics.len()
        );
        if !validation.is_valid() {
            return observation.fail(
                "hir_validation",
                serde_json::to_value(&validation.diagnostics)?,
                format!(
                    "HIR validation reported {} diagnostics",
                    validation.diagnostics.len()
                ),
            );
        }
        observation.publish(
            "hir_validation",
            serde_json::Value::Null,
            project.program.functions.len(),
            project.program.functions.len(),
            serde_json::Value::Null,
            serde_json::json!({"diagnostics": 0, "valid": true}),
        );
        let started = std::time::Instant::now();
        observation.publish(
            "compile",
            "compile_project".into(),
            0,
            project.program.functions.len(),
            serde_json::Value::Null,
            serde_json::json!({}),
        );
        let progress = |progress: erabasic_compiler::CompileProgress| {
            observation.publish(
                "compile",
                serde_json::json!({
                    "operation": "compile_project",
                    "projectProgress": {
                    "stage": format!("{:?}", progress.stage),
                    "completed": progress.completed,
                    "total": progress.total
                    }
                }),
                progress.completed,
                progress.total,
                serde_json::Value::Null,
                serde_json::json!({}),
            );
        };
        let compiled = erabasic_compiler::compile_project_with_artifact_and_progress(
            &project,
            &Default::default(),
            &erabasic_compiler::default_host_registry(),
            None,
            None,
            &progress,
        );
        println!(
            "compile_ms={} artifact={} diagnostics={} stats={:?}",
            started.elapsed().as_millis(),
            compiled.artifact.is_some(),
            compiled.diagnostics.len(),
            compiled.stats
        );
        if let Some(artifact) = &compiled.artifact {
            let instruction_count = artifact
                .functions
                .iter()
                .map(|function| function.code.len())
                .sum::<usize>();
            let instruction_payload_bytes = artifact
                .functions
                .iter()
                .flat_map(|function| &function.code)
                .map(|instruction| instruction.payload.len())
                .sum::<usize>();
            let instruction_capacity_bytes = artifact
                .functions
                .iter()
                .map(|function| {
                    function.code.capacity()
                        * std::mem::size_of::<erabasic_bytecode::EncodedInstruction>()
                })
                .sum::<usize>();
            let mergeable_source_entries = artifact
                .source_map
                .entries
                .windows(2)
                .filter(|entries| {
                    let [left, right] = entries else { return false };
                    left.function == right.function
                        && left.code_end == right.code_start
                        && left.source_index == right.source_index
                        && left.byte_start == right.byte_start
                        && left.byte_end == right.byte_end
                        && left.statement_fingerprint == right.statement_fingerprint
                        && left.origin_chain == right.origin_chain
                })
                .count();
            println!(
                "functions={} instructions={} instruction_payload_bytes={} instruction_capacity_bytes={} source_entries={} statement_fingerprints={} mergeable_source_entries={} size_source_entry={} size_encoded_instruction={} size_vm_value={}",
                artifact.functions.len(),
                instruction_count,
                instruction_payload_bytes,
                instruction_capacity_bytes,
                artifact.source_map.entries.len(),
                artifact.source_map.statement_fingerprints.len(),
                mergeable_source_entries,
                std::mem::size_of::<erabasic_bytecode::SourceMapEntry>(),
                std::mem::size_of::<erabasic_bytecode::EncodedInstruction>(),
                std::mem::size_of::<erabasic_vm::VmValue>(),
            );
            report_rss("after_compile");
            for function in artifact
                .functions
                .iter()
                .filter(|function| function.name.eq_ignore_ascii_case("EVENTTRAIN"))
            {
                let statics = artifact
                    .globals
                    .iter()
                    .filter(|global| global.owner == Some(function.key))
                    .map(|global| (global.name.as_str(), global.storage))
                    .collect::<Vec<_>>();
                println!("eventtrain={:?} statics={statics:?}", function.key);
            }
            println!(
                "artifact_id={} execution_id={}",
                artifact.manifest.artifact_id, artifact.manifest.program_version.execution_id
            );
            let mut cells = artifact
                .globals
                .iter()
                .map(|global| {
                    let elements = global
                        .dimensions
                        .iter()
                        .copied()
                        .fold(1u128, |product, length| {
                            product.saturating_mul(length.into())
                        });
                    (
                        elements,
                        global.name.as_str(),
                        global.storage,
                        global.value_type,
                    )
                })
                .collect::<Vec<_>>();
            cells.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            let mut storage_elements = std::collections::BTreeMap::new();
            for (elements, _, storage, _) in &cells {
                *storage_elements
                    .entry(format!("{storage:?}"))
                    .or_insert(0u128) += *elements;
            }
            println!(
                "global_elements={} storage_elements={storage_elements:?} top_globals={:?}",
                cells.iter().map(|entry| entry.0).sum::<u128>(),
                &cells[..cells.len().min(20)]
            );
        }
        for diagnostic in compiled.diagnostics.iter().take(100) {
            let where_ = diagnostic.location.map_or_else(String::new, |location| {
                project
                    .program
                    .sources
                    .iter()
                    .find(|source| source.id == location.source)
                    .map_or_else(String::new, |source| {
                        let line = source
                            .line_starts
                            .partition_point(|start| *start <= location.span.start as u64);
                        format!("{}:{}:{} ", source.relative_path, line, location.span.start)
                    })
            });
            println!(
                "compiler {:?}: {where_}{}",
                diagnostic.code, diagnostic.message
            );
        }
        let compiler_errors = compiled
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.severity == erabasic_compiler::CompilerDiagnosticSeverity::Error
            })
            .count();
        if compiler_errors != 0 || compiled.artifact.is_none() {
            let errors = compiled
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.severity == erabasic_compiler::CompilerDiagnosticSeverity::Error
                })
                .collect::<Vec<_>>();
            return observation.fail(
                "compile",
                serde_json::to_value(errors)?,
                format!(
                    "compilation reported {compiler_errors} errors and artifact={}",
                    compiled.artifact.is_some()
                ),
            );
        }
        observation.publish(
            "compile",
            serde_json::Value::Null,
            project.program.functions.len(),
            project.program.functions.len(),
            serde_json::Value::Null,
            serde_json::json!({
                "diagnostics": compiled.diagnostics.len(),
                "errors": 0,
                "artifact": true
            }),
        );
    }
    observation.publish(
        "completed",
        serde_json::Value::Null,
        observation.input_count,
        observation.input_count,
        serde_json::Value::Null,
        serde_json::json!({
            "csvErrors": 0,
            "analyzerErrors": 0,
            "hirDiagnostics": 0,
            "compilerErrors": 0,
            "artifact": compile
        }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::diagnostic_batch;
    use serde_json::json;

    #[test]
    fn project_load_failures_retain_every_collected_diagnostic() {
        let diagnostics = diagnostic_batch(json!([
            {"code": "first", "message": "one"},
            {"code": "second", "message": "two"}
        ]));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0]["code"], "first");
        assert_eq!(diagnostics[1]["code"], "second");
    }
}
