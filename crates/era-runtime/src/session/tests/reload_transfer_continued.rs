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
                    compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
                    configuration_digest: None,
                    project_revision: 1,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(ProjectManifest {
                    compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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

    let snapshot = session
        .project_snapshot
        .as_ref()
        .expect("the project should load");
    assert_eq!(snapshot.manifest.project_revision, 1);
    assert_eq!(
        crate::compiled_cache::project_identity(&snapshot.manifest),
        identity
    );
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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

    let snapshot = session
        .project_snapshot
        .as_ref()
        .expect("the reload should commit");
    assert_eq!(snapshot.manifest.project_revision, 2);
    assert!(matches!(
        &snapshot.manifest.files[0].payload,
        FilePayload::Utf8(source) if source.is_empty() && source.capacity() == 0
    ));
    assert_eq!(
        snapshot.manifest.files[0].content_hash,
        Some(changed_digest)
    );
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
    let initial_identity = session.project_snapshot.as_ref().unwrap().project_identity;
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
    assert!(
        committed
            .manifest
            .files
            .iter()
            .filter(|file| {
                matches!(
                    file.category,
                    FileCategory::Erb | FileCategory::Erh | FileCategory::Csv
                )
            })
            .all(|file| matches!(&file.payload, FilePayload::Utf8(value) if value.is_empty()))
    );

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
    assert!(
        reloaded
            .manifest
            .files
            .iter()
            .all(|file| match &file.payload {
                FilePayload::Utf8(value) if file.relative_path == "reraconfig.toml" =>
                    !value.is_empty(),
                FilePayload::Utf8(value) => value.is_empty() && value.capacity() == 0,
                _ => false,
            })
    );
}

#[test]
fn host_staged_manifest_is_owned_busy_single_use_and_identity_checked() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                    compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
                    configuration_digest: None,
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                    "[meta]\r\nschema_version = 4\r\n[text]\r\nfont_size = 20\r\n".into(),
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
        context: None,
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
        notification: DiagnosticNotification::default(),
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
                "[meta]\r\nschema_version = 4\r\n[text]\r\nfont_size = 20\r\n"
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
    assert_eq!(replayed.notification, DiagnosticNotification::LogOnly);
    assert_eq!(
        replayed.message,
        "warning retained with compiled output"
    );
    assert_eq!(replayed.source.as_ref().unwrap().byte_end, 13);
    let cached_snapshot = cached.snapshot.unwrap();
    assert_eq!(cached_snapshot.manifest.project_revision, 8);
    assert_eq!(
        cached_snapshot.configuration_snapshot().source_digest,
        expected_configuration_digest
    );
    assert!(
        cached_snapshot
            .manifest
            .files
            .iter()
            .all(|file| match &file.payload {
                FilePayload::Utf8(value) if file.relative_path == "reraconfig.toml" => {
                    !value.is_empty() && era_config::ReraConfigDocument::parse(value).is_ok()
                }
                FilePayload::Utf8(value) => value.is_empty() && value.capacity() == 0,
                FilePayload::Bytes(value) => value.as_slice().is_empty(),
                FilePayload::IoError(_) | FilePayload::ExternalResource(_) => true,
            })
    );
    assert_exact_cache_preparing_progress(&progress);
}

fn compatibility_cache_fixture() -> (ProjectManifest, Vec<u8>) {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                payload: FilePayload::Utf8("[meta]\nschema_version = 4\n".into()),
                content_hash: None,
            },
        ],
    };
    let mut build = crate::project::build_project(&manifest, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    build.incremental.compact();
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
        &manifest,
        &[],
        build.artifact.as_ref().unwrap(),
        &build.incremental,
        build.snapshot.as_ref().unwrap(),
        &build.report.diagnostics,
    )
    .unwrap();
    (manifest, cache)
}

