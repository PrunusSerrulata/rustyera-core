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

