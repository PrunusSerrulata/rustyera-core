use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use era_protocol::{
    Channel, Envelope, ProtocolBytes, VersionRange, WireLimits, decode_canonical, decode_envelope,
    encode_canonical, encode_envelope,
};
use era_runtime::{RuntimeDriveBudget, RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, DisplayLine, FileCategory, FrontendInput, FrontendIoError,
    FrontendIoErrorKind, HTML_GET_PRINTED_STR_OPERATION, HTML_GET_PRINTED_STR_OPERATION_VERSION,
    InputIntent, InputModality, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION,
    LineAlignment, LocalDateTimeResponse, PresentationOperation, ProjectManifest,
    ProjectionStringIndexRequest, ProjectionStringResponse, ProtocolDiagnostic,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeLogLevel, RuntimeMessage,
    SequenceAcknowledgement, ServiceCapability, ServiceKind, ServiceResponse, ServiceResult,
    ShutdownRequest, SnapshotExportPurpose, StartMode, StartRequest, StateExportKind,
    StateExportRequest, StateExportResult, StateImportBegin, StateImportChunk, StateImportCommit,
    StorageCapabilities, StorageNamespace, StorageOperation, StorageResponse, StorageResult,
    SubmittedFile, WaitChange,
};
use erabasic_analyzer::{builtin_function_names, builtin_instruction_names};
use erabasic_compiler::{ExecutionBinding, default_host_registry};

mod baseline;
mod compile_audit;
mod coverage;
mod project_extractor;
mod project_inputs;
mod snake_observations;
mod watchdog;

fn diagnostics_with_level(
    diagnostics: &[ProtocolDiagnostic],
    level: RuntimeLogLevel,
) -> impl Iterator<Item = &ProtocolDiagnostic> {
    diagnostics
        .iter()
        .filter(move |diagnostic| diagnostic.level == level)
}

fn decode_project_text(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_owned());
    }
    // The audit harness acts as a frontend: match Emuera's strict UTF-8-first
    // detection and normalize its Windows-31J fallback before submission.
    encoding_rs::SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .or_else(|| encoding_rs::GBK.decode_without_bom_handling_and_without_replacement(bytes))
        .map(|text| text.into_owned())
}

fn read_project_text(path: impl AsRef<Path>) -> std::io::Result<String> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    decode_project_text(&bytes).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} is not valid UTF-8, Windows-31J, or GBK", path.display()),
        )
    })
}

fn read_submitted_text(path: impl AsRef<Path>, category: FileCategory) -> std::io::Result<String> {
    let path = path.as_ref();
    if !matches!(category, FileCategory::Als | FileCategory::Erd)
        && !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("reraconfig.toml"))
    {
        return read_project_text(path);
    }
    let bytes = fs::read(path)?;
    std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
        .map(str::to_owned)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

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
    repository_root()
        .parent()
        .expect("core repository has a workspace parent")
        .join("eraTW")
}

fn target_directory() -> PathBuf {
    if let Some(path) = env::var_os("ERA_TARGET_DIR") {
        return PathBuf::from(path);
    }
    let repository = repository_root();
    let workspace_target = repository
        .parent()
        .expect("core repository has a workspace parent")
        .join("target");
    if repository
        .parent()
        .is_some_and(|parent| parent.join(".cargo/config.toml").is_file())
    {
        workspace_target
    } else {
        repository.join("target")
    }
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
        "baseline" => run_audit_command(watchdog::supervise("baseline", baseline::run_cli)),
        "coverage" => run_audit_command(watchdog::supervise("coverage", coverage::run_cli)),
        "snake-observations" => run_audit_command(watchdog::supervise(
            "snake-observations",
            snake_observations::run_cli,
        )),
        "registry" => audit_registry(),
        "minimal" => audit_minimal(false, false),
        "minimal-root-paths" => audit_minimal(true, false),
        "benchmark" => audit_minimal(true, true),
        "restore-saved" => audit_restore_saved(),
        "parse-file" => audit_parse_file(),
        "csv" => audit_csv(),
        "analyzer" => run_audit_command(compile_audit::run(false)),
        "compile" => run_audit_command(compile_audit::supervise()),
        "project-extractor-all" => project_extractor::audit_all_reference_games(),
        other => panic!("unknown command {other}"),
    }
}

