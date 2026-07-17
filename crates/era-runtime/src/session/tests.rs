use era_debug_protocol::{DEBUG_PROTOCOL_VERSION, DebugHello, DebugMessage, DebugScope};
use era_protocol::{Channel, Envelope, ProtocolBytes, decode_envelope, encode_envelope};
use era_runtime_protocol::{FileCategory, FileChange, FilePayload, ProjectManifest, SubmittedFile};

use super::*;

fn capabilities() -> ClientCapabilities {
    ClientCapabilities {
        input_modalities: vec![era_runtime_protocol::InputModality::Keyboard],
        rich_text: false,
        html: false,
        graphics: false,
        audio: false,
        video: false,
        font_metrics: false,
        column_cells: true,
        separators: true,
        available_fonts: vec!["sans-serif".into()],
        services: vec![
            ServiceCapability {
                kind: ServiceKind::Clock,
                operation: LOCAL_DATE_TIME_OPERATION.into(),
                versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::Entropy,
                operation: RANDOM_SEED_OPERATION.into(),
                versions: VersionRange::exact(RANDOM_SEED_OPERATION_VERSION),
            },
            ServiceCapability {
                kind: ServiceKind::InputState,
                operation: GET_KEY_STATE_OPERATION.into(),
                versions: VersionRange::exact(GET_KEY_STATE_OPERATION_VERSION),
            },
        ],
        storage: StorageCapabilities {
            revisions: true,
            atomic_replace: true,
            missing_precondition: true,
            delete: true,
        },
    }
}

#[allow(clippy::needless_pass_by_value)]
fn submit(session: &mut RuntimeSession, sequence: u64, message: RuntimeMessage) {
    let mut envelope = Envelope::new(
        Channel::Runtime,
        RUNTIME_PROTOCOL_VERSION,
        sequence,
        sequence + 1,
        message.tag(),
        ProtocolBytes::new(message.encode_payload().expect("encode message")),
    );
    if sequence != 0 {
        envelope.session = Some(session.options.session_id);
        envelope.session_epoch = Some(session.epoch);
    }
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    session.submit_envelope(&bytes).expect("submit envelope");
}

fn drain(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
    let mut messages = Vec::new();
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).expect("decode envelope");
        messages.push(RuntimeMessage::from_envelope(&envelope).expect("decode message"));
    }
    messages
}

fn submit_debug(session: &mut RuntimeSession, sequence: u64, message: &DebugMessage) {
    let envelope = message
        .envelope(
            Some(session.options.session_id),
            Some(session.epoch),
            sequence,
            10_000 + sequence,
            None,
        )
        .expect("debug envelope");
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode debug");
    session.submit_envelope(&bytes).expect("submit debug");
}

#[test]
fn handshake_selects_only_implemented_features() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::Audio, RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("drive");
    let messages = drain(&mut session);
    let RuntimeMessage::ServerHello(hello) = &messages[0] else {
        panic!("expected server hello");
    };
    assert_eq!(hello.selected_version, RUNTIME_PROTOCOL_VERSION);
    assert!(hello.features.contains(&RuntimeFeature::TimedInput));
    assert!(!hello.features.contains(&RuntimeFeature::Audio));
    assert_eq!(hello.selected_capabilities.storage, capabilities().storage);
}