#[test]
fn profileless_cache_is_never_executed_and_rebuilds_from_submitted_sources() {
    let (manifest, mut old_cache) = compatibility_cache_fixture();
    old_cache[8] = 8;
    let checksum_start = old_cache.len() - 32;
    let checksum = blake3::hash(&old_cache[..checksum_start]);
    old_cache[checksum_start..].copy_from_slice(checksum.as_bytes());
    let rebuilt = RuntimeSession::new(RuntimeOptions::default())
        .build_project_from_cache(
            ProjectLoadRequest {
                identity: crate::compiled_cache::project_identity(&manifest),
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
            Some(&old_cache),
            None,
        )
        .expect("old cache must fall back to the full supplied source manifest");
    assert!(rebuilt.report.success, "{:?}", rebuilt.report.diagnostics);
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_ignored")
    );
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "runtime.compiled_cache_hit")
    );
    assert_eq!(
        rebuilt.artifact.unwrap().artifact().manifest.compatibility,
        erabasic_compat::CompatibilityIdentity::reference()
    );
}

#[test]
fn invalid_profile_configuration_precedes_cache_consumption_and_cannot_fallback() {
    let (manifest, cache) = compatibility_cache_fixture();
    let mut session = negotiated_session();
    session
        .load_project(
            100,
            ProjectLoadRequest {
                identity: crate::compiled_cache::project_identity(&manifest),
                manifest: Some(manifest.clone()),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    drain(&mut session);
    let artifact_id = session
        .artifact
        .as_ref()
        .unwrap()
        .artifact()
        .manifest
        .artifact_id;
    let old_revision = session.project_revision();
    let transfer_id = session.stage_compiled_project_cache(cache).unwrap();
    let mut invalid = manifest;
    invalid.project_revision += 1;
    invalid.files[1].payload = FilePayload::Utf8(
        "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"unknown.profile\"\n".into(),
    );
    session
        .load_project(
            101,
            ProjectLoadRequest {
                identity: crate::compiled_cache::project_identity(&invalid),
                manifest: Some(invalid),
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();
    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::ProjectLoadReport(report) if !report.success && !report.payload_required
            && report.diagnostics.iter().any(|diagnostic| diagnostic.code == "runtime.invalid_reraconfig")
            && report.diagnostics.iter().all(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
    )), "{messages:?}");
    assert_eq!(session.phase(), RuntimePhase::Ready);
    assert_eq!(session.project_revision(), old_revision);
    assert_eq!(
        session
            .artifact
            .as_ref()
            .unwrap()
            .artifact()
            .manifest
            .artifact_id,
        artifact_id
    );
    assert_eq!(
        session
            .inbound_transfer
            .as_ref()
            .unwrap()
            .descriptor
            .transfer_id,
        transfer_id
    );
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                    compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
                    configuration_digest: None,
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
                    compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
                    configuration_digest: None,
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
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        let mut initial =
            build_project_with_extensions_and_progress(&manifest, None, None, &[], producer, None);
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
fn compiled_cache_with_a_stale_configuration_type_is_rebuilt_from_sources() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                payload: FilePayload::Utf8(
                    "[meta]\nschema_version = 3\n\n[interface]\nmenu_mode = \"auto\"\n".into(),
                ),
                content_hash: None,
            },
        ],
    };
    let mut initial = build_project_with_extensions_and_progress(
        &manifest,
        None,
        None,
        &[],
        ConfigurationClientProfile::Tui,
        None,
    );
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    let snapshot = initial.snapshot.as_mut().unwrap();
    let mut serialized = serde_json::to_value(&snapshot.configuration).unwrap();
    serialized["values"]["USEMENU"] = serde_json::json!({ "Boolean": true });
    snapshot.configuration = serde_json::from_value(serialized).unwrap();
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        snapshot,
        &initial.report.diagnostics,
    )
    .unwrap();

    let identity = crate::compiled_cache::project_identity(&manifest);
    let rebuilt = RuntimeSession::new(RuntimeOptions::default())
        .build_project_from_cache(
            ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
            None,
        )
        .expect("an incompatible cache should be rebuilt from its project sources");

    assert!(rebuilt.report.success, "{:?}", rebuilt.report.diagnostics);
    assert!(rebuilt.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "runtime.compiled_cache_ignored"
            && diagnostic.message.contains("interface.menu_mode")
    }));
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "runtime.compiled_cache_hit")
    );
    let configuration = rebuilt.snapshot.unwrap().configuration_snapshot();
    assert!(!configuration.restart_pending);
    assert_eq!(
        configuration
            .entries
            .iter()
            .find(|entry| entry.code == "UseMenu")
            .map(|entry| entry.effective_value.as_str()),
        Some("AUTO")
    );
}

