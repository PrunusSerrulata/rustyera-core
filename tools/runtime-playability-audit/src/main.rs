use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use era_protocol::{
    Channel, Envelope, ProtocolBytes, VersionRange, WireLimits, decode_envelope, encode_canonical,
    encode_envelope,
};
use era_runtime::{RuntimeDriveBudget, RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, DiagnosticSeverity, FileCategory, FilePayload, FrontendInput,
    FrontendIoError, FrontendIoErrorKind, InputIntent, InputModality, LOCAL_DATE_TIME_OPERATION,
    LOCAL_DATE_TIME_OPERATION_VERSION, LocalDateTimeResponse, ProjectManifest,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeMessage, SequenceAcknowledgement,
    ServiceCapability, ServiceKind, ServiceResponse, ServiceResult, ShutdownRequest, StartMode,
    StartRequest, StateExportKind, StateExportRequest, StateExportResult, StateImportBegin,
    StateImportChunk, StateImportCommit, StorageCapabilities, StorageNamespace, StorageOperation,
    StorageResponse, StorageResult, SubmittedFile, WaitChange,
};
use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use erabasic_compiler::{ExecutionBinding, default_host_registry};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("audit tool lives under the repository tools directory")
        .to_owned()
}

fn tool_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn default_project() -> PathBuf {
    repository_root().join("reference/eraTW")
}

fn project_argument(index: usize) -> PathBuf {
    env::args()
        .nth(index)
        .map(PathBuf::from)
        .unwrap_or_else(default_project)
}

