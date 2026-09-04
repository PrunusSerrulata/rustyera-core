#[test]
#[allow(clippy::too_many_lines)]
fn compiled_cache_export_prepares_the_payload_off_thread() {
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
                relative_path: "emuera.config".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8("Font size:18\n".into()),
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
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();

    let generated = session
        .project_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.generated_configuration_source.as_deref())
        .expect("legacy configuration generates reraconfig.toml");
    assert_rera_font_size(generated, 18);
    let snapshot_manifest = &session.project_snapshot.as_ref().unwrap().manifest;
    assert!(matches!(
        &snapshot_manifest.files[0].payload,
        FilePayload::Utf8(value) if value.is_empty()
    ));
    assert!(matches!(
        &snapshot_manifest.files[2].payload,
        FilePayload::Utf8(value) if value == "; no sprites\n"
    ));

    assert!(session.compiled_project_cache.is_none());
    assert!(session.compiled_cache_task.is_none());
    let _ = drain(&mut session);
    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "compiled project cache preparation started"
    )));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.compiled_cache_task.is_some() {
        session.poll_compiled_cache_task().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "compiled cache worker did not finish"
        );
        std::thread::yield_now();
    }
    let completion = drain(&mut session);
    assert!(
        completion.iter().any(|message| matches!(
            message,
            RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
                if code == "runtime.compiled_cache_ready"
        )),
        "{completion:#?}"
    );
    let bytes = session.compiled_project_cache.as_ref().unwrap();
    let decoded = crate::compiled_cache::decode(bytes, 64 * 1024 * 1024).unwrap();
    assert_manifest_rera_font_size(&decoded.snapshot.manifest, 18);
    assert!(matches!(
        &decoded.snapshot.manifest.files[2].payload,
        FilePayload::Utf8(value) if value == "; no sprites\n"
    ));
    assert!(crate::compiled_cache::decode_project_file(bytes, bytes.len()).is_err());

    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            kind: StateExportKind::CompiledProjectCache,
            result: StateExportResult::Ready { .. },
        })
    )));
}

#[test]
fn full_project_export_preempts_cache_streams_chunks_and_cancels_cleanly() {
    let (mut session, manifest, _) = low_memory_cooperative_cache_session();
    assert!(matches!(
        &session.project_snapshot.as_ref().unwrap().manifest.files[0].payload,
        FilePayload::Utf8(value) if value.is_empty() && value.capacity() == 0
    ));
    let cache_manifest = Arc::clone(&session.project_snapshot.as_ref().unwrap().manifest);
    session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(cooperative_cache_encoder(&session, cache_manifest)),
    });
    let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&progress);
    session.project_progress_reporter = Some(ProjectProgressReporter::new(move |value| {
        observed.lock().unwrap().push(value);
    }));

    session
        .stage_full_project_manifest(
            100,
            FullProjectManifest {
                manifest: manifest.clone(),
            },
        )
        .unwrap();
    assert!(matches!(
        &session
            .staged_full_project_manifest
            .as_ref()
            .unwrap()
            .manifest
            .files[0]
            .payload,
        FilePayload::Utf8(value) if value.contains("@SYSTEM_TITLE")
    ));
    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_none());
    assert!(session.full_project_task.is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "full project preparation started"
    )));

    finish_full_project_build(&mut session);
    let reports = progress.lock().unwrap();
    assert!(reports.iter().any(|value| {
        value.stage == ProjectProgressStage::Packaging && value.total > 1 && value.completed > 0
    }));
    drop(reports);

    session
        .export_state(
            102,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    let ready = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("full project transfer is ready");
    session
        .read_state_export(
            103,
            StateExportChunkRequest {
                transfer_id: ready.transfer_id,
                offset: 0,
                maximum_bytes: 17,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportChunk(StateExportChunk { offset: 0, .. })
    )));
    session.cancel_state_export(StateExportCancel {
        kind: StateExportKind::FullProjectFile,
    });
    assert!(session.outbound_transfer.is_none());
    assert!(session.full_project_task.is_none());
    assert!(session.staged_full_project_manifest.is_none());
}

fn finish_full_project_build(session: &mut RuntimeSession) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.full_project_task.is_some() {
        session.poll_full_project_task();
        assert!(
            std::time::Instant::now() < deadline,
            "full project worker did not finish"
        );
        std::thread::yield_now();
    }
}