#[test]
fn debug_channel_has_independent_sequence_and_cannot_widen_creator_policy() {
    let mut session = RuntimeSession::new(RuntimeOptions {
        debug_scope_mask: (1 << 2) | (1 << 5),
        ..RuntimeOptions::default()
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "debug-test".into(),
            features: Vec::new(),
            capabilities: capabilities(),
            requested_limits: RuntimeOptions::default().limits,
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);

    submit_debug(
        &mut session,
        0,
        &DebugMessage::Hello(DebugHello {
            versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
            requested_scopes: vec![
                DebugScope::ExecutionControl,
                DebugScope::VariablesWrite,
                DebugScope::GameFieldsRead,
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let bytes = session.poll_envelope().expect("debug grant");
    let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
    let DebugMessage::Grant(grant) = DebugMessage::from_envelope(&envelope).unwrap() else {
        panic!("expected debug grant");
    };
    assert_eq!(
        grant.scopes,
        vec![DebugScope::GameFieldsRead, DebugScope::ExecutionControl]
    );
    assert_eq!(grant.token.session_epoch, session.epoch.0);
}

#[test]
fn debugger_pause_freezes_frontend_time_until_resume() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::DebugPaused;
    session.logical_time_ns = 500;
    session.frontend_time_origin = Some((10, 500));
    session
        .handle_message(
            1,
            RuntimeMessage::AdvanceTime(AdvanceTime {
                monotonic_time_ns: 1_000,
            }),
        )
        .unwrap();
    assert_eq!(session.logical_time_ns, 500);
    session.resume_debug_time();
    assert_eq!(session.frontend_time_origin, Some((1_000, 500)));
}

#[test]
fn ready_project_reload_stages_and_commits_a_normalized_delta() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "reload-test".into(),
            features: vec![RuntimeFeature::ProjectReload],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
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
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "./main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL reloaded\nRETURN\n".into()),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .manifest
            .project_revision,
        2
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(era_runtime_protocol::ProjectLoadReport {
            project_revision: 2,
            success: true,
            ..
        })
    )));
}

#[test]
fn state_import_rejects_out_of_order_chunks_and_bad_digests() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TraditionalSave],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: 3,
            digest: ProtocolBytes::new([0; 32]),
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();

    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 1,
            data: ProtocolBytes::new([b'a']),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(*b"abc"),
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn training_reset_updates_shared_and_all_character_state_atomically() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let source = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "SOURCE")
        .expect("SOURCE")
        .key;
    let tflag = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "TFLAG")
        .expect("TFLAG")
        .key;
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: source,
                    indices: vec![0],
                    character: Some(0),
                    value: VmValue::Integer(9),
                },
                VmRuntimeWrite {
                    variable: tflag,
                    indices: vec![0],
                    character: None,
                    value: VmValue::Integer(7),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .expect("prepare dirty state");
    vm.commit_runtime_state(dirty).expect("commit dirty state");
    reset_training_state(&mut vm).expect("reset training state");
    assert_eq!(
        vm.read_runtime_state(&[
            erabasic_vm::VmRuntimeRead {
                variable: source,
                indices: vec![0],
                character: Some(0),
            },
            erabasic_vm::VmRuntimeRead {
                variable: tflag,
                indices: vec![0],
                character: None,
            },
        ]),
        Ok(vec![VmValue::Integer(0), VmValue::Integer(0)])
    );
}

#[test]
fn shop_purchase_validates_stock_and_commits_money_item_and_bought_together() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "ITEM.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("5,potion,120\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let mut vm = RuntimeVm::new(artifact, VmConfig::default());
    let sales = runtime_variable_key(&vm, "ITEMSALES").unwrap();
    let money = runtime_variable_key(&vm, "MONEY").unwrap();
    let dirty = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: sales,
                    indices: vec![5],
                    character: None,
                    value: VmValue::Integer(1),
                },
                VmRuntimeWrite {
                    variable: money,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(200),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .unwrap();
    vm.commit_runtime_state(dirty).unwrap();
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::Purchased
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(&vm, "BOUGHT", &[], None).unwrap(), 5);
    assert_eq!(
        purchase_item(&mut vm, 5, 100).unwrap(),
        PurchaseResult::NotEnoughMoney
    );
    assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
    assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
}

#[test]
fn train_controller_consumes_runtime_button_intent_and_loops_after_eventcomend() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let source = "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\nRETURN\n@EVENTCOM\nRETURN\n@COM0\nFLAG:0 += 1\nRESULT = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@EVENTCOMEND\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(3) },
        }),
    );
    for _ in 0..12 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let pending = session
        .operations
        .active_input()
        .expect("training command wait");
    let token = *pending.choices.keys().next().expect("PRINTBUTTON token");
    let wait_id = pending.wait.wait_id;
    let submission_token = pending.wait.submission_token;
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            intent: InputIntent::Activate(token),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some()
            && session.controller.step == SystemStep::TrainShowUser
        {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:#?}");
    let vm = session.vm.as_ref().expect("running VM");
    assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 1);
    assert_eq!(
        read_runtime_integer(vm, "SOURCE", &[0], Some(0)).unwrap(),
        0
    );
    assert_eq!(
        session.controller.step,
        SystemStep::TrainShowUser,
        "phase={:?}",
        session.phase
    );
}

