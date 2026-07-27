//! Full-corpus source-extractor round trips through the public runtime protocol.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use era_protocol::{ProtocolBytes, VersionRange};
use era_runtime::{RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, FileCategory, FilePayload, InputModality, ProjectManifest,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeLogLevel, RuntimeMessage,
    SnapshotExportPurpose, StateExportChunkRequest, StateExportKind, StateExportRequest,
    StateExportResult, StorageCapabilities, SubmittedFile,
};

use super::{
    audit_service_capabilities, audit_wire_limits, collect_project_files, diagnostics_with_level,
    drain, drive, has_direct_child_directory, read_project_text, repository_root, submit,
    submit_with_epoch,
};

const CACHE_BUILD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EXPORT_CHUNK_BYTES: u32 = 16 * 1024 * 1024;

pub(super) fn audit_all_reference_games() {
    let extractor = env::args().nth(2).map_or_else(
        || {
            repository_root().join("target/debug").join(format!(
                "rustyera-source-extractor{}",
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
        "source extractor binary does not exist at {}",
        extractor.display()
    );
    println!("source_extractor_games={}", games.len());
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
    let files = submitted_sources(game);
    let expected = expected_sources(&files);
    let cache = compile_and_export(ProjectManifest {
        project_revision: 1,
        files,
    });
    let temporary = TemporaryDirectory::new();
    let cache_path = temporary.0.join("compiled-project-v5.bin.zst");
    let output_path = temporary.0.join("extracted");
    fs::write(&cache_path, &cache).expect("write temporary compiled cache");
    let output = std::process::Command::new(extractor)
        .arg(&cache_path)
        .arg(&output_path)
        .output()
        .expect("run source extractor");
    assert!(
        output.status.success(),
        "source extractor failed for {}\nstdout={}\nstderr={}",
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
            fs::read(output_path.join(&relative_path)).expect("read extracted source"),
            contents,
            "extracted contents differ for {}/{relative_path}",
            game.display()
        );
    }
    println!(
        "source_extractor_ok={} files={} cache_bytes={} elapsed_ms={}",
        game.file_name().unwrap().to_string_lossy(),
        actual_paths.len(),
        cache.len(),
        started.elapsed().as_millis()
    );
}

fn submitted_sources(root: &Path) -> Vec<SubmittedFile> {
    collect_project_files(root)
        .into_iter()
        .filter_map(|relative_path| {
            let lower = relative_path.to_ascii_lowercase();
            let category = if lower.ends_with(".erb") {
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
            let text = read_project_text(root.join(&relative_path))
                .expect("decode submitted project source");
            let hash = blake3::hash(text.as_bytes());
            Some(SubmittedFile {
                relative_path,
                category,
                payload: FilePayload::Utf8(text),
                content_hash: Some(ProtocolBytes::new(hash.as_bytes().to_vec())),
            })
        })
        .collect()
}

fn expected_sources(files: &[SubmittedFile]) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .filter_map(|file| {
            let FilePayload::Utf8(text) = &file.payload else {
                return None;
            };
            Some((file.relative_path.clone(), text.as_bytes().to_vec()))
        })
        .collect()
}

fn compile_and_export(manifest: ProjectManifest) -> Vec<u8> {
    let mut options = RuntimeOptions::default();
    options.limits.maximum_envelope_bytes = 128 * 1024 * 1024;
    options.limits.maximum_payload_bytes = 127 * 1024 * 1024;
    options.wire_limits = audit_wire_limits();
    let requested_limits = options.limits;
    let mut session = RuntimeSession::new(options);
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "source-extractor-audit".into(),
            features: vec![RuntimeFeature::StateResynchronization],
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
    assert!(
        drain(&mut session)
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ServerHello(_))),
        "runtime negotiation failed"
    );
    submit(&mut session, 1, RuntimeMessage::ProjectManifest(manifest));
    drive(&mut session);
    let messages = drain(&mut session);
    let report = messages.iter().find_map(|message| {
        let RuntimeMessage::ProjectLoadReport(report) = message else {
            return None;
        };
        Some(report)
    });
    let report = report.expect("runtime did not return a project load report");
    let errors =
        diagnostics_with_level(&report.diagnostics, RuntimeLogLevel::Error).collect::<Vec<_>>();
    assert!(report.success, "project compilation failed: {errors:#?}");

    submit_with_epoch(
        &mut session,
        2,
        Some(1),
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::CompiledProjectCache,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
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
        3,
        Some(1),
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::CompiledProjectCache,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
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
    let mut sequence = 4;
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
            "rustyera-source-extractor-audit-{}-{next}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create source extractor audit directory");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
