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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
fn snapshot_identity_mismatches_preserve_the_live_vm_and_wait() {
    let mut session =
        super::key_macro_input::start_input_project("@SYSTEM_TITLE\nRESULT = 37\nINPUT\nRETURN\n");
    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    let bytes = session
        .outbound_transfer
        .take()
        .expect("snapshot export")
        .bytes;
    drain(&mut session);
    let before_vm = session.vm.as_ref().unwrap().snapshot().unwrap();
    let before_wait = session.operations.active_input().unwrap().wait.clone();
    let before_epoch = session.epoch;
    let before_revision = session.revision;
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );

    for mismatch in [
        "outer_profile",
        "inner_profile",
        "outer_artifact",
        "inner_artifact",
    ] {
        let mut payload = runtime_snapshot::decode(&bytes, usize::MAX).unwrap();
        match mismatch {
            "outer_profile" => payload.compatibility = snake.clone(),
            "outer_artifact" => payload.artifact_id = erabasic_bytecode::Digest([7; 32]),
            "inner_profile" | "inner_artifact" => {
                let vm = erabasic_vm::VmSnapshot::decode(&payload.vm_snapshot, usize::MAX).unwrap();
                let mut document = serde_json::to_value(vm).unwrap();
                if mismatch == "inner_profile" {
                    document["compatibility"] = serde_json::to_value(&snake).unwrap();
                } else {
                    document["artifact_id"] =
                        serde_json::to_value(erabasic_bytecode::Digest([7; 32])).unwrap();
                }
                let modified: erabasic_vm::VmSnapshot = serde_json::from_value(document).unwrap();
                payload.vm_snapshot = modified.encode().unwrap();
            }
            _ => unreachable!(),
        }
        let encoded = runtime_snapshot::encode(&payload).unwrap();
        session.start_vm_snapshot(101, &encoded).unwrap();
        let messages = drain(&mut session);
        assert!(
            messages.iter().any(|message| matches!(
                message,
                RuntimeMessage::CommandRejected(CommandRejected {
                    code: CommandErrorCode::VersionMismatch | CommandErrorCode::InvalidValue,
                    ..
                })
            )),
            "{mismatch}: {messages:?}"
        );
        assert_eq!(session.phase(), RuntimePhase::WaitingInput, "{mismatch}");
        assert_eq!(session.epoch, before_epoch, "{mismatch}");
        assert_eq!(session.revision, before_revision, "{mismatch}");
        assert_eq!(
            session.operations.active_input().unwrap().wait,
            before_wait,
            "{mismatch}"
        );
        assert_eq!(
            session.vm.as_ref().unwrap().snapshot().unwrap(),
            before_vm,
            "{mismatch}"
        );
    }
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
                compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
        runtime_builtins: Vec::new(),
        runtime_native_authorizations: Vec::new(),
        runtime_host_authorizations: Vec::new(),
        runtime_staged_authorizations: Vec::new(),
        runtime_variables: vec![erabasic_bytecode::RuntimeVariableSymbol {
            match_name_rejection: Some(
                erabasic_bytecode::MatchNameRejectionKind::Internal,
            ),
            character_disposal: erabasic_bytecode::CharacterArrayDisposal::Preserve,
            key,
            reference: false,
            reference_semantics: erabasic_bytecode::RuntimeReferenceSemantics {
                is_const: false,
                can_restructure: false,
            },
        }],
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "invalid-save.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nLOADDATA 0\nRETURN\n".into(),
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

    // Complete Start before exhausting the journal, so the error arises while
    // dispatching the real storage Host event with the VM taken out of the session.
    session.drive(RuntimeDriveBudget {
        maximum_vm_instructions: 0,
        maximum_runtime_transitions: 1,
    }).unwrap();
    drain(&mut session);
    let journal_limit = session.options.limits.maximum_journal_entries;
    session.options.limits.maximum_journal_entries = 0;
    let error = session.drive(RuntimeDriveBudget::default()).unwrap_err();
    assert!(matches!(error, RuntimeError::ResourceLimit("outbound journal is full")));
    assert!(session.vm.is_some(), "host error must not remove the VM");

    session.options.limits.maximum_journal_entries = journal_limit;
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

