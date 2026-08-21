#[test]
fn compiled_cache_export_does_not_retry_a_failed_background_build() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.compiled_cache_failure = Some("synthetic encoding failure".into());

    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();

    assert!(session.compiled_cache_task.is_none());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::ResourceLimit,
            message,
            ..
        }) if message.contains("synthetic encoding failure")
    )));
}

#[test]
fn project_load_rejects_an_uncommitted_cache_without_changing_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: 1,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(ProjectManifest {
                    project_revision: 1,
                    files: Vec::new(),
                }),
                compiled_cache_transfer_id: Some(123),
            },
        )
        .unwrap();

    assert_eq!(session.phase, RuntimePhase::Ready);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn identity_only_project_load_requests_payload_after_a_cache_miss() {
    let manifest = ProjectManifest {
        project_revision: 4,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let session = RuntimeSession::new(RuntimeOptions::default());

    let Err(report) = session.build_project_from_cache(
        ProjectLoadRequest {
            identity,
            manifest: None,
            compiled_cache_transfer_id: None,
        },
        None,
        None,
    ) else {
        panic!("an identity without an exact cache needs source payloads");
    };

    assert!(!report.success);
    assert!(report.payload_required);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_payload_required")
    );
}

#[test]
fn cold_project_load_reuses_the_owned_manifest_source_allocation() {
    let source = String::from("@SYSTEM_TITLE\nPRINTL MEMORY_STABLE\nRETURN\n");
    let source_pointer = source.as_ptr();
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(source),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.stage_project_manifest(manifest).unwrap();

    session
        .load_project(
            40,
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();

    let FilePayload::Utf8(retained) = &session
        .project_snapshot
        .as_ref()
        .expect("the project should load")
        .manifest
        .files[0]
        .payload
    else {
        panic!("the retained script payload should remain UTF-8");
    };
    assert_eq!(retained, "@SYSTEM_TITLE\nPRINTL MEMORY_STABLE\nRETURN\n");
    assert_eq!(retained.as_ptr(), source_pointer);
}

#[test]
fn low_memory_project_load_releases_source_payloads_but_preserves_identity() {
    let source = String::from("@SYSTEM_TITLE\nPRINTL MEMORY_STABLE\nRETURN\n");
    let digest = ProtocolBytes::new(blake3::hash(source.as_bytes()).as_bytes().to_vec());
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(source),
            content_hash: Some(digest),
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.stage_project_manifest(manifest).unwrap();

    session
        .load_project(
            40,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();

    let snapshot = session.project_snapshot.as_ref().expect("the project should load");
    assert_eq!(snapshot.manifest.project_revision, 1);
    assert_eq!(crate::compiled_cache::project_identity(&snapshot.manifest), identity);
    assert!(matches!(
        &snapshot.manifest.files[0].payload,
        FilePayload::Utf8(source) if source.is_empty() && source.capacity() == 0
    ));
}

#[test]
fn low_memory_full_payload_reload_remains_sparse_and_uses_the_new_source() {
    let initial = "@SYSTEM_TITLE\nPRINTL OLD\nRETURN\n";
    let initial_digest = ProtocolBytes::new(blake3::hash(initial.as_bytes()).as_bytes().to_vec());
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(initial.into()),
            content_hash: Some(initial_digest),
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.stage_project_manifest(manifest).unwrap();
    session
        .load_project(
            40,
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let _ = drain(&mut session);

    let changed = "@SYSTEM_TITLE\nPRINTL NEW\nRETURN\n";
    let changed_digest = ProtocolBytes::new(blake3::hash(changed.as_bytes()).as_bytes().to_vec());
    session
        .reload_project(
            41,
            &ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(changed.into()),
                        content_hash: Some(changed_digest.clone()),
                    },
                }],
            },
        )
        .unwrap();

    let snapshot = session.project_snapshot.as_ref().expect("the reload should commit");
    assert_eq!(snapshot.manifest.project_revision, 2);
    assert!(matches!(
        &snapshot.manifest.files[0].payload,
        FilePayload::Utf8(source) if source.is_empty() && source.capacity() == 0
    ));
    assert_eq!(snapshot.manifest.files[0].content_hash, Some(changed_digest));
    assert_eq!(
        session
            .artifact
            .as_ref()
            .unwrap()
            .artifact()
            .source_map
            .sources[0]
            .content_hash
            .0,
        *blake3::hash(changed.as_bytes()).as_bytes()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn low_memory_configuration_commit_preserves_sparse_sources_and_allows_full_reload() {
    let main = "@SYSTEM_TITLE\nRETURN\n";
    let other = "@OTHER\nRETURN\n";
    let configuration = "[meta]\nschema_version = 3\n[text]\nfont_size = 20\n";
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(main.into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "other.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(other.into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(configuration.into()),
                content_hash: None,
            },
        ],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.configuration_profile = ConfigurationClientProfile::Tui;
    session.stage_project_manifest(manifest).unwrap();
    session
        .load_project(
            50,
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let initial_configuration = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) if report.success => report.configuration,
            _ => None,
        })
        .expect("configuration snapshot");
    assert_eq!(
        initial_configuration.source_digest.as_slice(),
        blake3::hash(era_config::normalize_line_endings(configuration).as_bytes()).as_bytes()
    );
    let initial_identity = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .project_identity;
    session
        .prepare_configuration_update(
            51,
            &PrepareConfigurationUpdate {
                project_revision: 1,
                expected_source_digest: initial_configuration.source_digest,
                changes: vec![ConfigurationChange {
                    code: "MaxLog".into(),
                    value: "777".into(),
                }],
            },
        )
        .unwrap();
    let prepared = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdatePrepared(value) => Some(value),
            _ => None,
        })
        .expect("configuration preparation");
    session
        .finalize_configuration_update(
            52,
            FinalizeConfigurationUpdate {
                preparation_message_id: 51,
                outcome: ConfigurationUpdateOutcome::Commit,
            },
        )
        .unwrap();
    let _ = drain(&mut session);
    let committed = session.project_snapshot.as_ref().unwrap();
    assert_eq!(
        committed.configuration_snapshot().source_digest,
        prepared.prepared_source_digest
    );
    assert_ne!(committed.project_identity, initial_identity);
    assert!(committed.manifest.files.iter().filter(|file| {
        matches!(file.category, FileCategory::Erb | FileCategory::Erh | FileCategory::Csv)
    }).all(|file| matches!(&file.payload, FilePayload::Utf8(value) if value.is_empty())));

    session
        .reload_project(
            53,
            &ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![
                    FileChange::Upsert {
                        file: SubmittedFile {
                            relative_path: "main.erb".into(),
                            category: FileCategory::Erb,
                            payload: FilePayload::Utf8(main.into()),
                            content_hash: None,
                        },
                    },
                    FileChange::Upsert {
                        file: SubmittedFile {
                            relative_path: "other.erb".into(),
                            category: FileCategory::Erb,
                            payload: FilePayload::Utf8(other.into()),
                            content_hash: None,
                        },
                    },
                    FileChange::Upsert {
                        file: SubmittedFile {
                            relative_path: "reraconfig.toml".into(),
                            category: FileCategory::Configuration,
                            payload: FilePayload::Utf8(prepared.contents),
                            content_hash: None,
                        },
                    },
                ],
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report) if report.success && report.project_revision == 2
    )));
    let reloaded = session.project_snapshot.as_ref().unwrap();
    assert_eq!(reloaded.manifest.project_revision, 2);
    assert!(reloaded.manifest.files.iter().all(|file| matches!(
        &file.payload,
        FilePayload::Utf8(value) if value.is_empty() && value.capacity() == 0
    )));
}

