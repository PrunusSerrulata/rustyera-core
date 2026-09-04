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
                FilePayload::Utf8(value)
                    if file.category == FileCategory::ResourceManifest =>
                {
                    !value.is_empty()
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