fn global_storage_fixture(
    profile: erabasic_compat::CompatibilityProfileId,
    body: &str,
    binary_format: bool,
) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "global-storage-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let identity = erabasic_compat::CompatibilityIdentity::for_profile(profile);
    let files = [
        (
            "reraconfig.toml",
            FileCategory::Configuration,
            format!(
                "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"{}\"\n[save]\nbinary_format = {binary_format}\n",
                identity.profile.as_str()
            ),
        ),
        (
            "csv/VarExt.csv",
            FileCategory::Csv,
            "GLOBAL_MAPS,gmap\nGLOBAL_XMLS,1\nGLOBAL_DTS,column_global\n".into(),
        ),
        (
            "global.erb",
            FileCategory::Erb,
            format!("@SYSTEM_TITLE\n{body}\nINPUT\nRETURN\n"),
        ),
    ]
    .into_iter()
    .map(|(path, category, text)| SubmittedFile {
        relative_path: path.into(),
        category,
        payload: FilePayload::Utf8(text),
        content_hash: None,
    })
    .collect();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: identity,
            project_revision: 1,
            files,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready, "{messages:?}");
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame {
                seed: Some(123_456),
            },
        }),
    );
    session
}

fn next_global_storage_request(session: &mut RuntimeSession) -> StorageRequest {
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(session) {
            if let RuntimeMessage::StorageRequest(request) = message {
                assert_eq!(request.namespace, StorageNamespace::GlobalSave);
                assert_eq!(request.relative_path, "global.sav");
                return request;
            }
        }
    }
    panic!("missing GLOBAL storage request: {:?}", session.phase());
}

const GLOBAL_ROUNDTRIP_BODY: &str = r#"
RESULT = MAP_CREATE("gmap")
RESULT = MAP_SET("gmap", "key", "saved")
RESULT = XML_DOCUMENT(1, "<root>saved</root>")
RESULT = DT_CREATE("column_global")
RESULT = DT_COLUMN_ADD("column_global", "n", "int64", 1)
DT_COLUMN_OPTIONS "column_global", "n", DEFAULT, 12
RESULT = DT_ROW_ADD("column_global")
GLOBAL:0 = 7
FLAG:0 = 8
SAVEGLOBAL
GLOBAL:0 = 66
FLAG:0 = 55
RESULT = MAP_SET("gmap", "key", "changed")
RESULT = XML_REPLACE(1, "<root>changed</root>")
DT_COLUMN_OPTIONS "column_global", "n", DEFAULT, 99
RESULT = DT_ROW_ADD("column_global")
LOADGLOBAL
RESULT:10 = RESULT
RESULT:11 = GLOBAL:0
RESULT:12 = FLAG:0
RESULT:13 = DT_ROW_LENGTH("column_global")
RESULT = DT_ROW_ADD("column_global")
RESULT:14 = DT_CELL_GET("column_global", 1, "n")
RESULT:15 = DT_CELL_GET("column_global", 0, "n")
RESULTS:10 '= MAP_GET("gmap", "key")
RESULTS:11 '= XML_TOSTR(1)
"#;

fn global_fixture_at_load(
    profile: erabasic_compat::CompatibilityProfileId,
    binary_format: bool,
) -> (RuntimeSession, StorageRequest, Vec<u8>) {
    let mut session = global_storage_fixture(profile, GLOBAL_ROUNDTRIP_BODY, binary_format);
    let write = next_global_storage_request(&mut session);
    let StorageOperation::Write {
        data,
        atomic_replace,
        ..
    } = write.operation
    else {
        panic!("expected SAVEGLOBAL write")
    };
    assert!(atomic_replace);
    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: write.request_id,
            result: StorageResult::Written {
                revision: Some("saved".into()),
            },
        }),
    );
    let read = next_global_storage_request(&mut session);
    assert!(matches!(read.operation, StorageOperation::Read));
    let extensions = session
        .vm
        .as_ref()
        .unwrap()
        .structured_extensions(StructuredScope::Global)
        .unwrap();
    assert!(extensions.iter().any(|value| matches!(value, erabasic_vm::StructuredExtension::Xml { key, document } if key == "1" && document.contains("changed"))), "fixture must actually mutate XML before LOADGLOBAL");
    (session, read, data.as_slice().to_vec())
}