#[test]
fn project_load_start_and_print_cross_the_message_boundary() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL hello\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(loaded.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report) if report.success
    )));
    assert_eq!(session.phase(), RuntimePhase::Ready);

    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
    }
    assert_eq!(session.random_seed(), Some(1));
    let output = drain(&mut session);
    assert!(output.iter().any(|message| match message {
        RuntimeMessage::PresentationSnapshot(snapshot) =>
            snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("hello")
                ))
            }),
        _ => false,
    }));
}

#[test]
fn runtime_metadata_queries_use_the_active_artifact_and_fiber() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "metadata-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
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
                    relative_path: "metadata.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\n#DIMS VALUES, 3\n#DIMS CHOICES, 5\nCALL SIZE_OF, CHOICES\nPRINTFORML meta={VARSIZE(\"VALUES\")},{EXISTFUNCTION(\"SYSTEM_TITLE\")},{EXISTVAR(\"VALUES\")},%GETDOINGFUNCTION()%,{RESULT},%CHOICES:2%\nPRINTFORML funcs={ENUMFUNCWITH(\"SIZE\", CHOICES)},%CHOICES:0%\nPRINTFORML vars={ENUMVARWITH(\"SAVEDATA_TEXT\", CHOICES)},%CHOICES:0%\nCALL ORACLE_REFLECTION\nPRINTFORML reflection={RESULT:12},{RESULT:13},%RESULTS:8%,%RESULTS:9%\nRETURN\n@SIZE_OF(refChoices)\n#DIMS REF refChoices, 0\nrefChoices:2 = \"bound\"\nRESULT = VARSIZE(\"refChoices\")\nRETURN\n@ORACLE_REFLECTION\n#DIMS NAMES, 4\nRESULT:12 = ENUMFUNCWITH(\"ORACLE_REFLECTION\", NAMES)\nRESULTS:8 = %NAMES:0%\nRESULT:13 = ENUMVARWITH(\"SAVEDATA_TEXT\", NAMES)\nRESULTS:9 = %NAMES:0%\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        }),
        "{loaded:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let output = drain(&mut session);
    assert!(
        output.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| {
                    matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text { text, .. }
                            if text.contains("meta=3,1,0,SYSTEM_TITLE,5,bound")
                    )
                })
            }),
            _ => false,
        }),
        "{output:#?}"
    );
    let rendered = output
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .flat_map(|snapshot| snapshot.lines.iter())
        .flat_map(|line| line.runs.iter())
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(rendered.contains("funcs=1,SIZE_OF"), "{rendered}");
    assert!(rendered.contains("vars=1,SAVEDATA_TEXT"), "{rendered}");
    assert!(
        rendered.contains("reflection=1,1,ORACLE_REFLECTION,SAVEDATA_TEXT"),
        "{rendered}\n{output:#?}"
    );
}

#[test]
fn reference_presentation_fixture_preserves_logical_intent() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    // Keep this body identical to ORACLE_PRESENTATION in the C# fixture so
    // the oracle and Rust tests exercise the same EraBasic commands.
    let source = "@SYSTEM_TITLE\nPRINTBUTTON \"A\", 1\nPRINTBUTTONC \"B\", 2\nPRINTBUTTONLC \"C\", 3\nPRINTL\nDRAWLINE\nNOSKIP\nPRINTL VISIBLE\nENDNOSKIP\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        if session.phase() == RuntimePhase::Ready {
            break;
        }
    }
    let output = drain(&mut session);
    let snapshot = output
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .next_back()
        .expect("presentation snapshot");

    assert!(snapshot.lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Button { runs, .. }
                    if runs.iter().any(|run| matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text { text, .. } if text == "A"
                    ))
            )
        })
    }));
    assert_eq!(
        snapshot
            .lines
            .iter()
            .flat_map(|line| &line.runs)
            .filter(|run| matches!(run, era_runtime_protocol::DisplayRun::ColumnCell { .. }))
            .count(),
        2
    );
    assert!(snapshot.lines.iter().any(|line| {
        line.runs
            .iter()
            .any(|run| matches!(run, era_runtime_protocol::DisplayRun::Separator { .. }))
    }));
    assert!(snapshot.lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text { text, .. } if text == "VISIBLE"
            )
        })
    }));
}