#[test]
fn journaled_configuration_rebuilds_instead_of_exact_hitting_the_old_artifact() {
    let old_configuration = "[audio]\nvolume = 100\n";
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
                FilePayload::Utf8(source) if source == &era_config::ReraConfigDocument::parse("[audio]\nvolume = 42\n").unwrap().to_lf_string()
            ))
    );
}


fn static_call_diagnostic_manifest() -> ProjectManifest {
    ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::for_profile(
            era_runtime_protocol::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(), category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nCALL TARGET, 1, 2\nRETURN\n@TARGET(ARG)\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "reraconfig.toml".into(), category: FileCategory::Configuration,
                payload: FilePayload::Utf8("[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n".into()),
                content_hash: None,
            },
        ],
    }
}

#[test]
fn exact_compiled_cache_replays_static_call_diagnostics_without_accumulating_provenance() {
    let manifest = static_call_diagnostic_manifest();
    for full_project in [false, true] {
        let mut build = build_project_with_extensions_and_progress(
            &manifest, None, None, &[], ConfigurationClientProfile::Tui, None,
        );
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        let expected = build.report.diagnostics.iter().filter(|value| {
            value.code == "compat.call.excess_arguments"
        }).cloned().collect::<Vec<_>>();
        assert_eq!(expected.len(), 1);
        assert!(expected[0].source.is_some());
        assert_eq!(expected[0].context.as_ref().unwrap().identity.as_ref(), Some(&manifest.compatibility));
        for revision in [2, 3] {
            // Match commit_cold_project_load/commit_project_reload: cache notices themselves
            // are transient; the original source diagnostic remains the persistent plan.
            let persistent = build.report.diagnostics.iter().filter(|value| {
                !value.code.starts_with("runtime.compiled_cache_")
            }).cloned().collect::<Vec<_>>();
            let snapshot = build.snapshot.as_ref().unwrap();
            let encode = if full_project {
                crate::compiled_cache::encode_full_project_for_test
            } else {
                crate::compiled_cache::encode_compiled_cache_for_test
            };
            // A full project archive needs the original source payloads; an exact
            // warm compact cache intentionally retains only their hashes/offsets.
            let mut full_manifest = manifest.clone();
            full_manifest.project_revision = snapshot.manifest.project_revision;
            let packaging_manifest = if full_project { &full_manifest } else { &snapshot.manifest };
            let bytes = encode(
                packaging_manifest, &[], build.artifact.as_ref().unwrap(), &build.incremental,
                snapshot, &persistent,
            ).unwrap_or_else(|error| panic!("full={full_project}, revision={revision}: {error}"));
            let decoded = crate::compiled_cache::decode(&bytes, bytes.len()).unwrap();
            let mut identity = crate::compiled_cache::project_identity(&snapshot.manifest);
            identity.project_revision = revision;
            let session = RuntimeSession::new(RuntimeOptions::default());
            build = session.build_project_from_cache(
                ProjectLoadRequest { identity, manifest: None, compiled_cache_transfer_id: None },
                None, Some(decoded),
            ).expect("exact persistent artifact needs no source reanalysis");
            assert!(build.report.success);
            assert_eq!(build.report.project_revision, revision);
            assert!(build.report.diagnostics.iter().any(|value| value.code == "runtime.compiled_cache_hit"));
            let replayed = build.report.diagnostics.iter().filter(|value| {
                value.code == "compat.call.excess_arguments"
            }).cloned().collect::<Vec<_>>();
            assert_eq!(replayed, expected);
        }
    }
}

