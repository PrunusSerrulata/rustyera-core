//! Full-corpus project-extractor round trips through the public runtime protocol.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use era_protocol::{ProtocolBytes, VersionRange, encode_canonical};
use era_runtime::{RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, FileCategory, FilePayload, IMAGE_METADATA_OPERATION,
    IMAGE_METADATA_OPERATION_VERSION, ImageMetadataResponse, InputModality, ProjectManifest,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeLogLevel, RuntimeMessage, ServiceCapability,
    ServiceKind, ServiceResponse, ServiceResult, SnapshotExportPurpose, StateExportChunkRequest,
    StateExportKind, StateExportRequest, StateExportResult, StorageCapabilities, SubmittedFile,
};

use super::{
    audit_service_capabilities, audit_wire_limits, diagnostics_with_level, drain, drive,
    has_direct_child_directory, read_project_text, repository_root, submit, submit_with_epoch,
    target_directory,
};

const CACHE_BUILD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EXPORT_CHUNK_BYTES: u32 = 16 * 1024 * 1024;

pub(super) fn audit_all_reference_games() {
    let extractor = env::args().nth(2).map_or_else(
        || {
            target_directory().join("debug").join(format!(
                "rustyera-project-extractor{}",
                env::consts::EXE_SUFFIX
            ))
        },
        PathBuf::from,
    );
    let reference_root = env::args()
        .nth(3)
        .map_or_else(|| repository_root().join("reference"), PathBuf::from);
    let games = discover_games(&reference_root);
    assert!(
        !games.is_empty(),
        "no Era game projects found below {}",
        reference_root.display()
    );
    assert!(
        extractor.is_file(),
        "project extractor binary does not exist at {}",
        extractor.display()
    );
    println!("project_extractor_games={}", games.len());
    for game in games {
        audit_game(&extractor, &game);
    }
}

fn discover_games(reference_root: &Path) -> Vec<PathBuf> {
    let mut games = fs::read_dir(reference_root)
        .expect("read reference directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("era"))
                && has_direct_child_directory(path, "CSV")
                && has_direct_child_directory(path, "ERB")
        })
        .collect::<Vec<_>>();
    games.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    games
}

fn audit_game(extractor: &Path, game: &Path) {
    let started = Instant::now();
    let files = submitted_project_files(game);
    let expected = expected_project_files(&files);
    let cache = compile_and_export(ProjectManifest {
        project_revision: 1,
        files,
    });
    let temporary = TemporaryDirectory::new();
    let cache_path = temporary.0.join("compiled-project-v8.bin.zst");
    let output_path = temporary.0.join("extracted");
    fs::write(&cache_path, &cache).expect("write temporary compiled cache");
    let output = std::process::Command::new(extractor)
        .arg(&cache_path)
        .arg(&output_path)
        .output()
        .expect("run project extractor");
    assert!(
        output.status.success(),
        "project extractor failed for {}\nstdout={}\nstderr={}",
        game.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_paths = collect_output_files(&output_path);
    assert_eq!(
        actual_paths,
        expected.keys().cloned().collect::<Vec<_>>(),
        "extracted path set differs for {}",
        game.display()
    );
    for (relative_path, contents) in expected {
        assert_eq!(
            fs::read(output_path.join(&relative_path)).expect("read extracted project file"),
            contents,
            "extracted contents differ for {}/{relative_path}",
            game.display()
        );
    }
    println!(
        "project_extractor_ok={} files={} cache_bytes={} elapsed_ms={}",
        game.file_name().unwrap().to_string_lossy(),
        actual_paths.len(),
        cache.len(),
        started.elapsed().as_millis()
    );
}

fn submitted_project_files(root: &Path) -> Vec<SubmittedFile> {
    collect_project_paths(root)
        .into_iter()
        .filter_map(|relative_path| {
            let lower = relative_path.to_ascii_lowercase();
            let first = lower.split('/').next().unwrap_or_default();
            let category = if first == "resources" && lower.ends_with(".csv") {
                FileCategory::ResourceManifest
            } else if first == "resources"
                && [".bmp", ".gif", ".jpeg", ".jpg", ".png", ".webp"]
                    .iter()
                    .any(|suffix| lower.ends_with(suffix))
            {
                FileCategory::Resource
            } else if lower.ends_with(".erb") {
                FileCategory::Erb
            } else if lower.ends_with(".erh") {
                FileCategory::Erh
            } else if lower.ends_with(".csv") {
                FileCategory::Csv
            } else if lower.ends_with(".config") {
                FileCategory::Configuration
            } else {
                return None;
            };
            let payload = if category == FileCategory::Resource {
                FilePayload::Bytes(ProtocolBytes::new(
                    fs::read(root.join(&relative_path)).expect("read submitted project asset"),
                ))
            } else {
                FilePayload::Utf8(
                    read_project_text(root.join(&relative_path))
                        .expect("decode submitted project source"),
                )
            };
            let hash = match &payload {
                FilePayload::Utf8(text) => blake3::hash(text.as_bytes()),
                FilePayload::Bytes(bytes) => blake3::hash(bytes.as_slice()),
                FilePayload::IoError(_) => unreachable!(),
            };
            Some(SubmittedFile {
                relative_path,
                category,
                payload,
                content_hash: Some(ProtocolBytes::new(hash.as_bytes().to_vec())),
            })
        })
        .collect()
}

fn expected_project_files(files: &[SubmittedFile]) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .map(|file| {
            let contents = match &file.payload {
                FilePayload::Utf8(text) => text.as_bytes().to_vec(),
                FilePayload::Bytes(bytes) => bytes.as_slice().to_vec(),
                FilePayload::IoError(_) => unreachable!(),
            };
            (file.relative_path.clone(), contents)
        })
        .collect()
}