#[test]
fn typed_input_updates_result_and_sixth_argument_honors_message_skip() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nTINPUT 1000, 7, 1, \"timeout\", 0, 0\nTINPUT 1000, 9, 1, \"timeout\", 0, 0\nPRINTFORML got={RESULT}\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).expect("wait");
    }
    let opened = drain(&mut session);
    let wait = opened
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait.clone()),
            _ => None,
        })
        .expect("runtime should publish the input wait");
    assert_eq!(
        wait.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert_eq!(wait.stability, WaitStability::Transient);

    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 10,
            intent: InputIntent::CommitText("42".into()),
            message_skip: true,
        }),
    );
    for _ in 0..4 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("resume");
    }
    let output = drain(&mut session);
    assert!(output.iter().any(|message| match message {
        RuntimeMessage::PresentationSnapshot(snapshot) =>
            snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("got=9")
                ))
            }),
        _ => false,
    }));
}

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
    let output = drain(&mut restored);
    let display = output
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
            _ => None,
        })
        .flat_map(|snapshot| &snapshot.lines)
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|");
    let loadend = display.find("loadend=37").expect("SYSTEM_LOADEND output");
    let eventload = display.find("eventload").expect("EVENTLOAD output");
    let shop = display.find("shop").expect("SHOW_SHOP output");
    assert!(loadend < eventload && eventload < shop, "{display}");
    assert!(!display.contains("unexpected-autosave"), "{display}");

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
    let restored_wait = exact.operations.active_input().expect("restored wait");
    assert_eq!(exact.phase(), RuntimePhase::WaitingInput);
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
    assert!(session.presentation.snapshot().lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Text {
                    system_text: Some(reference),
                    ..
                } if reference.key == SystemTextKey::LoadQuestion
            )
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
                    entries: vec![StorageEntry {
                        relative_path: "save01.sav".into(),
                        byte_length: 3,
                        revision: Some("r1".into()),
                    }],
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
    session
        .complete_storage(
            3,
            StorageResponse {
                request_id: scan.request_id,
                result: StorageResult::Read {
                    data: ProtocolBytes::new(b"bad".to_vec()),
                    revision: Some("r1".into()),
                },
            },
        )
        .unwrap();
    assert!(session.invalid_slot_paths.contains("save01.sav"));
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
            .any(|value| value == &VmValue::Integer(-1_001))
    );
    session
        .finish_system_input(pending, &VmValue::Integer(-1_001))
        .unwrap();
    let delete = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .expect("revision-bound slot delete");
    assert_eq!(delete.relative_path, "save01.sav");
    assert!(matches!(
        delete.operation,
        StorageOperation::Delete {
            precondition: StoragePrecondition::Revision(ref revision),
        } if revision == "r1"
    ));
    session
        .complete_storage(
            4,
            StorageResponse {
                request_id: delete.request_id,
                result: StorageResult::Error {
                    error: era_runtime_protocol::FrontendIoError {
                        kind: FrontendIoErrorKind::Conflict,
                        message: "changed".into(),
                        platform_code: None,
                    },
                },
            },
        )
        .unwrap();
    assert!(session.operations.active_input().is_some());
    let pending = session.operations.take_active_input().unwrap();
    session
        .finish_system_input(pending, &VmValue::Integer(-1))
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
#[allow(clippy::too_many_lines)]
fn saveinfo_candidate_is_isolated_until_the_storage_commit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "candidate-test".into(),
            features: vec![RuntimeFeature::Storage],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
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
                    relative_path: "candidate.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nWAIT\nRETURN\n@SAVEINFO\nRESULT = 99\nRESULT:1 = GETCONFIG(\"Font size\")\nRESULTS:1 = %BARSTR(2, 4, 4)%\nPUTFORM suffix\nRETURN\n"
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
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    drain(&mut session);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0
    );

    let time = LocalDateTimeResponse {
        year: 2026,
        month: 7,
        day: 17,
        hour: 12,
        minute: 34,
        second: 56,
        millisecond: 0,
        utc_offset_minutes: 480,
    };
    let mut live = session.vm.take().unwrap();
    session
        .begin_candidate_save(&mut live, 99, CandidateSaveContinuation::Autosave)
        .unwrap();
    session.vm = Some(live);
    let stat_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(stat_request.operation, StorageOperation::Stat);
    session
        .complete_storage(
            0,
            StorageResponse {
                request_id: stat_request.request_id,
                result: StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                    byte_length: 12,
                    revision: Some("slot-rev".into()),
                }),
            },
        )
        .unwrap();
    let clock_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    session
        .complete_service(
            0,
            ServiceResponse {
                request_id: clock_request.request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(encode_canonical(&time).unwrap()),
                },
            },
        )
        .unwrap();
    let write_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    let StorageOperation::Write {
        data: bytes,
        atomic_replace,
        precondition,
    } = write_request.operation
    else {
        panic!("candidate did not issue a write")
    };
    assert!(atomic_replace);
    assert_eq!(
        precondition,
        StoragePrecondition::Revision("slot-rev".into())
    );
    let decoded = decode_scoped_save(
        bytes.as_slice(),
        session.vm.as_ref().unwrap().vm().artifact(),
        era_runtime_save::SaveFileKind::Normal,
    )
    .unwrap();
    assert_eq!(decoded.description, "2026/07/17 12:34:56 suffix");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        0,
        "candidate mutation leaked before commit"
    );
    session
        .complete_storage(
            0,
            StorageResponse {
                request_id: write_request.request_id,
                result: StorageResult::Written {
                    revision: Some("new-rev".into()),
                },
            },
        )
        .unwrap();
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        99
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        18
    );
    let vm = session.vm.as_ref().unwrap();
    let results = runtime_variable_key(vm, "RESULTS").unwrap();
    assert_eq!(
        vm.vm().read_variable(results, &[1], None),
        Ok(VmValue::String("[**..]".into()))
    );
}