fn take_static_call_report(messages: &[RuntimeMessage]) -> ProjectLoadReport {
    let reports = messages.iter().filter_map(|message| match message {
        RuntimeMessage::ProjectLoadReport(report) => Some(report.clone()), _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(reports.len(), 1, "{messages:?}");
    assert!(!messages.iter().any(|message| matches!(message,
        RuntimeMessage::Diagnostic(diagnostic) if diagnostic.code == "compat.call.excess_arguments"
    )), "static calls must not duplicate their load report through VM warning events");
    reports.into_iter().next().unwrap()
}

fn static_call_warnings(report: &ProjectLoadReport) -> Vec<&ProtocolDiagnostic> {
    report.diagnostics.iter().filter(|diagnostic| diagnostic.code == "compat.call.excess_arguments").collect()
}

fn assert_published_call_warning(session: &RuntimeSession, report: &ProjectLoadReport, generation: Option<u64>) {
    assert!(report.success, "{:?}", report.diagnostics);
    let warnings = static_call_warnings(report);
    assert_eq!(warnings.len(), 1);
    let warning = warnings[0];
    assert_eq!(warning.level, RuntimeLogLevel::Warning);
    assert_eq!(warning.notification, DiagnosticNotification::LogOnly);
    let source = warning.source.as_ref().unwrap();
    assert_eq!(source.relative_path, "main.erb");
    assert!(source.byte_end > source.byte_start);
    let context = warning.context.as_ref().unwrap();
    assert_eq!(context.identity.as_ref(), report.compatibility.as_ref());
    assert_eq!(context.stage, "compat");
    assert_eq!(context.api.as_deref(), Some("user_call"));
    assert_eq!(context.artifact.as_ref().unwrap().as_slice(), session.artifact.as_ref().unwrap().artifact().manifest.artifact_id.0);
    assert_eq!(context.project_load_id, Some(session.project_load_id));
    assert_eq!(context.runtime_epoch, Some(session.epoch.0));
    assert_eq!(context.generation, generation);
}

fn load_static_call_project(session: &mut RuntimeSession, message_id: u64, manifest: ProjectManifest) -> ProjectLoadReport {
    session.load_project(message_id, ProjectLoadRequest {
        identity: crate::compiled_cache::project_identity(&manifest),
        manifest: Some(manifest), compiled_cache_transfer_id: None,
    }).unwrap();
    take_static_call_report(&drain(session))
}

#[test]
fn static_call_publication_survives_real_cold_and_warm_full_cache_loads_without_vm_duplicate() {
    for full_project in [false, true] {
        let mut manifest = static_call_diagnostic_manifest();
        let FilePayload::Utf8(source) = &mut manifest.files[0].payload else { unreachable!() };
        *source = source.replace("CALL TARGET, 1, 2\nRETURN", "CALL TARGET, 1, 2\nWAIT\nRETURN");
        // A compact cache omits source payloads; a later source reload must supply them.
        let reload_source = manifest.files[0].clone();
        let mut cold = negotiated_session();
        let cold_report = load_static_call_project(&mut cold, 100, manifest);
        assert_published_call_warning(&cold, &cold_report, None);
        assert!(cold.vm.is_none());
        let snapshot = cold.project_snapshot.as_ref().unwrap();
        let encode = if full_project { crate::compiled_cache::encode_full_project_for_test } else { crate::compiled_cache::encode_compiled_cache_for_test };
        // Feed the already published copy deliberately: neither packaging backend may
        // preserve live publication scope in the reusable diagnostics section.
        let bytes = encode(&snapshot.manifest, &[], cold.artifact.as_ref().unwrap(), &cold.incremental, snapshot, &cold_report.diagnostics).unwrap();
        let decoded = crate::compiled_cache::decode(&bytes, bytes.len()).unwrap();
        let cached = decoded.diagnostics.iter().find(|diagnostic| diagnostic.code == "compat.call.excess_arguments").unwrap();
        let context = cached.context.as_ref().unwrap();
        assert_eq!((&context.artifact, context.project_load_id, context.runtime_epoch, context.generation), (&None, None, None, None));
        assert_eq!(cached.source, static_call_warnings(&cold_report)[0].source);
        let identity = crate::compiled_cache::project_identity(&snapshot.manifest);
        let mut warm = negotiated_session();
        let transfer_id = warm.stage_compiled_project_cache(bytes).unwrap();
        warm.load_project(101, ProjectLoadRequest { identity, manifest: None, compiled_cache_transfer_id: Some(transfer_id) }).unwrap();
        let warm_report = take_static_call_report(&drain(&mut warm));
        assert_published_call_warning(&warm, &warm_report, None);
        assert!(warm_report.diagnostics.iter().any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit"));
        assert_eq!(static_call_warnings(&warm_report)[0].source, static_call_warnings(&cold_report)[0].source);
        assert_eq!(warm.compiled_cache_diagnostics.iter().filter(|diagnostic| diagnostic.code == "compat.call.excess_arguments").count(), 1);
        assert!(warm.compiled_cache_diagnostics.iter().all(|diagnostic| diagnostic.context.as_ref().is_none_or(|context| context.artifact.is_none() && context.generation.is_none() && context.project_load_id.is_none() && context.runtime_epoch.is_none())));
        warm.emit_committed_project_report(102, warm_report.clone(), None).unwrap();
        assert!(static_call_warnings(&take_static_call_report(&drain(&mut warm))).is_empty());
        warm.start(103, &StartRequest { mode: StartMode::NewGame { seed: Some(7) } }).unwrap();
        for _ in 0..8 { warm.drive(RuntimeDriveBudget::default()).unwrap(); }
        let execution = drain(&mut warm);
        assert!(!execution.iter().any(|message| matches!(message,
            RuntimeMessage::Diagnostic(diagnostic) if diagnostic.code == "compat.call.excess_arguments"
        )));
        assert_eq!(warm.vm.as_ref().unwrap().current_generation().0, 1);
        assert_eq!(warm.phase, RuntimePhase::WaitingInput);
        let before_restore = warm.project_diagnostic_publication.clone().unwrap();
        warm.export_state(104, StateExportRequest { kind: StateExportKind::VmSnapshot, snapshot_purpose: SnapshotExportPurpose::Normal }).unwrap();
        let export_messages = drain(&mut warm);
        let snapshot = warm.outbound_transfer.take().unwrap_or_else(|| panic!("snapshot export: {export_messages:?}")).bytes;
        warm.start_vm_snapshot(105, &snapshot).unwrap();
        assert_eq!(warm.project_load_id, 1);
        assert!(warm.epoch.0 > before_restore.scope.runtime_epoch);
        assert_eq!(warm.project_diagnostic_publication.as_ref().unwrap().scope, before_restore.scope);
        assert_eq!(warm.project_diagnostic_publication.as_ref().unwrap().sites, before_restore.sites);
        assert!(!drain(&mut warm).iter().any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_))));
        warm.reload_project(106, &ReloadProject { base_revision: 1, target_revision: 2, changes: vec![FileChange::Upsert { file: reload_source }] }).unwrap();
        let restored_reload = take_static_call_report(&drain(&mut warm));
        assert_published_call_warning(&warm, &restored_reload, Some(warm.vm.as_ref().unwrap().current_generation().0));
    }
}

