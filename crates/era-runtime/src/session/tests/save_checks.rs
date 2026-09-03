use super::*;
use era_runtime_save::{SaveCodecLimits, SaveDocument, SaveFileKind, SaveFormat, SaveMetadata};

fn start_check(snake: bool, command: &str) -> (RuntimeSession, StorageRequest) {
    let source =
        format!("@SYSTEM_TITLE\nWAIT\n{command}\nPRINTFORML {{RESULT}}:%RESULTS%\nWAIT\nRETURN\n");
    let mut session = if snake {
        super::key_macro_input::start_snake_input_project(&source, capabilities())
    } else {
        super::key_macro_input::start_input_project(&source)
    };
    let wait = session.operations.active_input().unwrap().wait.clone();
    drain(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: false,
        }),
    );
    let request = (0..16)
        .find_map(|_| {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            drain(&mut session)
                .into_iter()
                .find_map(|message| match message {
                    RuntimeMessage::StorageRequest(request) => Some(request),
                    _ => None,
                })
        })
        .expect("save check request");
    assert_eq!(
        request.operation,
        StorageOperation::ReadRange {
            offset: 0,
            maximum_bytes: 64 * 1024,
            change_token: None
        }
    );
    (session, request)
}

fn respond(
    session: &mut RuntimeSession,
    sequence: u64,
    request_id: u64,
    result: StorageResult,
) -> Vec<RuntimeMessage> {
    submit(
        session,
        sequence,
        RuntimeMessage::StorageResponse(StorageResponse { request_id, result }),
    );
    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(session));
        if session.phase() != RuntimePhase::Running {
            break;
        }
    }
    messages
}

fn chunk(bytes: &[u8], offset: u64, complete: bool, token: &str) -> StorageResult {
    StorageResult::ReadChunk {
        data: ProtocolBytes::new(bytes.to_vec()),
        offset,
        complete,
        change_token: token.into(),
    }
}

fn assert_check(session: &RuntimeSession, status: i64, description: &str) {
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(
        read_runtime_integer(vm, "RESULT", &[], None).unwrap(),
        status
    );
    assert_eq!(read_runtime_string(vm, "RESULTS").unwrap(), description);
    assert!(
        session
            .presentation
            .log_text(false)
            .ends_with(&format!("{status}:{description}\r\n"))
    );
}

#[test]
fn save_checks_distinguish_missing_slots_from_io_errors_in_both_profiles() {
    for snake in [false, true] {
        for command in [
            "CHKDATA 99",
            "RESULT = CHKDATA(99)",
            "CHKCHARADATA \"slot\"",
        ] {
            for (kind, expected, status) in [
                (FrontendIoErrorKind::NotFound, "----", 1),
                (
                    FrontendIoErrorKind::PermissionDenied,
                    "permission denied",
                    4,
                ),
            ] {
                let (mut session, request) = start_check(snake, command);
                let message = if status == 1 {
                    "No such file or directory (os error 2)"
                } else {
                    expected
                };
                respond(
                    &mut session,
                    4,
                    request.request_id,
                    StorageResult::Error {
                        error: era_runtime_protocol::FrontendIoError {
                            kind,
                            message: message.into(),
                            platform_code: None,
                        },
                    },
                );
                assert_check(&session, status, expected);
            }
        }
    }
}

#[test]
fn save_checks_finish_at_the_header_without_reading_or_decoding_the_payload() {
    for snake in [false, true] {
        for format in [
            SaveFormat::Text1808,
            SaveFormat::Binary1808,
            SaveFormat::Binary1808Gzip,
        ] {
            let (mut session, request) = start_check(snake, "CHKDATA 0");
            let game = &session
                .vm
                .as_ref()
                .unwrap()
                .vm()
                .artifact()
                .project_data
                .static_data
                .game_base;
            let metadata = SaveMetadata {
                unique_code: game.unique_code,
                version: game.version,
                description: "slot description".into(),
            };
            let mut bytes = if format == SaveFormat::Text1808 {
                format!(
                    "{}\n{}\n{}\n",
                    metadata.unique_code, metadata.version, metadata.description
                )
                .into_bytes()
            } else {
                era_runtime_save::encode(
                    &SaveDocument {
                        format,
                        kind: SaveFileKind::Normal,
                        metadata,
                        characters: Vec::new(),
                        character_user_defined_starts: Vec::new(),
                        variables: Vec::new(),
                        opaque_extensions: Vec::new(),
                        text_payload: None,
                    },
                    format,
                    SaveCodecLimits::default(),
                )
                .unwrap()
            };
            if format != SaveFormat::Text1808 {
                bytes.truncate(
                    bytes.len()
                        - if format == SaveFormat::Binary1808Gzip {
                            8
                        } else {
                            1
                        },
                );
                if format == SaveFormat::Binary1808 {
                    bytes.push(0x80);
                }
                assert!(
                    era_runtime_save::decode_binary(&bytes, SaveCodecLimits::default()).is_err(),
                    "{format:?}"
                );
            }
            let messages = respond(
                &mut session,
                4,
                request.request_id,
                chunk(&bytes, 0, false, "v1"),
            );
            assert!(
                !messages
                    .iter()
                    .any(|message| matches!(message, RuntimeMessage::StorageRequest(_)))
            );
            assert_check(&session, 0, "slot description");
        }
    }
}

