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
                payload: FilePayload::Utf8("[meta]\nschema_version = 5\n[compatibility]\nprofile = \"emuera.skia.snake\"\n".into()),
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
        warm.start_vm_snapshot(105, &snapshot.copy_range(0..snapshot.len())).unwrap();
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