#[test]
#[allow(clippy::too_many_lines)]
fn ordinary_save_load_restores_randdata_but_keeps_the_active_native_stream() {
    fn next_request(session: &mut RuntimeSession) -> StorageRequest {
        for _ in 0..32 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            for message in drain(session) {
                if let RuntimeMessage::StorageRequest(request) = message {
                    assert_eq!(request.namespace, StorageNamespace::Save);
                    assert_eq!(request.relative_path, "save03.sav");
                    return request;
                }
            }
        }
        panic!("missing RNG save request: {:?}", session.phase());
    }
    let randdata = |vm: &RuntimeVm| {
        (0..625)
            .map(|index| read_runtime_integer(vm, "RANDDATA", &[index], None).unwrap())
            .collect::<Vec<_>>()
    };
    let mut reference_stream = None;
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let mut session = global_storage_fixture(
            profile,
            concat!(
                "FLAG:20 = RAND:1000000000\nDUMPRAND\nSAVEDATA 3, \"rng\"\n",
                "RANDOMIZE 4321\nFLAG:21 = RAND:1000000000\nDUMPRAND\nLOADDATA 3\nRETURN\n",
                "@SHOW_SHOP\nWAIT",
            ),
            true,
        );
        let write = next_request(&mut session);
        let StorageOperation::Write { data, .. } = write.operation else {
            panic!("SAVEDATA must write the ordinary save");
        };
        let saved_randdata = randdata(session.vm.as_ref().unwrap());
        assert_eq!(saved_randdata.len(), 625);
        assert_eq!(
            session.vm.as_ref().unwrap().export_random_state().unwrap(),
            saved_randdata
        );
        submit(
            &mut session,
            3,
            RuntimeMessage::StorageResponse(StorageResponse {
                request_id: write.request_id,
                result: StorageResult::Written {
                    revision: Some("rng-save".into()),
                },
            }),
        );
        let read = next_request(&mut session);
        assert!(matches!(read.operation, StorageOperation::Read));
        let vm = session.vm.as_mut().unwrap();
        let active = vm.export_random_state().unwrap();
        assert_ne!(
            active, saved_randdata,
            "fixture must change the actual random stream"
        );
        assert_eq!(randdata(vm), active);
        if let Some(reference) = &reference_stream {
            assert_eq!(
                &active, reference,
                "both profiles use the same legal SFMT state"
            );
        } else {
            reference_stream = Some(active.clone());
        }
        for invalid_index in [-1, 625, i64::MAX] {
            let mut invalid = saved_randdata.clone();
            invalid[624] = invalid_index;
            assert!(vm.restore_random_state(&invalid).is_err());
            assert_eq!(vm.export_random_state().unwrap(), active);
        }
        assert!(vm.restore_random_state(&saved_randdata[..624]).is_err());
        assert_eq!(vm.export_random_state().unwrap(), active);
        vm.restore_random_state(&active).unwrap();
        submit(
            &mut session,
            4,
            RuntimeMessage::StorageResponse(StorageResponse {
                request_id: read.request_id,
                result: StorageResult::Read {
                    data,
                    revision: Some("rng-save".into()),
                },
            }),
        );
        for _ in 0..32 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let vm = session.vm.as_ref().unwrap();
        // Ordinary saves store RANDDATA as a variable. Loading does not INITRAND:
        // the current native stream is retained; Ctrl-Z separately restores it.
        assert_eq!(randdata(vm), saved_randdata);
        assert_eq!(vm.export_random_state().unwrap(), active);
        assert_eq!(
            session.undo_checkpoint.as_ref().unwrap().random_state,
            active
        );
    }
}