#[test]
fn sequence_gaps_are_rejected_before_execution() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let message = RuntimeMessage::ClientHello(ClientHello {
        runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
        client_name: "test".into(),
        features: Vec::new(),
        requested_limits: RuntimeOptions::default().limits,
        capabilities: capabilities(),
        preferred_locales: vec!["ja".into()],
    });
    let envelope = message
        .envelope(None, None, 2, 1, None)
        .expect("create envelope");
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    assert!(matches!(
        session.submit_envelope(&bytes),
        Err(RuntimeError::InvalidSequence {
            expected: 0,
            actual: 2
        })
    ));
}

#[test]
fn active_session_rejects_stale_epochs_and_acknowledges_journal_entries() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::StateResynchronization],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    assert_eq!(session.outbound_journal.len(), 1);

    let ack = RuntimeMessage::Acknowledge(era_runtime_protocol::SequenceAcknowledgement {
        through_sequence: 0,
    });
    submit(&mut session, 1, ack);
    session.drive(RuntimeDriveBudget::default()).expect("ack");
    assert!(session.outbound_journal.is_empty());

    let message = RuntimeMessage::AdvanceTime(AdvanceTime {
        monotonic_time_ns: 1,
    });
    let mut envelope = message
        .envelope(
            Some(session.options.session_id),
            Some(SessionEpoch(session.epoch.0.saturating_sub(1))),
            2,
            3,
            None,
        )
        .expect("stale envelope");
    envelope.session = Some(session.options.session_id);
    let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
    assert!(matches!(
        session.submit_envelope(&bytes),
        Err(RuntimeError::SessionMismatch)
    ));
}