fn run_audit_command(result: Result<(), Box<dyn std::error::Error>>) {
    if let Err(error) = result {
        eprintln!("audit failed: {error}");
        std::process::exit(2);
    }
}

fn audit_restore_saved() {
    let root = project_argument(2);
    let paths = collect_project_files(&root);
    let files =
        project_inputs::ProjectInputs::new(&root, &paths).submitted_files(&root, &paths, true);
    let save_path = env::args()
        .nth(3)
        .map(PathBuf::from)
        .unwrap_or_else(|| artifact_path("save99.sav"));
    let save = ProtocolBytes::new(fs::read(save_path).expect("read saved audit autosave"));
    audit_restore(&files, save);
}

fn audit_csv() {
    let root = project_argument(2);
    let paths = collect_project_files(&root);
    let inputs = project_inputs::ProjectInputs::new(&root, &paths);
    let mut files = erabasic_csv::ProjectFiles::default();
    for relative in paths {
        let Some(category) = inputs.classify(&relative) else {
            continue;
        };
        if inputs.data_root(&relative, category) != Some(project_inputs::DataRoot::Csv) {
            continue;
        }
        let content = read_submitted_text(root.join(&relative), category).unwrap();
        files.csv.push(erabasic_csv::FrontendFile {
            relative_path: project_inputs::DataRoot::Csv.relative_path(&relative),
            source_path: Some(relative),
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
    let source = read_project_text(&path).unwrap();
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
                Some(
                    ExecutionBinding::Unsupported { .. }
                        | ExecutionBinding::UnsupportedCapability { .. },
                )
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

const MINIMAL_AUDIT_ANSWERS: &[i64] = &[
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 9999, 0, 2, 1999, 0, 100, 1,
];
const ERATW_BENCHMARK_ANSWERS: &[i64] = &[
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 9999, 0, 2, 1999, 0, 100, 1, 2000, 1999, 0, 100, 1, 100,
];

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
    let paths = collect_project_files(&root);
    let files = project_inputs::ProjectInputs::new(&root, &paths).submitted_files(
        &root,
        &paths,
        keep_root_paths,
    );
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
            configuration_profile: None,
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
                environment: Vec::new(),
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
                services: audit_service_capabilities(),
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                let errors =
                    diagnostics_with_level(&report.diagnostics, RuntimeLogLevel::Error).count();
                let warnings =
                    diagnostics_with_level(&report.diagnostics, RuntimeLogLevel::Warning).count();
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
                for diagnostic in
                    diagnostics_with_level(&report.diagnostics, RuntimeLogLevel::Error)
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
                            diagnostic.level,
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
    let answers = if benchmark {
        ERATW_BENCHMARK_ANSWERS
    } else {
        MINIMAL_AUDIT_ANSWERS
    };
    let mut answer_index = 0;
    let mut last_text = String::new();
    let mut storage =
        std::collections::BTreeMap::<(StorageNamespace, String), (ProtocolBytes, String)>::new();
    let mut day_one_elapsed = None;
    let mut wake_started = None;
    let mut wake_instruction = None;
    let mut wake_to_home_elapsed = None;
    let mut total_vm_instructions = 0_u64;
    let mut snapshot_count = 0_u64;
    let mut delta_count = 0_u64;
    let mut presentation_lines = Vec::<DisplayLine>::new();
    for step in 0..20_000 {
        let drive_started = std::time::Instant::now();
        let drive_report = session
            .drive(RuntimeDriveBudget {
                maximum_vm_instructions: 10_000,
                maximum_runtime_transitions: 128,
            })
            .unwrap();
        let drive_elapsed = drive_started.elapsed();
        total_vm_instructions = total_vm_instructions.saturating_add(drive_report.vm_instructions);
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
                    presentation_lines.clone_from(&snapshot.history.logical_lines);
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
                RuntimeMessage::PresentationDelta(delta) => {
                    delta_count += 1;
                    apply_presentation_delta(&mut presentation_lines, &delta.operations);
                }
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
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::PresentationQuery
                        && request.operation == HTML_GET_PRINTED_STR_OPERATION =>
                {
                    let query: ProjectionStringIndexRequest =
                        decode_canonical(request.payload.as_slice())
                            .expect("decode printed HTML audit query");
                    followups.push(RuntimeMessage::ServiceResponse(ServiceResponse {
                        request_id: request.request_id,
                        result: ServiceResult::Ready {
                            payload: ProtocolBytes::new(
                                encode_canonical(&ProjectionStringResponse {
                                    context: query.context,
                                    value: headless_html_printed_str(
                                        &presentation_lines,
                                        query.index,
                                    ),
                                })
                                .expect("encode printed HTML audit response"),
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
                            if benchmark && answer == 100 {
                                let contains_wake_prompt = presentation_lines
                                    .iter()
                                    .flat_map(|line| line.runs.iter())
                                    .map(display_text)
                                    .any(|text| text.contains("睜開眼睛"));
                                if contains_wake_prompt && wake_started.is_none() {
                                    println!("wake_input_instruction={total_vm_instructions}");
                                    wake_started = Some(std::time::Instant::now());
                                    wake_instruction = Some(total_vm_instructions);
                                }
                            }
                            if !benchmark {
                                println!("runtime_answer[{answer_index}]={answer}");
                            }
                            InputIntent::CommitText(answer.to_string())
                        } else {
                            if wait.system_input {
                                let reached_home = std::time::Instant::now();
                                day_one_elapsed = Some(reached_home.duration_since(start_started));
                                wake_to_home_elapsed = wake_started
                                    .map(|started| reached_home.duration_since(started));
                            }
                            println!("runtime_unplanned_wait={wait:?}");
                            if benchmark {
                                let visible_text = presentation_lines
                                    .iter()
                                    .rev()
                                    .take(12)
                                    .rev()
                                    .flat_map(|line| line.runs.iter())
                                    .map(display_text)
                                    .collect::<Vec<_>>()
                                    .join(" | ");
                                println!("runtime_unplanned_text={visible_text}");
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
                snapshot_purpose: SnapshotExportPurpose::Normal,
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
        println!("vm_instructions_to_day1={total_vm_instructions}");
        println!(
            "wake_to_home_ms={}",
            wake_to_home_elapsed.map_or(u128::MAX, |elapsed| elapsed.as_millis())
        );
        println!(
            "wake_to_home_instructions={}",
            wake_instruction.map_or(u64::MAX, |started| {
                total_vm_instructions.saturating_sub(started)
            })
        );
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
    let Ok(output) = Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
    else {
        println!("rss_{stage}_bytes=unavailable");
        return;
    };
    let Ok(stdout) = String::from_utf8(output.stdout) else {
        println!("rss_{stage}_bytes=unavailable");
        return;
    };
    let Ok(rss_kib) = stdout.trim().parse::<u64>() else {
        println!("rss_{stage}_bytes=unavailable");
        return;
    };
    println!("rss_{stage}_bytes={}", rss_kib.saturating_mul(1024));
}

fn audit_restore(files: &[SubmittedFile], save: ProtocolBytes) {
    let total_started = std::time::Instant::now();
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
            configuration_profile: None,
            features: vec![
                RuntimeFeature::TraditionalSave,
                RuntimeFeature::TimedInput,
                RuntimeFeature::Storage,
                RuntimeFeature::ExternalServices,
                RuntimeFeature::StateResynchronization,
            ],
            requested_limits,
            capabilities: ClientCapabilities {
                environment: Vec::new(),
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
                services: audit_service_capabilities(),
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
    let project_load_started = std::time::Instant::now();
    submit_with_epoch(
        &mut session,
        1,
        Some(1),
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: files.to_vec(),
        }),
    );
    drive(&mut session);
    let load = drain(&mut session);
    println!(
        "restore_project_load_ms={}",
        project_load_started.elapsed().as_millis()
    );
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
            digest: Some(digest),
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
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: None,
        }),
    );
    drive(&mut session);
    let committed = drain(&mut session);
    println!("restore_import_messages={committed:?}");
    let start_started = std::time::Instant::now();
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
    let mut presentation_lines = Vec::<DisplayLine>::new();
    for step in 0..2_000 {
        drive(&mut session);
        let mut followups = Vec::new();
        let mut stable_wait = false;
        for message in drain(&mut session) {
            match message {
                RuntimeMessage::Fault(fault) => println!("restore_fault_step={step} {fault:?}"),
                RuntimeMessage::PresentationSnapshot(snapshot) => {
                    presentation_lines.clone_from(&snapshot.history.logical_lines);
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
                RuntimeMessage::PresentationDelta(delta) => {
                    apply_presentation_delta(&mut presentation_lines, &delta.operations);
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
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::PresentationQuery
                        && request.operation == HTML_GET_PRINTED_STR_OPERATION =>
                {
                    let query: ProjectionStringIndexRequest =
                        decode_canonical(request.payload.as_slice())
                            .expect("decode restore printed HTML query");
                    followups.push(RuntimeMessage::ServiceResponse(ServiceResponse {
                        request_id: request.request_id,
                        result: ServiceResult::Ready {
                            payload: ProtocolBytes::new(
                                encode_canonical(&ProjectionStringResponse {
                                    context: query.context,
                                    value: headless_html_printed_str(
                                        &presentation_lines,
                                        query.index,
                                    ),
                                })
                                .expect("encode restore printed HTML response"),
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
    println!(
        "restore_start_to_wait_ms={}",
        start_started.elapsed().as_millis()
    );
    println!("restore_total_ms={}", total_started.elapsed().as_millis());
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

fn audit_service_capabilities() -> Vec<ServiceCapability> {
    vec![
        ServiceCapability {
            kind: ServiceKind::Clock,
            operation: LOCAL_DATE_TIME_OPERATION.into(),
            versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
        },
        ServiceCapability {
            kind: ServiceKind::PresentationQuery,
            operation: HTML_GET_PRINTED_STR_OPERATION.into(),
            versions: VersionRange::exact(HTML_GET_PRINTED_STR_OPERATION_VERSION),
        },
    ]
}

fn apply_presentation_delta(lines: &mut Vec<DisplayLine>, operations: &[PresentationOperation]) {
    for operation in operations {
        match operation {
            PresentationOperation::AppendLine { line } => lines.push(line.clone()),
            PresentationOperation::DeleteLines { count } => {
                lines.truncate(lines.len().saturating_sub(*count as usize));
            }
            PresentationOperation::Clear => lines.clear(),
            PresentationOperation::ReplaceLine { line_id, line } => {
                if let Some(current) = lines.iter_mut().find(|current| current.line_id == *line_id)
                {
                    current.clone_from(line);
                }
            }
            PresentationOperation::TrimLines { count } => {
                let count = (*count as usize).min(lines.len());
                lines.drain(..count);
            }
            PresentationOperation::SetTitle { .. }
            | PresentationOperation::ApplySceneDelta { .. }
            | PresentationOperation::SetAudio { .. }
            | PresentationOperation::SetInputWait { .. }
            | PresentationOperation::SetSettings { .. }
            | PresentationOperation::SetTooltip { .. }
            | PresentationOperation::SetResources { .. }
            | PresentationOperation::SetHtmlIsland { .. }
            | PresentationOperation::SetRedraw { .. }
            | PresentationOperation::SetButtonGeneration { .. } => {}
        }
    }
}

fn headless_html_printed_str(lines: &[DisplayLine], line_number: i64) -> String {
    let Ok(line_number) = usize::try_from(line_number) else {
        return String::new();
    };
    let mut logical_index = 0usize;
    let mut selected = Vec::new();
    for line in lines.iter().rev() {
        if logical_index == line_number {
            selected.push(line);
        }
        if line.logical_line_start {
            logical_index += 1;
        }
        if logical_index > line_number {
            break;
        }
    }
    if selected.is_empty() {
        return String::new();
    }
    selected.reverse();
    let alignment = match selected[0].alignment {
        LineAlignment::Left => "left",
        LineAlignment::Center => "center",
        LineAlignment::Right => "right",
    };
    let body = selected
        .into_iter()
        .map(|line| {
            let text = line.runs.iter().map(display_text).collect::<String>();
            escape_html(&text)
        })
        .collect::<Vec<_>>()
        .join("<br>");
    format!("<p align='{alignment}'><nobr>{body}</nobr></p>")
}

fn escape_html(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    for character in source.chars() {
        match character {
            '&' => result.push_str("&amp;"),
            '>' => result.push_str("&gt;"),
            '<' => result.push_str("&lt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(character),
        }
    }
    result
}

fn display_text(run: &era_runtime_protocol::DisplayRun) -> String {
    match run {
        era_runtime_protocol::DisplayRun::Text { text, .. }
        | era_runtime_protocol::DisplayRun::TextLayout { text, .. } => text.clone(),
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

fn try_collect(
    root: &Path,
    current: &Path,
    out: &mut Vec<String>,
    progress: &mut dyn FnMut(&Path, usize),
) -> std::io::Result<()> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "cannot read project directory {}: {error}",
                    current.display()
                ),
            )
        })?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot inspect project entry {}: {error}", path.display()),
            )
        })?;
        if file_type.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("project inventory rejects symbolic link {}", path.display()),
            ));
        }
        if file_type.is_dir() {
            try_collect(root, &path, out, progress)?;
        } else if file_type.is_file() {
            out.push(
                path.strip_prefix(root)
                    .map_err(|error| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("project entry {} escaped root: {error}", path.display()),
                        )
                    })?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
            progress(&path, out.len());
        }
    }
    Ok(())
}

fn try_collect_project_files(
    root: &Path,
    progress: &mut dyn FnMut(&Path, usize),
) -> std::io::Result<Vec<String>> {
    let mut paths = Vec::new();
    try_collect(root, root, &mut paths, progress)?;
    let inputs = project_inputs::ProjectInputs::new(root, &paths);
    paths.retain(|path| inputs.classify(path).is_some());
    paths.sort();
    Ok(paths)
}

fn collect_project_files(root: &Path) -> Vec<String> {
    try_collect_project_files(root, &mut |_, _| {})
        .unwrap_or_else(|error| panic!("cannot inventory project {}: {error}", root.display()))
}

fn has_direct_child_directory(root: &Path, expected: &str) -> bool {
    fs::read_dir(root).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.file_type().is_ok_and(|file_type| file_type.is_dir())
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(expected)
        })
    })
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
        maximum_envelope_bytes: 1024 * 1024 * 1024,
        maximum_payload_bytes: 1023 * 1024 * 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_project_files, decode_project_text, diagnostics_with_level, display_text,
        headless_html_printed_str, try_collect_project_files,
    };
    use era_runtime_protocol::{
        DisplayLine, DisplayRun, LineAlignment, ProtocolDiagnostic, RuntimeLogLevel, TextStyle,
    };
    use std::fs;

    fn text_line(
        line_id: u64,
        logical_line_start: bool,
        alignment: LineAlignment,
        text: &str,
    ) -> DisplayLine {
        DisplayLine {
            line_id,
            temporary: false,
            logical_line_start,
            line_end: true,
            alignment,
            text_background_eligible: !text.trim().is_empty(),
            runs: vec![DisplayRun::Text {
                text: text.into(),
                style: TextStyle::default(),
                system_text: None,
            }],
        }
    }

    #[test]
    fn project_text_decoder_prefers_utf8_and_strips_its_bom() {
        assert_eq!(
            decode_project_text(b"\xEF\xBB\xBFPRINTL \xE4\xBD\xA0\xE5\xA5\xBD").as_deref(),
            Some("PRINTL 你好")
        );
    }

    #[test]
    fn protocol_diagnostics_are_filtered_by_runtime_log_level() {
        let diagnostic = |code: &str, level| ProtocolDiagnostic {
            context: None,
            notification: era_runtime_protocol::DiagnosticNotification::default(),
            code: code.into(),
            level,
            message: String::new(),
            source: None,
        };
        let diagnostics = [
            diagnostic("warning", RuntimeLogLevel::Warning),
            diagnostic("error", RuntimeLogLevel::Error),
        ];

        assert_eq!(
            diagnostics_with_level(&diagnostics, RuntimeLogLevel::Error)
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["error"]
        );
    }

    #[test]
    fn project_text_decoder_falls_back_to_windows_31j() {
        let source = "サブディレクトリを検索する:YES";
        let (encoded, _, had_errors) = encoding_rs::SHIFT_JIS.encode(source);
        assert!(!had_errors);
        assert!(std::str::from_utf8(&encoded).is_err());
        assert_eq!(decode_project_text(&encoded).as_deref(), Some(source));
    }

    #[test]
    fn project_text_decoder_falls_back_to_gbk() {
        let source = ";阶层怪物列表\n#DIM KAI_LIST\n";
        let (encoded, _, had_errors) = encoding_rs::GBK.encode(source);
        assert!(!had_errors);
        assert!(std::str::from_utf8(&encoded).is_err());
        assert_eq!(decode_project_text(&encoded).as_deref(), Some(source));
    }

    #[test]
    fn project_text_decoder_rejects_invalid_supported_encodings() {
        assert_eq!(decode_project_text(b"\x81"), None);
    }

    #[test]
    fn project_collection_ignores_uninstalled_sources_beside_canonical_roots() {
        let root = std::env::temp_dir().join(format!(
            "rustyera-runtime-tester-project-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("CSV")).unwrap();
        fs::create_dir_all(root.join("ERB/GUIDE")).unwrap();
        fs::create_dir_all(root.join("GUIDE")).unwrap();
        fs::create_dir_all(root.join("patch/ERB")).unwrap();
        fs::write(root.join("CSV/GAMEBASE.CSV"), "コード,1\n").unwrap();
        fs::write(root.join("ERB/GUIDE/main.erb"), "@SYSTEM_TITLE\n").unwrap();
        fs::write(root.join("GUIDE/main.erb"), "@UNINSTALLED\n").unwrap();
        fs::write(root.join("patch/ERB/optional.erb"), "@UNINSTALLED\n").unwrap();
        fs::write(
            root.join("emuera.config"),
            "描画インターフェース:TEXTRENDERER",
        )
        .unwrap();

        let paths = collect_project_files(&root);

        assert_eq!(
            paths,
            ["CSV/GAMEBASE.CSV", "ERB/GUIDE/main.erb", "emuera.config"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn project_collection_rejects_symbolic_links() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "rustyera-runtime-tester-symlink-project-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("CSV")).unwrap();
        fs::write(root.join("outside.csv"), "コード,1\n").unwrap();
        symlink(root.join("outside.csv"), root.join("CSV/GAMEBASE.CSV")).unwrap();

        let error = try_collect_project_files(&root, &mut |_, _| {}).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("rejects symbolic link"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn headless_html_query_groups_the_newest_logical_line() {
        let lines = vec![
            text_line(1, true, LineAlignment::Left, "old"),
            text_line(2, true, LineAlignment::Center, "A&B"),
            text_line(3, false, LineAlignment::Center, "<tail>"),
        ];

        assert_eq!(
            headless_html_printed_str(&lines, 0),
            "<p align='center'><nobr>A&amp;B<br>&lt;tail&gt;</nobr></p>"
        );
        assert_eq!(
            headless_html_printed_str(&lines, 1),
            "<p align='left'><nobr>old</nobr></p>"
        );
        assert_eq!(headless_html_printed_str(&lines, 2), "");
    }

    #[test]
    fn display_text_accepts_frontend_projected_text_layout_runs() {
        let run = DisplayRun::TextLayout {
            text: "projected".into(),
            style: TextStyle::default(),
            system_text: None,
            columns: 9,
        };

        assert_eq!(display_text(&run), "projected");
    }
}