#[test]
fn global_storage_respects_save_format_and_restores_only_global_variables() {
    for profile in [
        erabasic_compat::CompatibilityProfileId::EmueraEm,
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        for binary_format in [true, false] {
            let (mut session, read, bytes) = global_fixture_at_load(profile, binary_format);
            submit(
                &mut session,
                4,
                RuntimeMessage::StorageResponse(StorageResponse {
                    request_id: read.request_id,
                    result: StorageResult::Read {
                        data: ProtocolBytes::new(bytes),
                        revision: Some("saved".into()),
                    },
                }),
            );
            for _ in 0..32 {
                session.drive(RuntimeDriveBudget::default()).unwrap();
                drain(&mut session);
                if session.phase() == RuntimePhase::WaitingInput {
                    break;
                }
            }
            assert_eq!(session.phase(), RuntimePhase::WaitingInput);
            let vm = session.vm.as_ref().unwrap();
            let values = (10..=15)
                .map(|index| read_runtime_integer(vm, "RESULT", &[index], None).unwrap())
                .collect::<Vec<_>>();
            // Both reference engines include VAREXT records only in binary saves.
            // Text loads clear declared global data, retaining the existing table schema.
            let expected = if binary_format {
                [1, 7, 55, 1, 12, 12]
            } else {
                [1, 7, 55, 0, 0, 99]
            };
            assert_eq!(values, expected, "{profile:?}, binary={binary_format}");
            let results = runtime_variable_key(vm, "RESULTS").unwrap();
            assert_eq!(
                vm.vm().read_variable(results, &[10], None).unwrap(),
                VmValue::String(if binary_format { "saved" } else { "" }.into())
            );
            let VmValue::String(xml) = vm.vm().read_variable(results, &[11], None).unwrap() else {
                panic!("XML result")
            };
            if binary_format {
                assert!(xml.contains("saved") && !xml.contains("changed"));
            } else {
                assert!(xml.is_empty());
            }
        }
    }
}

#[test]
fn global_storage_missing_file_returns_zero_without_clearing_state() {
    let mut session = global_storage_fixture(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        "GLOBAL:0 = 66\nFLAG:0 = 55\nRESULT = 777\nLOADGLOBAL\nRESULT:10 = RESULT",
        true,
    );
    let read = next_global_storage_request(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: read.request_id,
            result: StorageResult::Error {
                error: era_runtime_protocol::FrontendIoError {
                    kind: FrontendIoErrorKind::NotFound,
                    message: "missing fixture".into(),
                    platform_code: None,
                },
            },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[10], None).unwrap(), 0);
    assert_eq!(read_runtime_integer(vm, "GLOBAL", &[0], None).unwrap(), 66);
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 55);
}

#[test]
fn global_storage_corruption_and_wrong_profile_preserve_vm_and_replay() {
    for corrupt in [true, false] {
        let (mut session, read, _) = global_fixture_at_load(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
            true,
        );
        let bytes = if corrupt {
            b"corrupt global save".to_vec()
        } else {
            global_fixture_at_load(erabasic_compat::CompatibilityProfileId::EmueraEm, true).2
        };
        let replay = session.input_replay.encode().unwrap();
        let before = session
            .vm
            .as_ref()
            .unwrap()
            .structured_extensions(StructuredScope::Global)
            .unwrap();
        let error = session
            .complete_storage(
                99,
                StorageResponse {
                    request_id: read.request_id,
                    result: StorageResult::Read {
                        data: ProtocolBytes::new(bytes),
                        revision: None,
                    },
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("global save"), "{error}");
        let vm = session
            .vm
            .as_ref()
            .expect("failed GLOBAL load must retain the VM");
        assert_eq!(read_runtime_integer(vm, "GLOBAL", &[0], None).unwrap(), 66);
        assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 55);
        assert_eq!(
            vm.structured_extensions(StructuredScope::Global).unwrap(),
            before
        );
        assert_eq!(session.input_replay.encode().unwrap(), replay);
    }
}
