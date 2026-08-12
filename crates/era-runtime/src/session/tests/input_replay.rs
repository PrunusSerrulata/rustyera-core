use super::*;

fn prepare() -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "input-replay-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en-US".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 9,
            files: vec![SubmittedFile {
                relative_path: "replay.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nINPUT\nPRINTFORML got={RESULT}\nINPUT\nRETURN\n".into(),
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
            mode: StartMode::NewGame {
                seed: Some(u64::MAX),
            },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).expect("start");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    drain(&mut session);
    session
}

#[test]
fn accepted_input_exports_as_a_token_free_chunked_jsonl_segment() {
    let mut session = prepare();
    let wait = session
        .operations
        .active_input()
        .expect("first input wait")
        .wait
        .clone();
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: u64::MAX,
            intent: InputIntent::CommitText("37".into()),
            message_skip: true,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("accept input");
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    drain(&mut session);

    submit(
        &mut session,
        4,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::InputReplay,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("export replay");
    let descriptor = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: StateExportKind::InputReplay,
                result: StateExportResult::Ready { transfer },
            }) => Some(transfer),
            _ => None,
        })
        .expect("input replay descriptor");
    submit(
        &mut session,
        5,
        RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
            transfer_id: descriptor.transfer_id,
            offset: 0,
            maximum_bytes: u32::MAX,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("read replay");
    let chunk = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportChunk(chunk) => Some(chunk),
            _ => None,
        })
        .expect("input replay chunk");
    let bytes = chunk.data.as_slice();
    assert!(chunk.complete);
    assert_eq!(u64::try_from(bytes.len()).unwrap(), descriptor.total_bytes);
    assert_eq!(blake3::hash(bytes).as_bytes(), descriptor.digest.as_slice());
    let jsonl = std::str::from_utf8(bytes).expect("UTF-8 JSONL");
    assert!(jsonl.contains(r#""kind":"new_game""#));
    assert!(jsonl.contains(&format!(r#""seed":"{}""#, u64::MAX)));
    assert!(jsonl.contains(r#""action":"text""#));
    assert!(jsonl.contains(r#""value":"37""#));
    assert!(jsonl.contains(r#""message_skip":true"#));
    assert!(!jsonl.contains("submission_token"));
    assert!(!jsonl.contains("wait_id"));
    assert!(!jsonl.contains("interaction_token"));
}

#[test]
fn input_replay_import_is_rejected_before_allocating_a_transfer() {
    let mut session = prepare();
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::InputReplay,
            total_bytes: 2,
            digest: ProtocolBytes::new(vec![0; 32]),
            artifact_id: None,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("reject import");
    assert!(drain(&mut session).into_iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message.contains("export-only")
    )));
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn replay_metadata_can_drive_the_same_semantic_path_with_a_fresh_session_token() {
    let mut first = prepare();
    submit_current_text(&mut first, 3, "37");
    let first_replay = first.input_replay.encode().expect("encode first replay");
    let records = std::str::from_utf8(&first_replay)
        .expect("first replay is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse replay record"))
        .collect::<Vec<_>>();
    assert_eq!(records[0]["origin"]["kind"], "new_game");
    assert_eq!(records[0]["origin"]["seed"], u64::MAX.to_string());
    assert_eq!(records[1]["action"], "text");

    let mut second = prepare();
    let second_token = second
        .operations
        .active_input()
        .expect("second session input")
        .wait
        .submission_token;
    submit_current_text(&mut second, 3, records[1]["text"].as_str().unwrap());
    let second_replay = second.input_replay.encode().expect("encode second replay");
    assert_eq!(
        records[1],
        serde_json::from_str::<serde_json::Value>(
            std::str::from_utf8(&second_replay)
                .expect("second replay is UTF-8")
                .lines()
                .nth(1)
                .expect("second replay step")
        )
        .expect("parse second replay step")
    );
    assert!(second_token.epoch > 0);
    assert!(
        !second_replay
            .windows(b"submission_token".len())
            .any(|window| { window == b"submission_token" })
    );
}

#[test]
fn cancelling_an_input_replay_transfer_leaves_history_available_for_a_new_export() {
    let mut session = prepare();
    submit(
        &mut session,
        3,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::InputReplay,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("start replay export");
    drain(&mut session);
    assert!(session.outbound_transfer.is_some());

    submit(
        &mut session,
        4,
        RuntimeMessage::StateExportCancel(StateExportCancel {
            kind: StateExportKind::InputReplay,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("cancel replay export");
    drain(&mut session);
    assert!(session.outbound_transfer.is_none());

    submit(
        &mut session,
        5,
        RuntimeMessage::StateExportRequest(StateExportRequest {
            kind: StateExportKind::InputReplay,
            snapshot_purpose: SnapshotExportPurpose::Normal,
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("restart replay export");
    assert!(drain(&mut session).into_iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            kind: StateExportKind::InputReplay,
            result: StateExportResult::Ready { .. },
        })
    )));
}

#[test]
fn failed_timeline_load_preserves_the_current_replay_segment() {
    let mut session = prepare();
    submit_current_text(&mut session, 3, "37");
    let before = session
        .input_replay
        .encode()
        .expect("encode replay before failed load");

    session
        .start_traditional_save(4, b"not a traditional save")
        .expect("invalid load is rejected without a runtime error");
    drain(&mut session);

    assert_eq!(
        session
            .input_replay
            .encode()
            .expect("encode replay after failed load"),
        before
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn successful_global_and_character_loads_replace_the_segment_and_record_next_input() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "external-data-replay-test".into(),
            features: vec![RuntimeFeature::Storage],
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
            files: vec![SubmittedFile {
                relative_path: "external.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nSAVEGLOBAL\nLOADGLOBAL\nADDVOIDCHARA\nSAVECHARA \"replay\", \"character\", 0\nDELCHARA 0\nLOADCHARA \"replay\"\nINPUT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let report = drain(&mut session);
    assert!(
        report.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )),
        "{report:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(7) },
        }),
    );

    let mut sequence = 3;
    let mut stored = BTreeMap::<String, Vec<u8>>::new();
    let mut saw_global = false;
    let mut saw_character = false;
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        for message in messages {
            let RuntimeMessage::StorageRequest(request) = message else {
                continue;
            };
            let result = match request.operation {
                StorageOperation::Write { data, .. } => {
                    stored.insert(request.relative_path.clone(), data.as_slice().to_vec());
                    StorageResult::Written {
                        revision: Some("r1".into()),
                    }
                }
                StorageOperation::Read => StorageResult::Read {
                    data: ProtocolBytes::new(
                        stored
                            .get(&request.relative_path)
                            .expect("load follows matching save")
                            .clone(),
                    ),
                    revision: Some("r1".into()),
                },
                operation => panic!("unexpected storage operation: {operation:?}"),
            };
            submit(
                &mut session,
                sequence,
                RuntimeMessage::StorageResponse(StorageResponse {
                    request_id: request.request_id,
                    result,
                }),
            );
            sequence += 1;
        }
        if let Some(kind) = input_replay_records(&session)[0]["origin"]["data_type"].as_str() {
            saw_global |= kind == "global";
            saw_character |= kind == "character";
        }
        if saw_character && session.operations.active_input().is_some() {
            break;
        }
    }
    assert!(saw_global && saw_character);
    let records = input_replay_records(&session);
    assert_eq!(records[0]["origin"]["kind"], "external_data_load");
    assert_eq!(records[0]["origin"]["storage_path"], "chara_replay.dat");
    assert_eq!(records[0]["step_count"], 0);

    submit_current_text(&mut session, sequence, "5");
    let records = input_replay_records(&session);
    assert_eq!(records[0]["step_count"], 1);
    assert_eq!(records[1]["action"], "text");
}

fn submit_current_text(session: &mut RuntimeSession, sequence: u64, text: &str) {
    let wait = session
        .operations
        .active_input()
        .expect("current replay input")
        .wait
        .clone();
    submit(
        session,
        sequence,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::CommitText(text.into()),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("replay input");
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    drain(session);
}
