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
