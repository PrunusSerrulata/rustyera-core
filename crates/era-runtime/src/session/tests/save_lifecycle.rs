use super::*;

#[test]
#[allow(clippy::too_many_lines)]
fn traditional_save_export_and_restore_are_atomic_runtime_operations() {
    fn prepare() -> RuntimeSession {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "save-test".into(),
                features: vec![RuntimeFeature::TraditionalSave, RuntimeFeature::VmSnapshot],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en-US".into()],
                configuration_profile: None,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
                &mut session,
                1,
                RuntimeMessage::ProjectManifest(ProjectManifest {
                    project_revision: 1,
                    files: vec![
                        SubmittedFile {
                            relative_path: "variables.erh".into(),
                            category: FileCategory::Erh,
                            payload: FilePayload::Utf8("#DIM SAVEDATA ZZZSAVE\n".into()),
                            content_hash: None,
                        },
                        SubmittedFile {
                            relative_path: "save.erb".into(),
                            category: FileCategory::Erb,
                            payload: FilePayload::Utf8(
                                "@SYSTEM_TITLE\nINPUT\nZZZSAVE = RESULT\nINPUT\nRETURN\n@SYSTEM_LOADEND\nPRINTFORML loadend={ZZZSAVE}\nRETURN\n@EVENTLOAD\nPRINTL eventload\nRETURN\n@SHOW_SHOP\nPRINTL shop\nWAIT\nRETURN\n@SAVEINFO\nPRINTL unexpected-autosave\nRETURN\n"
                                    .into(),
                            ),
                            content_hash: None,
                        },
                        SubmittedFile {
                            relative_path: "resources/opaque.bin".into(),
                            category: FileCategory::Resource,
                            payload: FilePayload::Bytes(ProtocolBytes::new(vec![7; 4096])),
                            content_hash: None,
                        },
                    ],
                }),
            );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let load_messages = drain(&mut session);
        assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
        session
    }

    let mut source = prepare();
    submit(
        &mut source,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        source.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let wait = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait),
            _ => None,
        })
        .expect("first INPUT wait");
    submit(
        &mut source,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::CommitText("37".into()),
            message_skip: false,
        }),
    );
    for _ in 0..4 {
        source.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut source);
    assert_eq!(source.phase(), RuntimePhase::WaitingInput);
    submit(
        &mut source,
        4,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::TraditionalSave,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let descriptor = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("traditional save descriptor");
    submit(
        &mut source,
        5,
        RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
            transfer_id: descriptor.transfer_id,
            offset: 0,
            maximum_bytes: u32::MAX,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let bytes = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportChunk(chunk) => Some(chunk.data.as_slice().to_vec()),
            _ => None,
        })
        .expect("traditional save bytes");

    assert_eq!(source.traditional_save_slot_count(), Some(20));
    assert_eq!(
        source.inspect_traditional_save(&bytes).unwrap(),
        TraditionalSaveInspection {
            description: String::new(),
        }
    );
    assert!(matches!(
        source.inspect_traditional_save(b"not a save"),
        Err(TraditionalSaveValidationError::Invalid(_))
    ));
    assert_eq!(source.phase(), RuntimePhase::WaitingInput);

    let mut restored = prepare();
    submit(
        &mut restored,
        2,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: u64::try_from(bytes.len()).unwrap(),
            digest: descriptor.digest,
            artifact_id: None,
        }),
    );
    restored.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut restored)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .expect("accepted import");
    submit(
        &mut restored,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(bytes),
        }),
    );
    submit(
        &mut restored,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    restored.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut restored);
    submit(
        &mut restored,
        5,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::TraditionalSave { transfer_id },
        }),
    );
    for _ in 0..5 {
        restored.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut restored);
    let snapshot = restored.presentation.snapshot();
    let display = projected_presentation_text(&snapshot);
    let loadend = display.find("loadend=37").expect("SYSTEM_LOADEND output");
    let eventload = display.find("eventload").expect("EVENTLOAD output");
    let shop = display.find("shop").expect("SHOW_SHOP output");
    assert!(loadend < eventload && eventload < shop, "{display}");
    assert!(!display.contains("unexpected-autosave"), "{display}");
    assert_eq!(restored.system_menu, SystemMenuState::Title);

    // Snapshots written before the load-menu state was reset can contain a
    // gameplay wait together with a stale LoadSlots marker.
    source.system_menu = SystemMenuState::LoadSlots;
    source.controller.flow = Some(SystemFlow::Shop);
    source.phase = RuntimePhase::Faulted;
    let old_wait = source
        .operations
        .active_input()
        .expect("snapshot wait")
        .wait
        .clone();
    submit(
        &mut source,
        6,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::VmSnapshot,
            snapshot_purpose: SnapshotExportPurpose::Diagnosis,
        }),
    );
    source.drive(RuntimeDriveBudget::default()).unwrap();
    let snapshot_descriptor = drain(&mut source)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("runtime snapshot descriptor");
    let mut snapshot_bytes = Vec::new();
    let mut source_sequence = 7;
    loop {
        submit(
            &mut source,
            source_sequence,
            RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
                transfer_id: snapshot_descriptor.transfer_id,
                offset: u64::try_from(snapshot_bytes.len()).unwrap(),
                maximum_bytes: 1024 * 1024,
            }),
        );
        source_sequence += 1;
        source.drive(RuntimeDriveBudget::default()).unwrap();
        let chunk = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateExportChunk(chunk) => Some(chunk),
                _ => None,
            })
            .expect("runtime snapshot chunk");
        snapshot_bytes.extend_from_slice(chunk.data.as_slice());
        if chunk.complete {
            break;
        }
    }
    let marked = runtime_snapshot::decode(
        &snapshot_bytes,
        usize::try_from(source.options.limits.maximum_transfer_bytes).unwrap(),
    )
    .unwrap();
    assert_eq!(marked.origin, RuntimeSnapshotOrigin::Diagnosis);
    assert_eq!(marked.resource_graph.embedded_project_bytes(), 0);
    let inspection = crate::inspect_runtime_snapshot(
        &snapshot_bytes,
        usize::try_from(source.options.limits.maximum_transfer_bytes).unwrap(),
    )
    .unwrap();
    assert_eq!(inspection.container.magic, "RERARTS\\0");
    assert_eq!(inspection.payload["origin"], "Diagnosis");
    assert_eq!(inspection.payload["system_menu_name"], "load_slots");
    assert_eq!(
        inspection.payload["execution_state"]["container"]["magic"],
        "RERAVMS\\0"
    );
    assert_eq!(inspection.validation.runtime_container, "valid");
    assert_eq!(inspection.validation.artifact_compatibility, "not_checked");

    let mut exact = prepare();
    submit(
        &mut exact,
        2,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::VmSnapshot,
            total_bytes: u64::try_from(snapshot_bytes.len()).unwrap(),
            digest: snapshot_descriptor.digest,
            artifact_id: snapshot_descriptor.artifact_id,
        }),
    );
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut exact)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();
    let mut exact_sequence = 3;
    for (index, chunk) in snapshot_bytes.chunks(1024 * 1024).enumerate() {
        submit(
            &mut exact,
            exact_sequence,
            RuntimeMessage::StateImportChunk(StateImportChunk {
                transfer_id,
                offset: u64::try_from(index * 1024 * 1024).unwrap(),
                data: ProtocolBytes::new(chunk.to_vec()),
            }),
        );
        exact_sequence += 1;
    }
    submit(
        &mut exact,
        exact_sequence,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    exact_sequence += 1;
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut exact);
    submit(
        &mut exact,
        exact_sequence,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::VmSnapshot { transfer_id },
        }),
    );
    exact.drive(RuntimeDriveBudget::default()).unwrap();
    let restore_messages = drain(&mut exact);
    assert!(restore_messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
            if code == "runtime.snapshot_restored_from_diagnosis"
    )));
    let restored_wait = exact.operations.active_input().expect("restored wait");
    assert_eq!(exact.phase(), RuntimePhase::WaitingInput);
    assert_eq!(exact.system_menu, SystemMenuState::Title);
    assert_ne!(restored_wait.wait.wait_id, old_wait.wait_id);
    assert_ne!(
        restored_wait.wait.submission_token,
        old_wait.submission_token
    );
    assert_eq!(restored_wait.wait.submission_token.epoch, exact.epoch.0);
}