fn artifact_path(name: &str) -> PathBuf {
    let directory = env::var_os("ERA_AUDIT_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| tool_root().join("artifacts"));
    fs::create_dir_all(&directory).expect("create audit artifact directory");
    directory.join(name)
}

fn main() {
    let command = env::args().nth(1).unwrap_or_else(|| "registry".into());
    match command.as_str() {
        "registry" => audit_registry(),
        "minimal" => audit_minimal(false, false),
        "minimal-root-paths" => audit_minimal(true, false),
        "benchmark" => audit_minimal(true, true),
        "restore-saved" => audit_restore_saved(),
        "parse-file" => audit_parse_file(),
        "csv" => audit_csv(),
        "analyzer" => audit_analyzer(false),
        "compile" => audit_analyzer(true),
        other => panic!("unknown command {other}"),
    }
}

fn audit_restore_saved() {
    let root = project_argument(2);
    let mut paths = Vec::new();
    collect(&root, &root, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    for relative in paths {
        let lower = relative.to_ascii_lowercase();
        let category = if lower.ends_with(".erb") {
            FileCategory::Erb
        } else if lower.ends_with(".erh") {
            FileCategory::Erh
        } else if lower.ends_with(".csv") {
            FileCategory::Csv
        } else if lower.ends_with(".config") {
            FileCategory::Configuration
        } else {
            continue;
        };
        files.push(SubmittedFile {
            relative_path: relative.clone(),
            category,
            payload: FilePayload::Utf8(
                fs::read_to_string(root.join(relative)).expect("submitted project is UTF-8"),
            ),
            content_hash: None,
        });
    }
    let save_path = env::args()
        .nth(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| artifact_path("save99.sav"));
    let save = ProtocolBytes::new(fs::read(save_path).expect("read saved audit autosave"));
    audit_restore(&files, save);
}

fn audit_analyzer(compile: bool) {
    let total_started = std::time::Instant::now();
    let root = project_argument(2);
    let mut paths = Vec::new();
    collect(&root, &root, &mut paths);
    paths.sort();
    let mut csv_files = erabasic_csv::ProjectFiles::default();
    let mut sources = Vec::new();
    for relative in paths {
        let lower = relative.to_ascii_lowercase();
        if !matches!(lower.rsplit('.').next(), Some("csv" | "erb" | "erh")) {
            continue;
        }
        let text = fs::read_to_string(root.join(&relative)).unwrap();
        let stripped = relative
            .strip_prefix("CSV/")
            .or_else(|| relative.strip_prefix("ERB/"))
            .unwrap_or(&relative)
            .to_owned();
        if lower.ends_with(".csv") {
            csv_files.csv.push(erabasic_csv::FrontendFile {
                relative_path: stripped,
                payload: erabasic_csv::FilePayload::Utf8(text),
            });
        } else {
            sources.push(erabasic_analyzer::ProjectSource {
                relative_path: stripped,
                payload: erabasic_analyzer::SourcePayload::Utf8(text),
            });
        }
    }
    let files_elapsed = total_started.elapsed();
    let csv_started = std::time::Instant::now();
    let csv = erabasic_csv::load_project(
        &csv_files,
        &erabasic_csv::CsvLoadOptions {
            use_rename_file: true,
            search_subdirectories: true,
            sort_with_filename: true,
            allow_full_width_space: false,
            ..Default::default()
        },
    );
    let csv_elapsed = csv_started.elapsed();
    println!("csv_diagnostics={}", csv.diagnostics.len());
    let options = erabasic_analyzer::AnalyzerOptions {
        sort_with_filename: true,
        warn_function_overloading: false,
        ignore_uncalled_functions: true,
        compatible_function_argument_auto_convert: true,
        system_save_in_binary: true,
        allow_full_width_space: false,
        ..Default::default()
    };
    let analyze_started = std::time::Instant::now();
    let report = erabasic_analyzer::analyze_project(
        erabasic_analyzer::AnalysisInput {
            project_data: csv.data.unwrap(),
            sources,
        },
        &options,
        &Default::default(),
    );
    let analyze_elapsed = analyze_started.elapsed();
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
    if compile && errors.is_empty() {
        let project = report.project.unwrap();
        let started = std::time::Instant::now();
        let validation = erabasic_validator::validate_hir(&project.program, &project.data);
        println!(
            "hir_validation_ms={} valid={} diagnostics={}",
            started.elapsed().as_millis(),
            validation.is_valid(),
            validation.diagnostics.len()
        );
        let started = std::time::Instant::now();
        let compiled = erabasic_compiler::compile_project(
            &project,
            &Default::default(),
            &erabasic_compiler::default_host_registry(),
            None,
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
            let payload_capacity_bytes = artifact
                .functions
                .iter()
                .flat_map(|function| &function.code)
                .map(|instruction| instruction.payload.capacity())
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
                "functions={} instructions={} instruction_payload_bytes={} instruction_capacity_bytes={} payload_capacity_bytes={} source_entries={} statement_fingerprints={} mergeable_source_entries={} size_source_entry={} size_encoded_instruction={} size_vm_value={}",
                artifact.functions.len(),
                instruction_count,
                instruction_payload_bytes,
                instruction_capacity_bytes,
                payload_capacity_bytes,
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
    }
}

fn audit_csv() {
    let root = project_argument(2);
    let mut paths = Vec::new();
    collect(&root, &root, &mut paths);
    let mut files = erabasic_csv::ProjectFiles::default();
    for relative in paths {
        if !relative.to_ascii_lowercase().ends_with(".csv") {
            continue;
        }
        let content = fs::read_to_string(root.join(&relative)).unwrap();
        let path = relative
            .strip_prefix("CSV/")
            .unwrap_or(&relative)
            .to_owned();
        files.csv.push(erabasic_csv::FrontendFile {
            relative_path: path,
            payload: erabasic_csv::FilePayload::Utf8(content),
        });
    }
    let report = erabasic_csv::load_project(
        &files,
        &erabasic_csv::CsvLoadOptions {
            use_rename_file: true,
            search_subdirectories: true,
            use_erd: false,
            allow_full_width_space: false,
            ..Default::default()
        },
    );
    println!("diagnostics={}", report.diagnostics.len());
    let data = report.data.unwrap();
    if let Some(character) = data
        .static_data
        .characters
        .iter()
        .find(|character| character.no == 144)
    {
        println!("chara144={character:?}");
    }
    for (kind, name) in [
        (erabasic_data::NameTableKind::Tcvar, "工作開始"),
        (erabasic_data::NameTableKind::Cflag, "現在位置"),
        (erabasic_data::NameTableKind::Cflag, "睡眠"),
        (erabasic_data::NameTableKind::Talent, "恋慕"),
    ] {
        let table = &data.static_data.name_tables[&kind];
        println!(
            "{kind:?}: len={} lookup={} {name:?}={:?}",
            table.names.len(),
            table.lookup.len(),
            table.lookup.get(name)
        );
    }
}

fn audit_parse_file() {
    let path = env::args().nth(2).expect("parse-file path");
    let source = fs::read_to_string(&path).unwrap();
    let mut context = erabasic_parser::DefaultParserContext::default();
    let output = erabasic_parser::parse_erb(&source, &mut context);
    println!("parser_diagnostics={}", output.diagnostics.len());
    for diagnostic in output.diagnostics.iter().take(100) {
        let line = source[..diagnostic.span.start.min(source.len())]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        println!(
            "{:?}\tline={}\t{}..{}\t{}",
            diagnostic.code, line, diagnostic.span.start, diagnostic.span.end, diagnostic.message
        );
    }
}

fn audit_registry() {
    let registry = default_host_registry();
    let mut unsupported = builtin_instruction_names()
        .into_iter()
        .chain(builtin_function_names())
        .filter(|name| {
            matches!(
                registry.classification(name),
                Some(ExecutionBinding::Unsupported { .. })
            )
        })
        .collect::<Vec<_>>();
    unsupported.sort();
    unsupported.dedup();
    println!("unsupported_count={}", unsupported.len());
    for name in unsupported {
        println!("{name}");
    }
}

fn audit_minimal(keep_root_paths: bool, benchmark: bool) {
    let total_started = std::time::Instant::now();
    let root_argument = env::args().nth(2);
    let diagnostic_filter = env::args().nth(3);
    let root = root_argument.map_or_else(
        || {
            if benchmark {
                default_project()
            } else {
                tool_root().join("fixture-declaration")
            }
        },
        PathBuf::from,
    );
    let mut paths = Vec::new();
    collect(&root, &root, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    for relative in paths {
        let lower = relative.to_ascii_lowercase();
        let category = if lower.ends_with(".erb") {
            FileCategory::Erb
        } else if lower.ends_with(".erh") {
            FileCategory::Erh
        } else if lower.ends_with(".csv") {
            FileCategory::Csv
        } else if lower.ends_with(".config") {
            FileCategory::Configuration
        } else {
            continue;
        };
        let text = fs::read_to_string(root.join(&relative)).expect("minimal fixture is UTF-8");
        let submitted_path = if keep_root_paths {
            relative.clone()
        } else {
            relative
                .strip_prefix("CSV/")
                .or_else(|| relative.strip_prefix("ERB/"))
                .or_else(|| relative.strip_prefix("csv/"))
                .or_else(|| relative.strip_prefix("erb/"))
                .unwrap_or(&relative)
                .to_owned()
        };
        files.push(SubmittedFile {
            relative_path: submitted_path,
            category,
            payload: FilePayload::Utf8(text),
            content_hash: None,
        });
    }
    let file_prepare_elapsed = total_started.elapsed();
    if !benchmark {
        println!("submitted_files={}", files.len());
    }
    let restore_files = (!benchmark).then(|| files.clone());

    let mut runtime_options = RuntimeOptions::default();
    runtime_options.limits.maximum_envelope_bytes = 128 * 1024 * 1024;
    runtime_options.limits.maximum_payload_bytes = 127 * 1024 * 1024;
    runtime_options.wire_limits = audit_wire_limits();
    let requested_limits = runtime_options.limits;
    let mut session = RuntimeSession::new(runtime_options);
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "tui-audit".into(),
            features: vec![
                RuntimeFeature::TraditionalSave,
                RuntimeFeature::TimedInput,
                RuntimeFeature::Storage,
                RuntimeFeature::ExternalServices,
                RuntimeFeature::StateResynchronization,
                RuntimeFeature::VmSnapshot,
            ],
            requested_limits,
            capabilities: ClientCapabilities {
                input_modalities: vec![InputModality::Keyboard],
                rich_text: false,
                html: false,
                graphics: false,
                audio: false,
                video: false,
                font_metrics: false,
                column_cells: true,
                separators: true,
                available_fonts: Vec::new(),
                services: vec![ServiceCapability {
                    kind: ServiceKind::Clock,
                    operation: LOCAL_DATE_TIME_OPERATION.into(),
                    versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
                }],
                storage: StorageCapabilities {
                    revisions: true,
                    atomic_replace: true,
                    missing_precondition: true,
                    delete: true,
                },
            },
            preferred_locales: vec!["ja".into()],
        }),
    );
    drive(&mut session);
    for message in drain(&mut session) {
        if let RuntimeMessage::ServerHello(hello) = message
            && !benchmark
        {
            println!("selected_features={:?}", hello.features);
            println!("selected_capabilities={:?}", hello.selected_capabilities);
        }
    }

    let project_load_started = std::time::Instant::now();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files,
        }),
    );
    drive(&mut session);
    let project_load_elapsed = project_load_started.elapsed();
    if benchmark {
        report_rss("after_project_load");
    }
    let messages = drain(&mut session);
    for message in &messages {
        match message {
            RuntimeMessage::ProjectLoadReport(report) => {
                if !benchmark {
                    println!("load_success={}", report.success);
                }
                let errors = report
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Error)
                    .count();
                let warnings = report
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Warning)
                    .count();
                if !benchmark {
                    println!(
                        "diagnostics={} errors={} warnings={}",
                        report.diagnostics.len(),
                        errors,
                        warnings
                    );
                }
                let mut by_code = std::collections::BTreeMap::<String, usize>::new();
                let mut by_file = std::collections::BTreeMap::<String, usize>::new();
                for diagnostic in report
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Error)
                {
                    *by_code.entry(diagnostic.code.clone()).or_default() += 1;
                    *by_file
                        .entry(
                            diagnostic
                                .source
                                .as_ref()
                                .map_or("<none>", |s| s.relative_path.as_str())
                                .to_owned(),
                        )
                        .or_default() += 1;
                }
                let mut by_code = by_code.into_iter().collect::<Vec<_>>();
                by_code.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                let mut by_file = by_file.into_iter().collect::<Vec<_>>();
                by_file.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
                if !benchmark {
                    println!(
                        "top_error_codes={:?}",
                        by_code.into_iter().take(20).collect::<Vec<_>>()
                    );
                    println!(
                        "top_error_files={:?}",
                        by_file.into_iter().take(20).collect::<Vec<_>>()
                    );
                    for diagnostic in report
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| {
                            diagnostic_filter.as_ref().is_none_or(|filter| {
                                diagnostic
                                    .source
                                    .as_ref()
                                    .is_some_and(|source| source.relative_path.contains(filter))
                                    || diagnostic.code.contains(filter)
                            })
                        })
                        .take(200)
                    {
                        println!(
                            "{:?}\t{}\t{}:{}:{}\t{}",
                            diagnostic.severity,
                            diagnostic.code,
                            diagnostic
                                .source
                                .as_ref()
                                .map_or("", |s| s.relative_path.as_str()),
                            diagnostic.source.as_ref().and_then(|s| s.line).unwrap_or(0),
                            diagnostic.source.as_ref().map_or(0, |s| s.byte_start),
                            diagnostic.message.replace('\n', " ")
                        );
                    }
                }
            }
            RuntimeMessage::ServiceRequest(request) => {
                if !benchmark {
                    println!("load_service={:?}/{}", request.kind, request.operation);
                }
            }
            RuntimeMessage::Fault(fault) if !benchmark => println!("load_fault={fault:?}"),
            _ => {}
        }
    }
    if !benchmark {
        println!("phase_after_load={:?}", session.phase());
    }
    if !messages.iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success),
    ) {
        return;
    }
    let start_started = std::time::Instant::now();
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut sequence = 3;
    let answers = [
        0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 9999, 0, 2, 1999, 0, 100, 1,
    ];
    let mut answer_index = 0;
    let mut last_text = String::new();
    let mut storage =
        std::collections::BTreeMap::<(StorageNamespace, String), (ProtocolBytes, String)>::new();
    let mut day_one_elapsed = None;
    let mut snapshot_count = 0_u64;
    let mut delta_count = 0_u64;
    for step in 0..20_000 {
        let drive_started = std::time::Instant::now();
        let drive_report = session
            .drive(RuntimeDriveBudget {
                maximum_vm_instructions: 10_000,
                maximum_runtime_transitions: 128,
            })
            .unwrap();
        let drive_elapsed = drive_started.elapsed();
        let drain_started = std::time::Instant::now();
        let (out, last_outbound_sequence) = drain_with_last_sequence(&mut session);
        let drain_elapsed = drain_started.elapsed();
        if benchmark && (drive_elapsed.as_millis() >= 250 || drain_elapsed.as_millis() >= 250) {
            println!(
                "slow_step={step} drive_ms={} drain_ms={} instructions={} transitions={} envelopes={}",
                drive_elapsed.as_millis(),
                drain_elapsed.as_millis(),
                drive_report.vm_instructions,
                drive_report.runtime_transitions,
                drive_report.queued_envelopes,
            );
        }
        let mut followups = Vec::new();
        let mut unplanned_wait = false;
        for message in out {
            match &message {
                RuntimeMessage::PresentationSnapshot(snapshot) => {
                    snapshot_count += 1;
                    if benchmark {
                        println!(
                            "snapshot step={step} elapsed_ms={} revision={} lines={} history={} sprites={} canvases={} redraw={}",
                            start_started.elapsed().as_millis(),
                            snapshot.revision,
                            snapshot.history.logical_lines.len(),
                            snapshot.history.operations.len(),
                            snapshot.resources.sprites.len(),
                            snapshot.resources.canvases.len(),
                            snapshot.redraw.enabled,
                        );
                    }
                }
                RuntimeMessage::PresentationDelta(_) => delta_count += 1,
                _ => {}
            }
            match message {
                RuntimeMessage::Fault(fault) => {
                    println!("runtime_fault_step={step} {fault:?}");
                }
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::Clock
                        && request.operation == LOCAL_DATE_TIME_OPERATION =>
                {
                    followups.push(RuntimeMessage::ServiceResponse(ServiceResponse {
                        request_id: request.request_id,
                        result: ServiceResult::Ready {
                            payload: ProtocolBytes::new(
                                encode_canonical(&LocalDateTimeResponse {
                                    year: 2026,
                                    month: 7,
                                    day: 19,
                                    hour: 12,
                                    minute: 0,
                                    second: 0,
                                    millisecond: 0,
                                    utc_offset_minutes: 480,
                                })
                                .expect("encode fixed audit time"),
                            ),
                        },
                    }));
                }
                RuntimeMessage::ServiceRequest(request) if !benchmark => println!(
                    "runtime_service_step={step} {:?}/{}",
                    request.kind, request.operation
                ),
                RuntimeMessage::StorageRequest(request) => {
                    if !benchmark {
                        println!(
                            "runtime_storage_step={step} {:?} {} {:?}",
                            request.namespace, request.relative_path, request.operation
                        );
                    }
                    let key = (request.namespace, request.relative_path.clone());
                    let not_found = || StorageResult::Error {
                        error: FrontendIoError {
                            kind: FrontendIoErrorKind::NotFound,
                            message: "audit fixture has no stored file".into(),
                            platform_code: None,
                        },
                    };
                    let result = match request.operation {
                        StorageOperation::Read => {
                            storage
                                .get(&key)
                                .map_or_else(not_found, |stored| StorageResult::Read {
                                    data: stored.0.clone(),
                                    revision: Some(stored.1.clone()),
                                })
                        }
                        StorageOperation::Stat => {
                            storage.get(&key).map_or_else(not_found, |stored| {
                                StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                                    byte_length: stored.0.as_slice().len() as u64,
                                    revision: Some(stored.1.clone()),
                                })
                            })
                        }
                        StorageOperation::Write { data, .. } => {
                            let revision = format!("audit-{step}");
                            storage.insert(key, (data, revision.clone()));
                            StorageResult::Written {
                                revision: Some(revision),
                            }
                        }
                        StorageOperation::List { .. } => StorageResult::Listed {
                            entries: Vec::new(),
                        },
                        StorageOperation::Delete { .. } => StorageResult::Deleted,
                        StorageOperation::ReadRange {
                            offset,
                            maximum_bytes,
                            change_token,
                        } => storage.get(&key).map_or_else(not_found, |stored| {
                            if change_token
                                .as_ref()
                                .is_some_and(|expected| expected != &stored.1)
                            {
                                return StorageResult::Error {
                                    error: FrontendIoError {
                                        kind: FrontendIoErrorKind::Conflict,
                                        message: "audit storage changed during range read".into(),
                                        platform_code: None,
                                    },
                                };
                            }
                            let bytes = stored.0.as_slice();
                            let start = usize::try_from(offset)
                                .unwrap_or(usize::MAX)
                                .min(bytes.len());
                            let end = start
                                .saturating_add(maximum_bytes as usize)
                                .min(bytes.len());
                            StorageResult::ReadChunk {
                                data: ProtocolBytes::new(bytes[start..end].to_vec()),
                                offset,
                                complete: end == bytes.len(),
                                change_token: stored.1.clone(),
                            }
                        }),
                    };
                    followups.push(RuntimeMessage::StorageResponse(StorageResponse {
                        request_id: request.request_id,
                        result,
                    }));
                }
                RuntimeMessage::WaitChanged(wait) => {
                    if !benchmark {
                        println!("runtime_wait_step={step} {wait:?}");
                        println!("runtime_wait_text={last_text}");
                    }
                    if let WaitChange::Opened(wait) = wait {
                        let intent = if wait.kind == era_runtime_protocol::WaitKind::EnterKey {
                            InputIntent::Enter
                        } else if let Some(answer) = answers.get(answer_index).copied() {
                            answer_index += 1;
                            if !benchmark {
                                println!("runtime_answer[{answer_index}]={answer}");
                            }
                            InputIntent::CommitText(answer.to_string())
                        } else {
                            if !benchmark {
                                println!("runtime_unplanned_wait={wait:?}");
                            }
                            if wait.system_input {
                                day_one_elapsed = Some(start_started.elapsed());
                            }
                            unplanned_wait = true;
                            continue;
                        };
                        followups.push(RuntimeMessage::Input(FrontendInput {
                            wait_id: wait.wait_id,
                            token: wait.submission_token,
                            monotonic_time_ns: sequence * 1_000_000,
                            intent,
                            message_skip: false,
                        }));
                    }
                }
                RuntimeMessage::PresentationSnapshot(snapshot) if !benchmark => {
                    last_text = snapshot
                        .history
                        .logical_lines
                        .iter()
                        .rev()
                        .take(12)
                        .rev()
                        .flat_map(|line| line.runs.iter())
                        .map(display_text)
                        .collect::<Vec<_>>()
                        .join(" | ");
                }
                RuntimeMessage::StateChanged(state) if !benchmark => {
                    println!("runtime_state_step={step} {:?}", state.phase)
                }
                _ => {}
            }
        }
        if let Some(through_sequence) = last_outbound_sequence {
            followups.push(RuntimeMessage::Acknowledge(SequenceAcknowledgement {
                through_sequence,
            }));
        }
        for followup in followups {
            submit(&mut session, sequence, followup);
            sequence += 1;
        }
        if matches!(session.phase(), era_runtime_protocol::RuntimePhase::Faulted) {
            break;
        }
        if unplanned_wait {
            break;
        }
        if benchmark && step != 0 && step % 1_000 == 0 {
            println!(
                "progress_step={step} elapsed_ms={} snapshots={snapshot_count} deltas={delta_count}",
                start_started.elapsed().as_millis()
            );
        }
    }
    if benchmark {
        report_rss("at_day1");
        submit(
            &mut session,
            sequence,
            RuntimeMessage::StateExportRequest(StateExportRequest {
                kind: StateExportKind::VmSnapshot,
            }),
        );
        drive(&mut session);
        for message in drain(&mut session) {
            if let RuntimeMessage::StateExportReady(ready) = message {
                match ready.result {
                    StateExportResult::Ready { transfer } => {
                        println!("vm_snapshot_bytes={}", transfer.total_bytes);
                    }
                    StateExportResult::Ineligible { reasons } => {
                        println!("vm_snapshot_ineligible={reasons:?}");
                    }
                }
            }
        }
        report_rss("after_snapshot_export");
        println!("file_prepare_ms={}", file_prepare_elapsed.as_millis());
        println!("project_load_ms={}", project_load_elapsed.as_millis());
        println!(
            "start_to_day1_ms={}",
            day_one_elapsed.map_or(u128::MAX, |elapsed| elapsed.as_millis())
        );
        println!("total_to_day1_ms={}", total_started.elapsed().as_millis());
        println!("snapshots={snapshot_count} deltas={delta_count}");
        println!("phase_after_start={:?}", session.phase());
        if env::var_os("ERA_AUDIT_PAUSE").is_some() {
            println!("audit_pid={} paused", std::process::id());
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
        }
        return;
    }
    if let (Some(restore_files), Some((save, _))) = (
        restore_files.as_ref(),
        storage.get(&(StorageNamespace::Save, "save99.sav".into())),
    ) {
        fs::write(artifact_path("save99.sav"), save.as_slice()).expect("persist audit autosave");
        audit_restore(restore_files, save.clone());
    }
    println!("runtime_final_text={last_text}");
    println!("phase_after_start={:?}", session.phase());
}