fn commit_restart_pending_configuration(session: &mut RuntimeSession) -> (ProjectIdentity, String) {
    let current = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .configuration_snapshot();
    session
        .prepare_configuration_update(
            100,
            &PrepareConfigurationUpdate {
                project_revision: current.project_revision,
                expected_source_digest: current.source_digest,
                changes: vec![ConfigurationChange {
                    code: "AutoSave".into(),
                    value: "NO".into(),
                }],
            },
        )
        .unwrap();
    assert!(drain(session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ConfigurationUpdatePrepared(value) if value.restart_required
    )));
    session
        .finalize_configuration_update(
            101,
            FinalizeConfigurationUpdate {
                preparation_message_id: 100,
                outcome: ConfigurationUpdateOutcome::Commit,
            },
        )
        .unwrap();
    let committed = drain(session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ConfigurationUpdateCommitted(value) => Some(value.configuration),
            _ => None,
        })
        .expect("restart configuration is committed");
    assert!(committed.restart_pending);
    let manifest = session.project_snapshot.as_ref().unwrap().manifest.as_ref();
    let identity = crate::compiled_cache::project_identity(manifest);
    let source = manifest
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case("reraconfig.toml"))
        .and_then(|file| match &file.payload {
            FilePayload::Utf8(source) => Some(source.clone()),
            _ => None,
        })
        .expect("committed configuration is present in the pending manifest");
    (identity, source)
}

fn assert_journaled_configuration_reopens(
    session: &RuntimeSession,
    pending_identity: &ProjectIdentity,
    pending_source: &str,
) {
    let bytes = session.full_project_file.as_ref().unwrap();
    let bytes = bytes.copy_range(0..bytes.len());
    let decoded = crate::compiled_cache::decode_project_file(&bytes, bytes.len()).unwrap();
    assert_eq!(&decoded.identity, pending_identity);
    let decoded_source = decoded
        .manifest
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case("reraconfig.toml"))
        .and_then(|file| match &file.payload {
            FilePayload::Utf8(source) => Some(source),
            _ => None,
        })
        .expect("exported project retains the pending configuration source");
    assert_eq!(decoded_source, pending_source);
    let rebuilt = RuntimeSession::new(RuntimeOptions::default())
        .build_project_from_cache(
            ProjectLoadRequest {
                identity: decoded.identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&bytes),
            None,
        )
        .expect("journaled configuration rebuilds from the embedded project sources");
    assert!(rebuilt.report.success, "{:?}", rebuilt.report.diagnostics);
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "runtime.compiled_cache_hit")
    );
    assert!(
        !rebuilt
            .snapshot
            .unwrap()
            .configuration_snapshot()
            .restart_pending
    );
}

#[test]
fn full_project_export_journals_configuration_that_is_pending_restart() {
    let (mut session, _, _) = cooperative_cache_session();
    session.configuration_profile = ConfigurationClientProfile::Tui;
    session
        .project_snapshot
        .as_mut()
        .unwrap()
        .configuration_profile = ConfigurationClientProfile::Tui;
    let (pending_identity, pending_source) = commit_restart_pending_configuration(&mut session);
    let manifest = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .manifest
        .as_ref()
        .clone();
    session
        .stage_full_project_manifest(102, FullProjectManifest { manifest })
        .unwrap();
    session
        .export_state(
            103,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "full project preparation started"
    )));
    finish_full_project_build(&mut session);
    assert!(session.full_project_task.is_none());
    assert!(session.full_project_failure.is_none());
    assert_journaled_configuration_reopens(&session, &pending_identity, &pending_source);
}

#[test]
fn full_project_export_rejects_a_stale_materialized_manifest() {
    let (mut session, mut manifest, _) = cooperative_cache_session();
    manifest.files[0].payload = FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n".into());
    session
        .stage_full_project_manifest(100, FullProjectManifest { manifest })
        .unwrap();
    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();

    assert!(session.full_project_task.is_none());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message.contains("changed after the active project")
    )));
}

fn assert_manifest_rera_font_size(manifest: &ProjectManifest, expected: i64) {
    let source = manifest
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case("reraconfig.toml"))
        .and_then(|file| match &file.payload {
            FilePayload::Utf8(source) => Some(source.as_str()),
            _ => None,
        })
        .expect("compiled project embeds the generated reraconfig.toml");
    assert_rera_font_size(source, expected);
}

fn assert_rera_font_size(source: &str, expected: i64) {
    let values = era_config::ReraConfigDocument::parse(source)
        .unwrap()
        .values()
        .unwrap();
    assert_eq!(
        values.get_code("FontSize"),
        Some(&era_config::ConfigValue::Integer(expected))
    );
}