fn collect_project_paths(root: &Path) -> Vec<String> {
    fn collect(root: &Path, current: &Path, paths: &mut Vec<String>) {
        for entry in fs::read_dir(current).expect("read project directory") {
            let entry = entry.expect("read project entry");
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, paths);
            } else {
                paths.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    let mut paths = Vec::new();
    collect(root, root, &mut paths);
    let has_csv_root = has_direct_child_directory(root, "CSV");
    let has_erb_root = has_direct_child_directory(root, "ERB");
    paths.retain(|relative| {
        let lower = relative.to_ascii_lowercase();
        let first = lower.split('/').next().unwrap_or_default();
        if first == "resources" {
            return lower.ends_with(".csv")
                || [".bmp", ".gif", ".jpeg", ".jpg", ".png", ".webp"]
                    .iter()
                    .any(|suffix| lower.ends_with(suffix));
        }
        if lower.ends_with(".csv") && has_csv_root {
            return first == "csv";
        }
        if (lower.ends_with(".erb") || lower.ends_with(".erh")) && has_erb_root {
            return first == "erb";
        }
        if lower.ends_with(".config") && has_csv_root && lower.contains('/') {
            return first == "csv";
        }
        lower.ends_with(".csv")
            || lower.ends_with(".erb")
            || lower.ends_with(".erh")
            || lower.ends_with(".config")
    });
    paths.sort();
    paths
}

fn compile_and_export(manifest: ProjectManifest) -> Vec<u8> {
    let mut options = RuntimeOptions::default();
    options.limits.maximum_envelope_bytes = 1024 * 1024 * 1024;
    options.limits.maximum_payload_bytes = 1023 * 1024 * 1024;
    options.wire_limits = audit_wire_limits();
    let requested_limits = options.limits;
    let mut session = RuntimeSession::new(options);
    let mut services = audit_service_capabilities();
    services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "project-extractor-audit".into(),
            features: vec![RuntimeFeature::StateResynchronization],
            requested_limits,
            capabilities: ClientCapabilities {
                input_modalities: vec![InputModality::Keyboard],
                rich_text: false,
                html: false,
                graphics: true,
                audio: false,
                video: false,
                font_metrics: false,
                column_cells: true,
                separators: true,
                available_fonts: Vec::new(),
                services,
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
    assert!(
        drain(&mut session)
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServerHello(_))),
        "runtime negotiation failed"
    );
    submit(&mut session, 1, RuntimeMessage::ProjectManifest(manifest));
    drive(&mut session);
    let mut sequence = 2;
    let report = loop {
        let messages = drain(&mut session);
        if let Some(report) = messages.iter().find_map(|message| {
            let RuntimeMessage::ProjectLoadReport(report) = message else {
                return None;
            };
            Some(report.clone())
        }) {
            break report;
        }
        let requests = messages
            .into_iter()
            .filter_map(|message| {
                let RuntimeMessage::ServiceRequest(request) = message else {
                    return None;
                };
                (request.kind == ServiceKind::Image
                    && request.operation == IMAGE_METADATA_OPERATION)
                    .then_some(request.request_id)
            })
            .collect::<Vec<_>>();
        assert!(
            !requests.is_empty(),
            "runtime did not return a project load report or image metadata request"
        );
        for request_id in requests {
            submit_with_epoch(
                &mut session,
                sequence,
                Some(1),
                RuntimeMessage::ServiceResponse(ServiceResponse {
                    request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(
                            encode_canonical(&ImageMetadataResponse {
                                width: 1,
                                height: 1,
                                format: "audit".into(),
                                animated: false,
                            })
                            .expect("encode image metadata response"),
                        ),
                    },
                }),
            );
            sequence += 1;
        }
        drive(&mut session);
    };
    let errors =
        diagnostics_with_level(&report.diagnostics, RuntimeLogLevel::Error).collect::<Vec<_>>();
    assert!(report.success, "project compilation failed: {errors:#?}");

    submit_with_epoch(
        &mut session,
        sequence,
        Some(1),
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::CompiledProjectCache,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    sequence += 1;
    drive(&mut session);
    let _ = drain(&mut session);
    let deadline = Instant::now() + CACHE_BUILD_TIMEOUT;
    loop {
        assert!(
            Instant::now() < deadline,
            "compiled project cache worker timed out"
        );
        drive(&mut session);
        let ready = drain(&mut session).into_iter().any(|message| {
            matches!(
                message,
                RuntimeMessage::Diagnostic(diagnostic)
                    if diagnostic.code == "runtime.compiled_cache_ready"
            )
        });
        if ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    submit_with_epoch(
        &mut session,
        sequence,
        Some(1),
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::CompiledProjectCache,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    sequence += 1;
    drive(&mut session);
    let transfer = drain(&mut session)
        .into_iter()
        .find_map(|message| {
            let RuntimeMessage::StateExportReady(ready) = message else {
                return None;
            };
            let StateExportResult::Ready { transfer } = ready.result else {
                return None;
            };
            Some(transfer)
        })
        .expect("runtime did not make the compiled cache transfer ready");
    let mut bytes = Vec::with_capacity(
        usize::try_from(transfer.total_bytes).expect("compiled cache length is addressable"),
    );
    loop {
        submit_with_epoch(
            &mut session,
            sequence,
            Some(1),
            RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
                transfer_id: transfer.transfer_id,
                offset: bytes.len() as u64,
                maximum_bytes: EXPORT_CHUNK_BYTES,
            }),
        );
        sequence += 1;
        drive(&mut session);
        let chunk = drain(&mut session)
            .into_iter()
            .find_map(|message| {
                let RuntimeMessage::StateExportChunk(chunk) = message else {
                    return None;
                };
                Some(chunk)
            })
            .expect("runtime did not return a compiled cache chunk");
        assert_eq!(chunk.offset, bytes.len() as u64);
        bytes.extend_from_slice(chunk.data.as_slice());
        if chunk.complete {
            break;
        }
    }
    assert_eq!(bytes.len() as u64, transfer.total_bytes);
    assert_eq!(blake3::hash(&bytes).as_bytes(), transfer.digest.as_slice());
    bytes
}

fn collect_output_files(root: &Path) -> Vec<String> {
    fn collect(root: &Path, current: &Path, paths: &mut Vec<String>) {
        for entry in fs::read_dir(current).expect("read extracted source directory") {
            let entry = entry.expect("read extracted source entry");
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, paths);
            } else {
                paths.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut paths = Vec::new();
    collect(root, root, &mut paths);
    paths.sort();
    paths
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "rustyera-project-extractor-audit-{}-{next}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create project extractor audit directory");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