#[test]
fn save_checks_continue_split_headers_with_a_stable_change_token() {
    let (mut session, request) = start_check(true, "CHKDATA 0");
    let game = &session
        .vm
        .as_ref()
        .unwrap()
        .vm()
        .artifact()
        .project_data
        .static_data
        .game_base;
    let prefix = format!("{}\n{}\nslot ", game.unique_code, game.version);
    let messages = respond(
        &mut session,
        4,
        request.request_id,
        chunk(prefix.as_bytes(), 0, false, "v1"),
    );
    let next = messages
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    assert_eq!(next.relative_path, request.relative_path);
    assert_eq!(
        next.operation,
        StorageOperation::ReadRange {
            offset: prefix.len() as u64,
            maximum_bytes: 64 * 1024,
            change_token: Some("v1".into())
        }
    );
    respond(
        &mut session,
        5,
        next.request_id,
        chunk(b"description\n", prefix.len() as u64, false, "v1"),
    );
    assert_check(&session, 0, "slot description");
}

#[test]
fn save_checks_reject_corrupt_and_nonprogressing_headers() {
    for result in [
        chunk(b"broken\n", 0, true, "v1"),
        chunk(b"", 0, false, "v1"),
    ] {
        let (mut session, request) = start_check(true, "CHKDATA 0");
        respond(&mut session, 4, request.request_id, result);
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let vm = session.vm.as_ref().unwrap();
        assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 4);
        assert_ne!(read_runtime_string(vm, "RESULTS").unwrap(), "----");
    }
}

#[test]
fn redraw_disabled_save_checks_publish_the_page_only_at_input() {
    let mut session = super::key_macro_input::start_snake_input_project(
        "@SYSTEM_TITLE\nWAIT\nREDRAW 0\nFOR LOCAL, 0, 41\nCHKDATA LOCAL\nPRINTFORML [{LOCAL}] - %RESULTS%\nNEXT\nINPUT\nRETURN\n",
        capabilities(),
    );
    let initial_output = session.presentation.log_text(false);
    let wait = session.operations.active_input().unwrap().wait.clone();
    drain(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: false,
        }),
    );
    let mut sequence = 4;
    let mut checks = 0;
    for _ in 0..256 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            assert_save_page_before_input(&messages);
            break;
        }
        assert!(
            !messages.iter().any(|message| match message {
                RuntimeMessage::PresentationSnapshot(snapshot) =>
                    !snapshot.history.logical_lines.is_empty(),
                RuntimeMessage::PresentationDelta(delta) =>
                    delta.operations.iter().any(|operation| matches!(
                        operation,
                        PresentationOperation::AppendLine { .. }
                            | PresentationOperation::ReplaceLine { .. }
                    )),
                _ => false,
            }),
            "partial save page was published: {messages:#?}"
        );
        for message in messages {
            if let RuntimeMessage::StorageRequest(request) = message {
                assert_eq!(request.relative_path, save_slot_path(checks));
                checks += 1;
                submit(
                    &mut session,
                    sequence,
                    RuntimeMessage::StorageResponse(StorageResponse {
                        request_id: request.request_id,
                        result: StorageResult::Error {
                            error: era_runtime_protocol::FrontendIoError {
                                kind: FrontendIoErrorKind::NotFound,
                                message: "No such file".into(),
                                platform_code: None,
                            },
                        },
                    }),
                );
                sequence += 1;
            }
        }
    }
    assert_eq!(checks, 41);
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    let output = session.presentation.log_text(false);
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        initial_output
            .lines()
            .map(str::to_owned)
            .chain((0..41).map(|slot| format!("[{slot}] - ----")))
            .collect::<Vec<_>>()
    );
}

#[test]
fn save_checks_finish_invalid_responses_and_ignore_duplicate_completions() {
    let too_large = vec![b'x'; 64 * 1024 + 1];
    for result in [
        StorageResult::Listed {
            entries: Vec::new(),
        },
        chunk(b"0\n0\nx\n", 1, false, "v1"),
        chunk(&too_large, 0, false, "v1"),
    ] {
        let (mut session, request) = start_check(true, "CHKDATA 0");
        respond(&mut session, 4, request.request_id, result);
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
            4
        );
        let wait_id = session.operations.active_input().unwrap().wait.wait_id;
        let messages = respond(
            &mut session,
            5,
            request.request_id,
            chunk(b"0\n0\nx\n", 0, false, "v1"),
        );
        assert!(messages.iter().any(|message| matches!(message, RuntimeMessage::CommandRejected(rejected) if rejected.code == CommandErrorCode::StaleRequest)));
        assert_eq!(
            session.operations.active_input().unwrap().wait.wait_id,
            wait_id
        );
    }
    let (mut session, request) = start_check(true, "CHKDATA 0");
    let messages = respond(
        &mut session,
        4,
        request.request_id,
        chunk(b"0\n0\n", 0, false, "v1"),
    );
    let next = messages
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    respond(
        &mut session,
        5,
        next.request_id,
        chunk(b"x\n", 4, false, "v2"),
    );
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        4
    );
}

fn assert_save_page_before_input(messages: &[RuntimeMessage]) {
    let presentation_index = messages
        .iter()
        .position(|message| {
            matches!(
                message,
                RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
            )
        })
        .expect("complete page publication");
    let wait_index = messages
        .iter()
        .position(|message| matches!(message, RuntimeMessage::WaitChanged(WaitChange::Opened(_))))
        .expect("input wait publication");
    assert!(presentation_index < wait_index);
    let lines = match &messages[presentation_index] {
        RuntimeMessage::PresentationSnapshot(snapshot) => snapshot
            .history
            .logical_lines
            .iter()
            .map(projected_line_text)
            .collect::<Vec<_>>(),
        RuntimeMessage::PresentationDelta(delta) => delta
            .operations
            .iter()
            .filter_map(|operation| match operation {
                PresentationOperation::AppendLine { line } => Some(projected_line_text(line)),
                _ => None,
            })
            .collect::<Vec<_>>(),
        _ => unreachable!(),
    };
    assert_eq!(
        lines,
        (0..41)
            .map(|slot| format!("[{slot}] - ----"))
            .collect::<Vec<_>>()
    );
}