#[test]
fn host_staged_manifest_is_owned_busy_single_use_and_identity_checked() {
    let manifest = ProjectManifest {
        project_revision: 4,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;

    session.stage_project_manifest(manifest.clone()).unwrap();
    assert!(matches!(
        session.stage_project_manifest(manifest.clone()),
        Err(RuntimeError::Busy(_))
    ));
    session
        .load_project(
            41,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let report = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(report.success, "{:?}", report.diagnostics);
    assert!(session.staged_project_manifest.is_none());

    session
        .load_project(
            42,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let missing = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(missing.payload_required);

    session.stage_project_manifest(manifest).unwrap();
    session
        .load_project(
            43,
            ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: identity.project_revision,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let mismatch = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(
        mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_identity_mismatch")
    );
    assert!(session.staged_project_manifest.is_none());
}

#[test]
fn rejected_or_explicit_project_load_discards_a_host_staged_manifest() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: Vec::new(),
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Running;
    session.stage_project_manifest(manifest.clone()).unwrap();

    session
        .load_project(
            51,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(session.staged_project_manifest.is_none());

    session.phase = RuntimePhase::Ready;
    session.stage_project_manifest(manifest.clone()).unwrap();
    session
        .load_project(
            52,
            ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(session.staged_project_manifest.is_none());
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_compiled_cache_load_does_not_require_a_manifest() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "CSV/GAMEBASE.CSV".into(),
                category: FileCategory::Csv,
                payload: FilePayload::Utf8(
                    "タイトル,Cached Demo\n作者,   \nバージョン,1001\n".into(),
                ),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(
                    "[meta]\r\nschema_version = 3\r\n[text]\r\nfont_size = 20\r\n".into(),
                ),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/sprites.csv".into(),
                category: FileCategory::ResourceManifest,
                payload: FilePayload::Utf8("; no sprites\n".into()),
                content_hash: None,
            },
        ],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.report.diagnostics.push(ProtocolDiagnostic {
        code: "compiler.cached_warning".into(),
        level: RuntimeLogLevel::Warning,
        message: "warning retained with compiled output".into(),
        source: Some(era_runtime_protocol::SourceLocation {
            relative_path: "main.erb".into(),
            byte_start: 0,
            byte_end: 13,
            line: Some(0),
            byte_column: Some(0),
        }),
    });
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let mut identity = crate::compiled_cache::project_identity(&manifest);
    identity.project_revision = 8;
    let expected_configuration_digest = initial
        .snapshot
        .as_ref()
        .unwrap()
        .configuration_snapshot()
        .source_digest;
    assert_eq!(
        expected_configuration_digest.as_slice(),
        blake3::hash(
            era_config::normalize_line_endings(
                "[meta]\r\nschema_version = 3\r\n[text]\r\nfont_size = 20\r\n"
            )
            .as_bytes()
        )
        .as_bytes()
    );
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&progress);
    session.project_progress_reporter = Some(ProjectProgressReporter::new(move |value| {
        observed.lock().unwrap().push(value);
    }));

    let cached = session
        .build_project_from_cache(
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
            None,
        )
        .expect("an exact cache loads from source identity alone");

    assert!(cached.report.success);
    assert!(!cached.report.payload_required);
    assert_eq!(cached.report.project_revision, 8);
    assert_eq!(
        cached.report.game_information,
        initial.report.game_information
    );
    let replayed = cached
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "compiler.cached_warning")
        .expect("cached compiler warning is replayed");
    assert_eq!(replayed.level, RuntimeLogLevel::Warning);
    assert_eq!(
        replayed.message,
        "[cached] warning retained with compiled output"
    );
    assert_eq!(replayed.source.as_ref().unwrap().byte_end, 13);
    let cached_snapshot = cached.snapshot.unwrap();
    assert_eq!(cached_snapshot.manifest.project_revision, 8);
    assert_eq!(
        cached_snapshot.configuration_snapshot().source_digest,
        expected_configuration_digest
    );
    assert!(cached_snapshot.manifest.files.iter().all(|file| match &file.payload {
        FilePayload::Utf8(value) => value.is_empty() && value.capacity() == 0,
        FilePayload::Bytes(value) => value.as_slice().is_empty(),
        FilePayload::IoError(_) | FilePayload::ExternalResource(_) => true,
    }));
    assert_exact_cache_preparing_progress(&progress);
}

fn assert_exact_cache_preparing_progress(progress: &Arc<std::sync::Mutex<Vec<ProjectProgress>>>) {
    let values = progress.lock().unwrap();
    let preparing: Vec<_> = values
        .iter()
        .filter(|value| value.stage == ProjectProgressStage::Preparing)
        .copied()
        .collect();
    assert_eq!(
        preparing,
        [
            ProjectProgress {
                stage: ProjectProgressStage::Preparing,
                completed: 0,
                total: 1,
            },
            ProjectProgress {
                stage: ProjectProgressStage::Preparing,
                completed: 1,
                total: 1,
            },
        ]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn host_staged_exact_cache_uses_the_normal_project_load_contract() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let identity = crate::compiled_cache::project_identity(&manifest);
    let expected_session = RuntimeSession::new(RuntimeOptions::default());
    let expected = expected_session
        .build_project_from_cache(
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
            None,
        )
        .unwrap()
        .report;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;
    let mismatch_cache = cache.clone();
    session.stage_project_manifest(manifest.clone()).unwrap();
    let transfer_id = session.stage_compiled_project_cache(cache).unwrap();

    session
        .load_project(
            44,
            ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();

    let mut report = None;
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
        let message = RuntimeMessage::from_envelope(&envelope).unwrap();
        assert!(!matches!(
            message,
            RuntimeMessage::StateImportAccepted(_) | RuntimeMessage::StateImportReady(_)
        ));
        if let RuntimeMessage::ProjectLoadReport(value) = message {
            assert_eq!(envelope.correlation_id, Some(44));
            report = Some(value);
        }
    }
    let report = report.expect("staged cache load emits a project report");
    assert_eq!(report.success, expected.success);
    assert_eq!(report.payload_required, expected.payload_required);
    assert_eq!(report.project_revision, expected.project_revision);
    assert_eq!(report.diagnostics, expected.diagnostics);
    assert_eq!(session.phase, RuntimePhase::Ready);
    assert!(session.inbound_transfer.is_none());
    assert!(session.staged_project_manifest.is_none());

    session
        .load_project(
            45,
            ProjectLoadRequest {
                identity: crate::compiled_cache::project_identity(&manifest),
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    let mut mismatch = RuntimeSession::new(RuntimeOptions::default());
    mismatch.phase = RuntimePhase::Ready;
    let transfer_id = mismatch
        .stage_compiled_project_cache(mismatch_cache)
        .unwrap();
    mismatch
        .load_project(
            46,
            ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: manifest.project_revision,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(manifest),
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();
    let mismatch_report = drain(&mut mismatch)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(!mismatch_report.success);
    assert!(!mismatch_report.payload_required);
    assert!(
        mismatch_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_identity_mismatch")
    );
    assert!(mismatch.inbound_transfer.is_none());
}

#[test]
fn host_staged_corrupt_cache_reports_a_normal_cache_miss() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;
    let transfer_id = session.stage_compiled_project_cache(vec![7; 64]).unwrap();

    session
        .load_project(
            51,
            ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: 9,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();

    let report = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(!report.success);
    assert!(report.payload_required);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_ignored")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_payload_required")
    );
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn compiled_cache_is_reused_across_configuration_profiles() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    for (producer, consumer) in [
        (
            ConfigurationClientProfile::Tui,
            ConfigurationClientProfile::Browser,
        ),
        (
            ConfigurationClientProfile::Browser,
            ConfigurationClientProfile::Tui,
        ),
    ] {
        let mut initial = build_project_with_extensions_and_progress(
            &manifest,
            None,
            None,
            &[],
            producer,
            None,
        );
        assert!(initial.report.success, "{:?}", initial.report.diagnostics);
        initial.incremental.compact();
        let cache = crate::compiled_cache::encode_compiled_cache_for_test(
            &manifest,
            &[],
            initial.artifact.as_ref().unwrap(),
            &initial.incremental,
            initial.snapshot.as_ref().unwrap(),
            &initial.report.diagnostics,
        )
        .unwrap();
        let mut identity = crate::compiled_cache::project_identity(&manifest);
        identity.project_revision = 9;
        let mut compact_manifest = manifest.clone();
        compact_manifest.project_revision = 9;
        compact_manifest.files[0].content_hash = Some(ProtocolBytes::new(
            blake3::hash(b"@SYSTEM_TITLE\nRETURN\n").as_bytes().to_vec(),
        ));
        compact_manifest.files[0].payload = FilePayload::Utf8(String::new());
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.configuration_profile = consumer;

        let build = session
            .build_project_from_cache(
                ProjectLoadRequest {
                    identity,
                    manifest: Some(compact_manifest),
                    compiled_cache_transfer_id: None,
                },
                Some(&cache),
                None,
            )
            .expect("a host-neutral compiled cache should load in either frontend profile");

        assert!(build.report.success);
        assert!(!build.report.payload_required);
        assert_eq!(build.report.project_revision, 9);
        assert!(
            build
                .report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit")
        );
        assert_eq!(build.snapshot.unwrap().configuration_profile, consumer);
    }
}

#[test]
fn journaled_configuration_rebuilds_instead_of_exact_hitting_the_old_artifact() {
    let old_configuration = "[audio]\nvolume = 100\n";
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(old_configuration.into()),
                content_hash: None,
            },
        ],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.incremental.compact();
    let mut cache = crate::compiled_cache::encode_full_project_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let old_key = crate::compiled_cache::decode(&cache, cache.len())
        .unwrap()
        .key;
    let expected = blake3::hash(old_configuration.as_bytes());
    let update = crate::compiled_cache::prepare_project_configuration_update(
        &cache,
        usize::MAX,
        expected.as_bytes(),
        "[audio]\nvolume = 42\n",
    )
    .unwrap();
    cache.extend_from_slice(&update.append);
    let request_identity = crate::compiled_cache::decode_project_file(&cache, cache.len())
        .unwrap()
        .identity;
    let expected_key = crate::compiled_cache::project_key(&request_identity, &[]);
    assert_ne!(old_key, expected_key);

    let session = RuntimeSession::new(RuntimeOptions::default());
    let rebuilt = session
        .build_project_from_cache(
            ProjectLoadRequest {
                identity: request_identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
            None,
        )
        .expect("journaled embedded manifest can safely rebuild the project");

    assert!(rebuilt.report.success, "{:?}", rebuilt.report.diagnostics);
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "runtime.compiled_cache_hit")
    );
    assert!(
        rebuilt
            .snapshot
            .unwrap()
            .manifest
            .files
            .iter()
            .any(|file| matches!(
                &file.payload,
                FilePayload::Utf8(source) if source == "[audio]\nvolume = 42\n"
            ))
    );
}