#[test]
fn configuration_is_parsed_and_resources_receive_stable_identities() {
    let build = build_project(
        &ProjectManifest {
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
                    payload: FilePayload::Utf8("Language=Chinese".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8("; name,path".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    assert!(build.report.success);
    let codes: Vec<_> = build
        .report
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.as_str())
        .collect();
    assert!(codes.contains(&"runtime.invalid_configuration"));
    assert!(!codes.contains(&"runtime.resource_manifest_deferred"));
    let snapshot = build.snapshot.expect("normalized project snapshot");
    assert_eq!(snapshot.resources.len(), 1);
    assert_eq!(snapshot.resources[0].relative_path, "resources.csv");
    assert_eq!(
        snapshot.resources[0].payload_digest,
        *blake3::hash(b"; name,path").as_bytes()
    );
    assert_ne!(snapshot.project_identity, [0; 32]);
}

#[test]
fn frontend_calendar_values_match_dotnet_datetime_shapes() {
    let time = LocalDateTimeResponse {
        year: 2026,
        month: 7,
        day: 15,
        hour: 13,
        minute: 4,
        second: 5,
        millisecond: 6,
        utc_offset_minutes: 480,
    };
    assert_eq!(calendar_number(time), 20_260_715_130_405_006);
    assert_eq!(calendar_string(time), "2026/07/15 13:04:05");
    assert_eq!(
        milliseconds_since_year_one(LocalDateTimeResponse {
            year: 1,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
            millisecond: 0,
            utc_offset_minutes: 0,
        }),
        0
    );
    assert_eq!(milliseconds_since_year_one(time) / 1_000, 63_919_717_445);
}

#[test]
fn deterministic_width_and_integer_format_tables_cover_era_usage() {
    assert_eq!(to_full_width("ABC 123 ｶﾞﾊﾟ"), "ＡＢＣ　１２３　ガパ");
    assert_eq!(to_half_width("ＡＢＣ　１２３　ガパ"), "ABC 123 ｶﾞﾊﾟ");
    assert_eq!(convert_kana_mode("あいうガ", 1), "アイウガ");
    assert_eq!(convert_kana_mode("アイウガ", 2), "あいうが");
    assert_eq!(convert_kana_mode("ｶﾞ ABC", 3), "が　ＡＢＣ");
    assert_eq!(format_era_integer(12_345, "#,##0"), Ok("12,345".into()));
    assert_eq!(format_era_integer(-7, "D3"), Ok("-007".into()));
    assert_eq!(format_era_integer(255, "X4"), Ok("00FF".into()));
}

#[test]
fn reference_bar_and_portable_named_colors_are_deterministic() {
    assert_eq!(make_bar(5, 10, 4, '*', '.'), Ok("[**..]".into()));
    assert_eq!(
        make_bar(1, 0, 4, '*', '.'),
        Err("BAR maximum must be positive")
    );
    assert_eq!(
        make_bar(1, 2, 100, '*', '.'),
        Err("BAR length must be between 1 and 99")
    );
    assert_eq!(named_color("Magenta"), Some(0x00ff_00ff));
    assert_eq!(named_color("transparent"), None);
}

#[test]
fn flow_input_configuration_shapes_system_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let ordinary = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    assert_eq!(ordinary.kind, WaitKind::IntegerButton);
    session.flow_input_enabled = true;
    session.flow_input_default = 7;
    let integer = session.system_wait(InteractionToken { epoch: 0, id: 2 });
    assert_eq!(integer.kind, WaitKind::IntegerValue);
    assert_eq!(
        integer.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert!(integer.mouse_input);
    session.flow_input_string = true;
    session.flow_input_default_string = "fallback".into();
    let string = session.system_wait(InteractionToken { epoch: 0, id: 3 });
    assert_eq!(string.kind, WaitKind::StringValue);
    assert_eq!(
        string.default_value,
        Some(era_runtime_protocol::ProtocolValue::String(
            "fallback".into()
        ))
    );
}

#[test]
fn frontend_monotonic_time_rebases_onto_restored_logical_time() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.logical_time_ns = 100;
    assert_eq!(session.observe_frontend_time(5), 100);
    assert_eq!(session.observe_frontend_time(15), 110);
    assert_eq!(session.observe_frontend_time(9), 110);
}

#[test]
fn primitive_input_uses_runtime_selection_tokens_and_rejects_timeout_spoofing() {
    let submission = InteractionToken { epoch: 7, id: 1 };
    let selection = InteractionToken { epoch: 7, id: 2 };
    let pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 9,
            kind: WaitKind::PrimitiveMouseKey,
            stability: WaitStability::Transient,
            one_input: false,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: None,
            deadline_ns: Some(10),
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
        },
        result_name: Some("RESULT".into()),
        choices: BTreeMap::from([(selection, VmValue::Integer(42))]),
        timeout_duration_ns: Some(10),
        post_input: None,
    };
    let input = era_runtime_protocol::PrimitiveInput {
        input_type: 1,
        result_1: 10,
        result_2: 20,
        result_3: 1,
        result_4: 3,
        selection_token: Some(selection),
    };
    assert_eq!(
        input_value(&pending, submission, InputIntent::Primitive(input)),
        Some(InputSubmission::Primitive(PrimitiveResult {
            fields: [1, 10, 20, 1, 3],
            selection: Some(VmValue::Integer(42)),
        }))
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(selection)),
        Some(InputSubmission::Value(VmValue::Integer(42)))
    );
    assert!(
        input_value(
            &pending,
            InteractionToken { epoch: 7, id: 99 },
            InputIntent::Activate(selection),
        )
        .is_none()
    );
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::Primitive(era_runtime_protocol::PrimitiveInput {
                input_type: 4,
                result_1: 0,
                result_2: 0,
                result_3: 0,
                result_4: 0,
                selection_token: None,
            }),
        )
        .is_none()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn project_resource_metadata_is_frontend_decoded_before_load_commit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "resource-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/sprites.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8("FACE,face.png".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    let request_id = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("image metadata request");
    submit(
        &mut session,
        2,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 32,
                        height: 16,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16))
    );

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let reload_messages = drain(&mut session);
    let reload_request = reload_messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("changed image metadata request");
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16)),
        "the live graph must not change before candidate metadata commits"
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: reload_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 64,
                        height: 24,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success && report.project_revision == 2)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((64, 24))
    );

    submit(
        &mut session,
        5,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("second changed image metadata request");
    submit(
        &mut session,
        6,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: failed_request,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "decoder.invalid".into(),
                    message: "invalid image".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed = drain(&mut session);
    assert!(failed.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if !report.success && report.project_revision == 3)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .map(|project| project.manifest.project_revision),
        Some(2),
        "failed candidate metadata must leave the previous project authoritative"
    );
}

#[test]
fn font_profile_is_session_fixed_case_insensitive_and_deterministic() {
    let mut requested = capabilities();
    requested.available_fonts = vec!["Zeta".into(), "alpha".into(), "ALPHA".into()];
    let selected = selected_capabilities(&requested);
    assert_eq!(selected.available_fonts, vec!["alpha", "Zeta"]);
}

#[test]
fn effect_acknowledgements_are_exact_and_failures_become_diagnostics() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.epoch = SessionEpoch(1);
    session
        .emit_effect(EffectKind::StartAnimation("flash".into()))
        .expect("emit effect");
    let _ = drain(&mut session);
    session
        .handle_message(
            10,
            RuntimeMessage::EffectAcknowledgement(EffectAcknowledgement {
                outcomes: vec![era_runtime_protocol::EffectOutcome {
                    effect_id: 1,
                    status: EffectOutcomeStatus::Failed,
                    message: Some("device unavailable".into()),
                }],
            }),
        )
        .expect("acknowledge effect");
    assert!(session.effect_journal.is_empty());
    assert!(matches!(
        drain(&mut session).as_slice(),
        [RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })]
            if code == "runtime.device_effect_failed"
    ));
}