#[test]
fn static_call_publication_retires_cold_load_scopes_even_when_returning_to_same_artifact() {
    let first = static_call_diagnostic_manifest();
    let mut second = first.clone();
    let FilePayload::Utf8(source) = &mut second.files[0].payload else { unreachable!() };
    source.push_str("; distinct committed source\n");
    let mut session = negotiated_session();
    let mut artifact = None;
    for (index, manifest) in [first.clone(), second, first].into_iter().enumerate() {
        let report = load_static_call_project(&mut session, 110 + index as u64, manifest);
        assert_published_call_warning(&session, &report, None);
        assert_eq!(session.project_load_id, index as u64 + 1);
        let scope = &session.project_diagnostic_publication.as_ref().unwrap().scope;
        if index == 0 { artifact = Some(scope.artifact); }
        if index == 2 { assert_eq!(Some(scope.artifact), artifact); }
        assert_eq!(session.project_diagnostic_publication.as_ref().unwrap().sites.len(), 1);
    }
}

#[test]
fn static_call_publication_reloads_actual_generation_and_ignores_failed_candidates() {
    let mut manifest = static_call_diagnostic_manifest();
    let FilePayload::Utf8(source) = &mut manifest.files[0].payload else { unreachable!() };
    *source = source.replace("CALL TARGET, 1, 2\nRETURN", "CALL TARGET, 1, 2\nWAIT\nRETURN");
    let original = manifest.files[0].clone();
    let mut session = negotiated_session();
    let initial = load_static_call_project(&mut session, 120, manifest);
    assert_published_call_warning(&session, &initial, None);
    session.start(121, &StartRequest { mode: StartMode::NewGame { seed: Some(7) } }).unwrap();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput { break; }
    }
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    drain(&mut session);
    let old_generation = session.vm.as_ref().unwrap().current_generation().0;
    let old_artifact = session.vm.as_ref().unwrap().artifact_id();
    session.reload_project(122, &ReloadProject { base_revision: 1, target_revision: 2, changes: Vec::new() }).unwrap();
    let reloaded = take_static_call_report(&drain(&mut session));
    let generation = session.vm.as_ref().unwrap().current_generation().0;
    assert!(generation > old_generation);
    assert_eq!(session.vm.as_ref().unwrap().artifact_id(), old_artifact);
    assert_published_call_warning(&session, &reloaded, Some(generation));
    assert_eq!(session.project_load_id, 1);
    session.emit_committed_project_report(123, reloaded.clone(), Some(generation)).unwrap();
    assert!(static_call_warnings(&take_static_call_report(&drain(&mut session))).is_empty());
    let before = session.project_diagnostic_publication.clone().unwrap();
    let mut broken = original.clone();
    broken.payload = FilePayload::Utf8("@SYSTEM_TITLE\nIF\n".into());
    session.reload_project(124, &ReloadProject { base_revision: 2, target_revision: 3, changes: vec![FileChange::Upsert { file: broken }] }).unwrap();
    let failed = take_static_call_report(&drain(&mut session));
    assert!(!failed.success);
    let after = session.project_diagnostic_publication.as_ref().unwrap();
    assert_eq!(after.scope, before.scope);
    assert_eq!(after.sites, before.sites);
    assert_eq!(session.vm.as_ref().unwrap().current_generation().0, generation);
    session.reload_project(125, &ReloadProject { base_revision: 2, target_revision: 3, changes: vec![FileChange::Upsert { file: original }] }).unwrap();
    let retry = take_static_call_report(&drain(&mut session));
    let next_generation = session.vm.as_ref().unwrap().current_generation().0;
    assert!(next_generation > generation);
    assert_published_call_warning(&session, &retry, Some(next_generation));
    assert_eq!(session.project_diagnostic_publication.as_ref().unwrap().sites.len(), 1);
    // Existing calls retain their old generation after reload. Normal snapshots
    // must reject that state; restoration is covered at the warm stable wait above.
    session.export_state(126, StateExportRequest { kind: StateExportKind::VmSnapshot, snapshot_purpose: SnapshotExportPurpose::Normal }).unwrap();
    assert!(session.outbound_transfer.is_none());
    let export_messages = drain(&mut session);
    assert!(export_messages.iter().any(|message| matches!(message,
        RuntimeMessage::StateExportReady(StateExportReady {
            kind: StateExportKind::VmSnapshot,
            result: StateExportResult::Ineligible { reasons },
        }) if reasons == &[SnapshotIneligibleReason::SnapshotStateUnavailable]
    )), "{export_messages:?}");
    session.reload_project(128, &ReloadProject { base_revision: 3, target_revision: 4, changes: Vec::new() }).unwrap();
    let restored_reload = take_static_call_report(&drain(&mut session));
    assert_published_call_warning(&session, &restored_reload, Some(session.vm.as_ref().unwrap().current_generation().0));
}