#[test]
fn empty_storage_listing_opens_a_fixed_runtime_tokenized_page() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingExternal;
    session.epoch = SessionEpoch(1);
    session.selected_locale = "en".into();
    session.storage_capabilities = StorageCapabilities {
        revisions: true,
        atomic_replace: true,
        missing_precondition: true,
        delete: true,
    };
    session
        .operations
        .insert_storage(7, PendingStorage::ListLoadSlots);
    session
        .complete_storage(
            10,
            StorageResponse {
                request_id: 7,
                result: StorageResult::Listed {
                    entries: Vec::new(),
                },
            },
        )
        .unwrap();
    assert_eq!(
        session.load_slot_paths.first().map(String::as_str),
        Some("save00.sav")
    );
    assert_eq!(
        session.load_slot_paths.last().map(String::as_str),
        Some("save99.sav")
    );
    assert_eq!(session.load_slot_paths.len(), 21);
    assert!(session.occupied_slot_paths.is_empty());
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let wait = session.operations.active_input().expect("system slot wait");
    assert!(wait.wait.system_input);
    assert!(
        wait.choices
            .keys()
            .all(|token| token.epoch == session.epoch.0)
    );
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .iter()
            .any(|line| {
                line.runs.iter().any(|run| {
                    matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text {
                            system_text: Some(reference),
                            ..
                        }
                        | era_runtime_protocol::DisplayRun::TextLayout {
                            system_text: Some(reference),
                            ..
                        } if reference.key == SystemTextKey::LoadQuestion
                    )
                })
            })
    );
}

