//! Full-corpus project-extractor round trips through the public runtime protocol.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use era_protocol::{ProtocolBytes, VersionRange, encode_canonical};
use era_runtime::{RuntimeOptions, RuntimeSession};
use era_runtime_protocol::{
    ClientCapabilities, ClientHello, FilePayload, FullProjectManifest, IMAGE_METADATA_OPERATION,
    IMAGE_METADATA_OPERATION_VERSION, ImageMetadataResponse, InputModality, ProjectManifest,
    RUNTIME_PROTOCOL_VERSION, RuntimeFeature, RuntimeLogLevel, RuntimeMessage, ServiceCapability,
    ServiceKind, ServiceResponse, ServiceResult, SnapshotExportPurpose, StateExportChunkRequest,
    StateExportKind, StateExportRequest, StateExportResult, StorageCapabilities, SubmittedFile,
};

use super::{
    audit_service_capabilities, audit_wire_limits, collect_project_files, diagnostics_with_level,
    drain, drive, has_direct_child_directory, project_inputs::ProjectInputs, repository_root,
    submit, submit_with_epoch, target_directory,
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
    let project_file = compile_and_export(ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files,
    });
    let temporary = TemporaryDirectory::new();
    let project_path = temporary.0.join("compiled-project.reraproj");
    let output_path = temporary.0.join("extracted");
    fs::write(&project_path, &project_file).expect("write temporary full project file");
    let output = std::process::Command::new(extractor)
        .arg(&project_path)
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
        "project_extractor_ok={} files={} project_bytes={} elapsed_ms={}",
        game.file_name().unwrap().to_string_lossy(),
        actual_paths.len(),
        project_file.len(),
        started.elapsed().as_millis()
    );
}

fn submitted_project_files(root: &Path) -> Vec<SubmittedFile> {
    let paths = collect_project_files(root);
    ProjectInputs::new(root, &paths).submitted_files(root, &paths, true)
}

fn expected_project_files(files: &[SubmittedFile]) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .map(|file| {
            let contents = match &file.payload {
                FilePayload::Utf8(text) => text.as_bytes().to_vec(),
                FilePayload::Bytes(bytes) => bytes.as_slice().to_vec(),
                FilePayload::IoError(_) | FilePayload::ExternalResource(_) => unreachable!(),
            };
            (file.relative_path.clone(), contents)
        })
        .collect()
}