fn report_rss(stage: &str) {
    let pid = std::process::id().to_string();
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .expect("query current RSS");
    let rss_kib = String::from_utf8(output.stdout)
        .expect("ps RSS is UTF-8")
        .trim()
        .parse::<u64>()
        .expect("ps RSS is numeric");
    println!("rss_{stage}_bytes={}", rss_kib * 1024);
}

fn audit_restore(files: &[SubmittedFile], save: ProtocolBytes) {
    println!("restore_begin_bytes={}", save.as_slice().len());
    let mut runtime_options = RuntimeOptions::default();
    runtime_options.limits.maximum_envelope_bytes = 128 * 1024 * 1024;
    runtime_options.limits.maximum_payload_bytes = 127 * 1024 * 1024;
    runtime_options.wire_limits = audit_wire_limits();
    let requested_limits = runtime_options.limits;
    let mut session = RuntimeSession::new(runtime_options);
    submit_with_epoch(
        &mut session,
        0,
        None,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "tui-audit-restore".into(),
            features: vec![
                RuntimeFeature::TraditionalSave,
                RuntimeFeature::TimedInput,
                RuntimeFeature::Storage,
                RuntimeFeature::ExternalServices,
                RuntimeFeature::StateResynchronization,
            ],
            requested_limits,
            capabilities: ClientCapabilities {
                input_modalities: vec![InputModality::Keyboard],
                rich_text: false,
                html: false,
                graphics: false,
                audio: false,
                video: false,
                font_metrics: false,
                column_cells: true,
                separators: true,
                available_fonts: Vec::new(),
                services: vec![ServiceCapability {
                    kind: ServiceKind::Clock,
                    operation: LOCAL_DATE_TIME_OPERATION.into(),
                    versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
                }],
                storage: StorageCapabilities {
                    revisions: true,
                    atomic_replace: true,
                    missing_precondition: true,
                    delete: true,
                },
            },
            preferred_locales: vec!["ja".into()],
        }),
    );
    drive(&mut session);
    drain(&mut session);
    submit_with_epoch(
        &mut session,
        1,
        Some(1),
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: files.to_vec(),
        }),
    );
    drive(&mut session);
    let load = drain(&mut session);
    let loaded = load.iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success),
    );
    println!("restore_project_loaded={loaded}");
    if !loaded {
        for message in load {
            if matches!(
                message,
                RuntimeMessage::Fault(_) | RuntimeMessage::ProjectLoadReport(_)
            ) {
                println!("restore_load_message={message:?}");
            }
        }
        return;
    }

    let digest = ProtocolBytes::new(*blake3::hash(save.as_slice()).as_bytes());
    submit_with_epoch(
        &mut session,
        2,
        Some(1),
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: save.as_slice().len() as u64,
            digest,
            artifact_id: None,
        }),
    );
    drive(&mut session);
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            other => {
                println!("restore_import_begin_message={other:?}");
                None
            }
        })
        .expect("traditional save import accepted");
    submit_with_epoch(
        &mut session,
        3,
        Some(1),
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: save,
        }),
    );
    submit_with_epoch(
        &mut session,
        4,
        Some(1),
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    drive(&mut session);
    let committed = drain(&mut session);
    println!("restore_import_messages={committed:?}");
    submit_with_epoch(
        &mut session,
        5,
        Some(1),
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::TraditionalSave { transfer_id },
        }),
    );

    let mut sequence = 6;
    let mut last_text = String::new();
    for step in 0..2_000 {
        drive(&mut session);
        let mut followups = Vec::new();
        let mut stable_wait = false;
        for message in drain(&mut session) {
            match message {
                RuntimeMessage::Fault(fault) => println!("restore_fault_step={step} {fault:?}"),
                RuntimeMessage::PresentationSnapshot(snapshot) => {
                    last_text = snapshot
                        .history
                        .logical_lines
                        .iter()
                        .rev()
                        .take(12)
                        .rev()
                        .flat_map(|line| line.runs.iter())
                        .map(display_text)
                        .collect::<Vec<_>>()
                        .join(" | ");
                }
                RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => {
                    println!("restore_wait_step={step} {wait:?}");
                    if wait.kind == era_runtime_protocol::WaitKind::EnterKey {
                        followups.push(RuntimeMessage::Input(FrontendInput {
                            wait_id: wait.wait_id,
                            token: wait.submission_token,
                            monotonic_time_ns: sequence * 1_000_000,
                            intent: InputIntent::Enter,
                            message_skip: false,
                        }));
                    } else {
                        stable_wait = true;
                    }
                }
                RuntimeMessage::StorageRequest(request) => {
                    followups.push(RuntimeMessage::StorageResponse(StorageResponse {
                        request_id: request.request_id,
                        result: StorageResult::Error {
                            error: FrontendIoError {
                                kind: FrontendIoErrorKind::NotFound,
                                message: "restore audit storage is empty".into(),
                                platform_code: None,
                            },
                        },
                    }));
                }
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::Clock
                        && request.operation == LOCAL_DATE_TIME_OPERATION =>
                {
                    followups.push(RuntimeMessage::ServiceResponse(ServiceResponse {
                        request_id: request.request_id,
                        result: ServiceResult::Ready {
                            payload: ProtocolBytes::new(
                                encode_canonical(&LocalDateTimeResponse {
                                    year: 2026,
                                    month: 7,
                                    day: 19,
                                    hour: 12,
                                    minute: 0,
                                    second: 0,
                                    millisecond: 0,
                                    utc_offset_minutes: 480,
                                })
                                .expect("encode fixed audit time"),
                            ),
                        },
                    }));
                }
                RuntimeMessage::ServiceRequest(request) => {
                    println!("restore_unhandled_service={request:?}");
                }
                RuntimeMessage::StateChanged(state) => {
                    println!("restore_state_step={step} {:?}", state.phase)
                }
                _ => {}
            }
        }
        for followup in followups {
            submit_with_epoch(&mut session, sequence, Some(2), followup);
            sequence += 1;
        }
        if stable_wait || matches!(session.phase(), era_runtime_protocol::RuntimePhase::Faulted) {
            break;
        }
    }
    println!("restore_final_text={last_text}");
    println!("restore_final_phase={:?}", session.phase());
    submit_with_epoch(
        &mut session,
        sequence,
        Some(2),
        RuntimeMessage::ShutdownRequest(ShutdownRequest { graceful: true }),
    );
    drive(&mut session);
    println!("restore_shutdown_messages={:?}", drain(&mut session));
    println!("restore_shutdown_phase={:?}", session.phase());
}