#[test]
fn occupied_save_slots_do_not_expose_delete_actions() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingExternal;
    session.epoch = SessionEpoch(1);
    session.selected_locale = "en".into();
    session.storage_capabilities = StorageCapabilities {
        revisions: true,
        atomic_replace: true,
        missing_precondition: true,
        delete: true,
    };
    session.occupied_slot_paths.insert("save00.sav".into());
    session
        .slot_labels
        .insert("save00.sav".into(), "occupied".into());

    session.render_slot_menu(false).unwrap();

    let wait = session.operations.active_input().expect("load slot wait");
    assert!(
        wait.choices
            .values()
            .all(|value| { matches!(value, VmValue::Integer(selection) if *selection >= 0) })
    );
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().all(|line| {
        line.runs.iter().all(|run| match run {
            DisplayRun::Button { runs, .. } => runs.iter().all(|run| {
                !matches!(
                    run,
                    DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. }
                        if text.starts_with("Delete ")
                )
            }),
            _ => true,
        })
    }));
}

#[test]
#[allow(clippy::too_many_lines)]
fn nested_savegame_cancel_resumes_the_suspended_vm_call() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "save-menu-test".into(),
            features: vec![RuntimeFeature::Storage, RuntimeFeature::VmSnapshot],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "menu.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SHOW_SHOP\nSAVEGAME\nRESULT = 7\nWAIT\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let artifact = session.artifact.clone().expect("compiled menu fixture");
    let entry = artifact
        .artifact()
        .functions
        .iter()
        .find(|function| function.name == "SHOW_SHOP")
        .expect("SHOW_SHOP")
        .key;
    let code = artifact
        .artifact()
        .functions
        .iter()
        .find(|function| function.key == entry)
        .unwrap()
        .code
        .clone();
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    vm.spawn_entry(entry, Vec::new()).unwrap();
    session.vm = Some(vm);
    session.controller.flow = Some(SystemFlow::Normal);
    session.phase = RuntimePhase::Running;

    let mut request = None;
    let mut observed = Vec::new();
    let mut reports = Vec::new();
    for _ in 0..4 {
        reports.push(session.drive(RuntimeDriveBudget::default()).unwrap());
        let messages = drain(&mut session);
        request = messages.iter().find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request.clone()),
            _ => None,
        });
        observed.extend(messages);
        if request.is_some() {
            break;
        }
    }
    let request = request.unwrap_or_else(|| {
            panic!(
                "SAVEGAME list request; phase={:?}, code={code:#?}, reports={reports:#?}, output={observed:#?}",
                session.phase,
            )
        });
    assert!(matches!(request.operation, StorageOperation::List { .. }));
    session
        .complete_storage(
            2,
            StorageResponse {
                request_id: request.request_id,
                result: StorageResult::Listed {
                    entries: vec![
                        StorageEntry {
                            relative_path: "save01.sav".into(),
                            byte_length: 3,
                            revision: None,
                            change_token: Some("t1".into()),
                        },
                        StorageEntry {
                            relative_path: "save25.sav".into(),
                            byte_length: 3,
                            revision: None,
                            change_token: Some("t25".into()),
                        },
                    ],
                },
            },
        )
        .unwrap();
    let scan = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .expect("slot metadata read");
    assert_eq!(scan.relative_path, "save01.sav");
    assert!(matches!(
        scan.operation,
        StorageOperation::ReadRange {
            offset: 0,
            maximum_bytes: 65_536,
            ..
        }
    ));
    session
        .complete_storage(
            3,
            StorageResponse {
                request_id: scan.request_id,
                result: StorageResult::ReadChunk {
                    data: ProtocolBytes::new(b"bad".to_vec()),
                    offset: 0,
                    complete: true,
                    change_token: "t1".into(),
                },
            },
        )
        .unwrap();
    assert!(session.invalid_slot_paths.contains("save01.sav"));
    assert!(session.occupied_slot_paths.contains("save25.sav"));
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::StorageRequest(_)))
    );
    let pending = session
        .operations
        .take_active_input()
        .expect("save menu wait");
    assert!(pending.host_request.is_some());
    session.operations.restore_active_input(pending.clone());
    assert!(session.operations.is_snapshot_stable());
    assert!(session.vm.as_ref().unwrap().snapshot().is_ok());
    session
        .export_state(
            99,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(drain(&mut session).into_iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            result: StateExportResult::Ready { .. },
            ..
        })
    )));
    let pending = session.operations.take_active_input().unwrap();
    assert!(
        pending
            .choices
            .values()
            .all(|value| matches!(value, VmValue::Integer(selection) if *selection >= 0))
    );
    session
        .finish_system_input(pending, &VmValue::Integer(100))
        .unwrap();
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        7
    );
}