fn compile_and_export(manifest: ProjectManifest) -> Vec<u8> {
    let full_manifest = manifest.clone();
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
            configuration_profile: None,
            features: vec![RuntimeFeature::StateResynchronization],
            requested_limits,
            capabilities: ClientCapabilities {
                environment: Vec::new(),
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
        RuntimeMessage::FullProjectManifest(FullProjectManifest {
            manifest: full_manifest,
        }),
    );
    sequence += 1;
    drive(&mut session);
    let _ = drain(&mut session);

    submit_with_epoch(
        &mut session,
        sequence,
        Some(1),
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::FullProjectFile,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    sequence += 1;
    drive(&mut session);
    let _ = drain(&mut session);
    let deadline = Instant::now() + CACHE_BUILD_TIMEOUT;
    let transfer = loop {
        assert!(Instant::now() < deadline, "full project worker timed out");
        drive(&mut session);
        let _ = drain(&mut session);
        submit_with_epoch(
            &mut session,
            sequence,
            Some(1),
            RuntimeMessage::StateExportRequest(StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            }),
        );
        sequence += 1;
        drive(&mut session);
        if let Some(transfer) = drain(&mut session).into_iter().find_map(|message| {
            let RuntimeMessage::StateExportReady(ready) = message else {
                return None;
            };
            let StateExportResult::Ready { transfer } = ready.result else {
                return None;
            };
            Some(transfer)
        }) {
            break transfer;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut bytes = Vec::with_capacity(
        usize::try_from(transfer.total_bytes).expect("full project length is addressable"),
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
            .expect("runtime did not return a full project chunk");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_inputs::DataRoot;
    use era_runtime_protocol::FileCategory;

    fn write_input(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn collector_and_extractor_include_data_resources_and_exclude_outside_indices() {
        let directory = TemporaryDirectory::new();
        let root = &directory.0;
        for path in [
            "resources/schema.xml",
            "resources/story.txt",
            "resources/seed.db",
            "resources/seed.sqlite",
        ] {
            write_input(root, path, [0xff, 0x00, 0x42]);
        }
        write_input(root, "CSV/BUFF.csv", "0,csv_main\n");
        write_input(root, "CSV/BUFF.als", "11,csv_alias\n");
        write_input(root, "ERB/BUFF.erd", "0,erd_main\n");
        write_input(root, "ERB/BUFF.als", "10,erb_alias\n");
        for path in [
            "backup/BUFF.als",
            "backup/BUFF.erd",
            "CSV/BUFF.erd",
            "data/overlay.xml",
            "sav/save.txt",
            "logs/a.db",
            ".rustyera/cache.sqlite",
            "plugins/ignored.dll",
        ] {
            write_input(root, path, "ignored");
        }
        let paths = collect_project_files(root);
        let files = submitted_project_files(root);
        assert_eq!(
            files
                .iter()
                .map(|file| &file.relative_path)
                .collect::<Vec<_>>(),
            paths.iter().collect::<Vec<_>>()
        );
        assert_eq!(paths.len(), 8);
        for file in files
            .iter()
            .filter(|file| file.relative_path.starts_with("resources/"))
        {
            assert_eq!(file.category, FileCategory::Resource);
            assert_eq!(
                file.payload,
                FilePayload::Bytes(ProtocolBytes::new(vec![0xff, 0x00, 0x42]))
            );
            assert_eq!(
                file.content_hash.as_ref().unwrap().as_slice(),
                blake3::hash(&[0xff, 0x00, 0x42]).as_bytes()
            );
        }
    }

    #[test]
    fn same_stem_csv_and_erb_aliases_remain_distinct_in_minimal_and_extracted_inputs() {
        let directory = TemporaryDirectory::new();
        let root = &directory.0;
        for (path, content) in [
            ("CSV/BUFF.csv", "0,csv_main\n"),
            ("CSV/BUFF.als", "11,csv_alias\n"),
            ("ERB/BUFF.erd", "0,erd_main\n"),
            ("ERB/BUFF.als", "10,erb_alias\n"),
        ] {
            write_input(root, path, content);
        }
        let paths = collect_project_files(root);
        let inputs = ProjectInputs::new(root, &paths);
        let minimal = inputs.submitted_files(root, &paths, false);
        assert_eq!(minimal, submitted_project_files(root));
        let mut csv_files = erabasic_csv::ProjectFiles::default();
        for file in minimal {
            let data_root = inputs
                .data_root(&file.relative_path, file.category)
                .unwrap();
            let FilePayload::Utf8(text) = file.payload else {
                panic!("index input must be UTF-8")
            };
            let frontend_file = erabasic_csv::FrontendFile {
                relative_path: data_root.relative_path(&file.relative_path),
                source_path: Some(file.relative_path),
                payload: erabasic_csv::FilePayload::Utf8(text),
            };
            match data_root {
                DataRoot::Csv => csv_files.csv.push(frontend_file),
                DataRoot::Erb => csv_files.erb.push(frontend_file),
            }
        }
        let options = erabasic_csv::CsvLoadOptions {
            compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
                erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            ),
            use_erd: true,
            ..Default::default()
        };
        let mut project = erabasic_csv::load_project(&csv_files, &options)
            .data
            .unwrap();
        let diagnostics = erabasic_csv::resolve_deferred_indices(
            &mut project,
            &[erabasic_data::UserIndexRegistration {
                variable_name: "BUFF".into(),
                source_stem: "BUFF".into(),
                dimension: None,
                length: 50,
            }],
            &options,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let resolved = &project.static_data.deferred_indices.resolved["BUFF"];
        assert_eq!(resolved.entries["csv_alias"], 11);
        assert_eq!(resolved.entries["erb_alias"], 10);
    }

    #[test]
    fn new_index_inputs_reject_legacy_encoding_without_changing_csv_decoding() {
        let directory = TemporaryDirectory::new();
        for (path, category) in [
            ("BUFF.als", FileCategory::Als),
            ("BUFF.erd", FileCategory::Erd),
        ] {
            write_input(&directory.0, path, b"0,\x82\xa0\n");
            let error = crate::read_submitted_text(directory.0.join(path), category).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        }
        write_input(&directory.0, "BUFF.csv", b"0,\x82\xa0\n");
        assert_eq!(
            crate::read_submitted_text(directory.0.join("BUFF.csv"), FileCategory::Csv).unwrap(),
            "0,あ\n"
        );
    }
}