fn display_text(run: &era_runtime_protocol::DisplayRun) -> String {
    match run {
        era_runtime_protocol::DisplayRun::Text { text, .. } => text.clone(),
        era_runtime_protocol::DisplayRun::Button { runs, value, .. } => format!(
            "[{} => {value:?}]",
            runs.iter().map(display_text).collect::<String>()
        ),
        era_runtime_protocol::DisplayRun::ColumnCell { content, .. } => {
            content.iter().map(display_text).collect()
        }
        era_runtime_protocol::DisplayRun::HtmlDocument { document } => format!("{document:?}"),
        era_runtime_protocol::DisplayRun::Image { alt_text, .. } => {
            alt_text.clone().unwrap_or_else(|| "[image]".into())
        }
        era_runtime_protocol::DisplayRun::Separator { pattern, .. } => pattern.clone(),
        era_runtime_protocol::DisplayRun::Shape { .. } => "[shape]".into(),
        era_runtime_protocol::DisplayRun::Space { .. } => " ".into(),
    }
}

fn collect(root: &Path, current: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(current).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

fn submit(session: &mut RuntimeSession, sequence: u64, message: RuntimeMessage) {
    submit_with_epoch(
        session,
        sequence,
        (sequence != 0).then_some(if sequence >= 3 { 2 } else { 1 }),
        message,
    );
}

fn submit_with_epoch(
    session: &mut RuntimeSession,
    sequence: u64,
    epoch: Option<u64>,
    message: RuntimeMessage,
) {
    let mut envelope = Envelope::new(
        Channel::Runtime,
        RUNTIME_PROTOCOL_VERSION,
        sequence,
        sequence + 1,
        message.tag(),
        ProtocolBytes::new(message.encode_payload().unwrap()),
    );
    if let Some(epoch) = epoch {
        envelope.session = Some(RuntimeOptions::default().session_id);
        envelope.session_epoch = Some(era_protocol::SessionEpoch(epoch));
    }
    let bytes = encode_envelope(&envelope, audit_wire_limits()).unwrap();
    session.submit_envelope(&bytes).unwrap();
}

fn drive(session: &mut RuntimeSession) {
    let report = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 10_000,
            maximum_runtime_transitions: 128,
        })
        .unwrap();
    let _ = report;
}

fn drain(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
    drain_with_last_sequence(session).0
}

fn drain_with_last_sequence(session: &mut RuntimeSession) -> (Vec<RuntimeMessage>, Option<u64>) {
    let mut messages = Vec::new();
    let mut last_sequence = None;
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, audit_wire_limits()).unwrap();
        last_sequence = Some(envelope.sequence);
        messages.push(RuntimeMessage::from_envelope(&envelope).unwrap());
    }
    (messages, last_sequence)
}

fn audit_wire_limits() -> WireLimits {
    WireLimits {
        maximum_envelope_bytes: 128 * 1024 * 1024,
        maximum_payload_bytes: 127 * 1024 * 1024,
    }
}