#[test]
fn project_title_can_open_loadgame() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "title-loadgame-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "title.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nLOADGAME\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut messages = Vec::new();
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::StorageRequest(_)))
        {
            break;
        }
    }
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StorageRequest(StorageRequest {
                operation: StorageOperation::List { .. },
                ..
            })
        )),
        "{messages:#?}"
    );
    assert_ne!(session.phase(), RuntimePhase::Faulted);
}

#[test]
fn vm_snapshot_export_accepts_a_runtime_owned_system_wait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "successive-root-snapshot-test".into(),
            features: vec![RuntimeFeature::VmSnapshot],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "snapshot.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nBEGIN SHOP\n@EVENTSHOP\nRETURN\n@SHOW_SHOP\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..12 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert!(
        session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.system_input && input.host_request.is_none())
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::VmSnapshot,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { .. },
                ..
            })
        )),
        "{messages:#?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn savedata_uses_atomic_frontend_storage_and_resumes_only_after_completion() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "storage-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "save.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPUTFORM suffix\nRESULT = SAVENOS()\nSAVEDATA 2, \"slot\"\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut request = None;
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::StorageRequest(value) = message {
                request = Some(value);
            }
        }
        if request.is_some() {
            break;
        }
    }
    let request = request.expect("SAVEDATA storage request");
    assert_eq!(request.namespace, StorageNamespace::Save);
    assert_eq!(request.relative_path, "save02.sav");
    let StorageOperation::Write {
        data,
        atomic_replace,
        precondition,
    } = request.operation
    else {
        panic!("SAVEDATA must write")
    };
    assert!(atomic_replace);
    assert_eq!(precondition, StoragePrecondition::Any);
    let decoded = era_runtime_save::decode(
        data.as_slice(),
        era_runtime_save::SaveCodecLimits::default(),
    )
    .expect("current save bytes");
    assert_eq!(decoded.metadata.description, "slot");
    assert_eq!(session.phase(), RuntimePhase::WaitingExternal);

    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: request.request_id,
            result: StorageResult::Written {
                revision: Some("r1".into()),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("WAIT after save")
            .wait
            .kind,
        WaitKind::EnterKey
    );
    let vm = session.vm.as_ref().expect("runtime VM");
    assert_eq!(read_runtime_string(vm, "SAVEDATA_TEXT").unwrap(), "suffix");
    assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 20);
}