#[test]
fn static_call_publication_journal_failure_does_not_consume_a_source_site() {
    let mut session = negotiated_session();
    let mut report = load_static_call_project(&mut session, 130, static_call_diagnostic_manifest());
    let before = session.project_diagnostic_publication.clone().unwrap();
    let diagnostic = report.diagnostics.iter_mut().find(|diagnostic| diagnostic.code == "compat.call.excess_arguments").unwrap();
    diagnostic.source.as_mut().unwrap().byte_end += 1;
    let maximum = session.options.limits.maximum_journal_entries;
    session.options.limits.maximum_journal_entries = 0;
    assert!(matches!(session.emit_committed_project_report(131, report.clone(), None), Err(RuntimeError::ResourceLimit(_))));
    session.options.limits.maximum_journal_entries = maximum;
    assert_eq!(session.project_diagnostic_publication.as_ref().unwrap().sites, before.sites);
    assert_eq!(session.project_diagnostic_publication.as_ref().unwrap().scope, before.scope);
    session.emit_committed_project_report(132, report, None).unwrap();
    assert_eq!(static_call_warnings(&take_static_call_report(&drain(&mut session))).len(), 1);
}

#[test]
fn new_vm_owner_can_publish_the_same_artifact_and_generation_number_again() {
    let mut manifest = static_call_diagnostic_manifest();
    let FilePayload::Utf8(source) = &mut manifest.files[0].payload else { unreachable!() };
    *source = source.replace("CALL TARGET, 1, 2\nRETURN", "CALL TARGET, 1, 2\nWAIT\nRETURN");
    let mut session = negotiated_session();
    load_static_call_project(&mut session, 140, manifest);
    session.start(141, &StartRequest { mode: StartMode::NewGame { seed: Some(7) } }).unwrap();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput { break; }
    }
    drain(&mut session);
    session.reload_project(142, &ReloadProject { base_revision: 1, target_revision: 2, changes: Vec::new() }).unwrap();
    let first = take_static_call_report(&drain(&mut session));
    assert_published_call_warning(&session, &first, Some(2));
    let old = session.project_diagnostic_publication.as_ref().unwrap().scope.clone();
    session.return_to_title(143).unwrap();
    let seed_request = drain(&mut session).into_iter().find_map(|message| match message {
        RuntimeMessage::ServiceRequest(request) if request.operation == RANDOM_SEED_OPERATION => Some(request.request_id), _ => None,
    }).expect("new title VM requests an actual new seed");
    submit(&mut session, 1, RuntimeMessage::ServiceResponse(ServiceResponse {
        request_id: seed_request,
        result: ServiceResult::Ready { payload: ProtocolBytes::new(encode_canonical(&RandomSeedResponse { seed: 11 }).unwrap()) },
    }));
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput { break; }
    }
    assert_eq!(session.vm.as_ref().unwrap().current_generation().0, 1);
    drain(&mut session);
    session.reload_project(144, &ReloadProject { base_revision: 2, target_revision: 3, changes: Vec::new() }).unwrap();
    let second = take_static_call_report(&drain(&mut session));
    assert_published_call_warning(&session, &second, Some(2));
    let current = &session.project_diagnostic_publication.as_ref().unwrap().scope;
    assert_eq!(current.artifact, old.artifact);
    assert_eq!(current.generation, old.generation);
    assert!(current.runtime_epoch > old.runtime_epoch);
    assert_eq!(current.project_load_id, old.project_load_id);
    assert_eq!(session.project_diagnostic_publication.as_ref().unwrap().sites.len(), 1);
}