#[test]
fn binary_save_adapter_encodes_zero_length_saved_arrays() {
    use std::collections::BTreeMap;

    use erabasic_bytecode::{
        ArtifactManifest, BytecodeArtifact, BytecodeCallCompatibility, BytecodeGlobal,
        BytecodePersistence, BytecodeStorage, BytecodeType, Digest, SourceMap, SymbolKey,
    };
    use erabasic_csv::{CsvLoadOptions, ProjectFiles, load_project};
    use erabasic_vm::EraVariableState;

    let key = SymbolKey::derive("zero-length-save", b"empty");
    let project_data = load_project(&ProjectFiles::default(), &CsvLoadOptions::default())
        .data
        .unwrap();
    let artifact = BytecodeArtifact {
        manifest: ArtifactManifest::new(Digest::default()),
        call_compatibility: BytecodeCallCompatibility::default(),
        project_data,
        globals: vec![BytecodeGlobal {
            key,
            name: "EMPTY".into(),
            value_type: BytecodeType::Integer,
            dimensions: vec![0],
            mutable: true,
            storage: BytecodeStorage::Project,
            persistence: BytecodePersistence::GameSave,
            initial_values: Vec::new(),
            owner: None,
        }],
        native_imports: Vec::new(),
        host_imports: Vec::new(),
        functions: Vec::new(),
        event_groups: Vec::new(),
        source_map: SourceMap::default(),
    };
    let state = EraState {
        unique_code: 1,
        version: 2,
        variables: BTreeMap::from([(
            key,
            EraVariableState {
                name: "EMPTY".into(),
                value_type: BytecodeType::Integer,
                dimensions: vec![0],
                persistence: BytecodePersistence::GameSave,
                storage: BytecodeStorage::Project,
                values: Vec::new(),
                sparse_values: None,
            },
        )]),
        characters: Vec::new(),
    };

    let encoded = encode_scoped_save(
        &state,
        &artifact,
        era_runtime_save::SaveFileKind::Normal,
        "zero".into(),
        Vec::new(),
        era_runtime_save::SaveFormat::Binary1808,
    )
    .unwrap();
    let decoded =
        era_runtime_save::decode_sparse(&encoded, era_runtime_save::SaveCodecLimits::default())
            .unwrap();
    assert!(matches!(
        &decoded.variables[0],
        era_runtime_save::SaveEntry {
            name,
            value: era_runtime_save::SaveValue::SparseIntegers { dimensions, values },
        } if name == "EMPTY" && dimensions == &[0] && values.is_empty()
    ));
}

#[test]
fn runtime_drive_reinstalls_the_vm_before_propagating_host_event_errors() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "host-error-vm-ownership-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "invalid-save.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nSAVEDATA -1, \"invalid\"\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let load_messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );

    let error = session.drive(RuntimeDriveBudget::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "SAVEDATA argument 1 must be between 0 and 2147483647"
    );
    assert!(session.vm.is_some(), "host error must not remove the VM");

    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_ne!(session.phase(), RuntimePhase::Faulted, "{messages:#?}");
    assert!(session.vm.is_some());
    assert!(messages.iter().all(|message| {
        !matches!(
            message,
            RuntimeMessage::Fault(RuntimeFault { message, .. })
                if message == "running phase has no VM"
        )
    }));
}
