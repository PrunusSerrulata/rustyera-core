use super::*;

fn assert_outbound_journal_byte_invariant(session: &RuntimeSession) {
    let encoded_bytes = session
        .outbound_journal
        .values()
        .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
        .sum::<u64>();
    assert_eq!(session.outbound_journal_bytes, encoded_bytes);
}

#[test]
fn chkdata_returns_its_status_and_updates_the_description() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "check-save-test".into(),
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
                relative_path: "check.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nCHKDATA 99\nWAIT\nRETURN\n".into()),
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
    let request = (0..8)
        .find_map(|_| {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            drain(&mut session)
                .into_iter()
                .find_map(|message| match message {
                    RuntimeMessage::StorageRequest(request) => Some(request),
                    _ => None,
                })
        })
        .expect("CHKDATA storage request");
    assert_eq!(request.relative_path, "save99.sav");
    assert_eq!(request.operation, StorageOperation::Read);

    submit(
        &mut session,
        3,
        RuntimeMessage::StorageResponse(StorageResponse {
            request_id: request.request_id,
            result: StorageResult::Error {
                error: era_runtime_protocol::FrontendIoError {
                    kind: FrontendIoErrorKind::NotFound,
                    message: "missing slot".into(),
                    platform_code: None,
                },
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
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 1);
    assert_eq!(read_runtime_string(vm, "RESULTS").unwrap(), "missing slot");
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
                    relative_path: "candidate.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nWAIT\nRETURN\n@SAVEINFO\nIF RESULT:2\nWAIT\nENDIF\nRESULT = 99\nRESULT:1 = GETCONFIG(\"Font size\")\nRESULTS:1 = %BARSTR(2, 4, 4)%\nPUTFORM %TOSTR(12345, \"0克尔\")%\nRETURN\n"
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
    assert_outbound_journal_byte_invariant(&session);

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
    assert_outbound_journal_byte_invariant(&session);
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
    assert_outbound_journal_byte_invariant(&session);
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
    assert_outbound_journal_byte_invariant(&session);
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
    assert_eq!(decoded.description, "2026/07/17 12:34:56 12345克尔");
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
        0
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
    assert_outbound_journal_byte_invariant(&session);

    let mut live = session.vm.take().unwrap();
    write_runtime_integer(&mut live, "RESULT", &[2], None, 1).unwrap();
    session
        .begin_candidate_save(&mut live, 98, CandidateSaveContinuation::Autosave)
        .unwrap();
    session.vm = Some(live);
    let stat_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) => Some(request),
            _ => None,
        })
        .unwrap();
    session
        .complete_storage(
            0,
            StorageResponse {
                request_id: stat_request.request_id,
                result: StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                    byte_length: 0,
                    revision: None,
                }),
            },
        )
        .unwrap();
    assert_outbound_journal_byte_invariant(&session);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.candidate_save_failed"
    )));
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
        configuration_profile: None,
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
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    assert_eq!(session.outbound_journal.len(), 2);
    assert!(session.outbound_journal_bytes > 0);
    assert_outbound_journal_byte_invariant(&session);

    let ack = RuntimeMessage::Acknowledge(era_runtime_protocol::SequenceAcknowledgement {
        through_sequence: 0,
    });
    submit(&mut session, 1, ack);
    session.drive(RuntimeDriveBudget::default()).expect("ack");
    assert_eq!(session.outbound_journal.len(), 1);
    assert_outbound_journal_byte_invariant(&session);
    let remaining = session.outbound_journal_bytes;
    session
        .emit_log(RuntimeLogLevel::Info, "reuses acknowledged budget")
        .unwrap();
    assert!(session.outbound_journal_bytes > remaining);
    assert_outbound_journal_byte_invariant(&session);

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
fn outbound_journal_rejects_an_encoded_message_over_its_byte_budget() {
    let mut options = RuntimeOptions::default();
    options.limits.maximum_journal_bytes = 1;
    let mut session = RuntimeSession::new(options);

    assert!(matches!(
        session.emit_log(RuntimeLogLevel::Info, "larger than one byte"),
        Err(RuntimeError::ResourceLimit(
            "outbound journal byte budget is exhausted"
        ))
    ));
    assert!(session.outbound.is_empty());
    assert!(session.outbound_journal.is_empty());
    assert_eq!(session.outbound_journal_bytes, 0);
}

#[test]
fn outbound_journal_enforces_cumulative_encoded_bytes_at_the_exact_boundary() {
    let mut probe = RuntimeSession::new(RuntimeOptions::default());
    probe.emit_log(RuntimeLogLevel::Info, "first").unwrap();
    probe.emit_log(RuntimeLogLevel::Info, "second").unwrap();
    let exact = probe.outbound_journal_bytes;
    assert_outbound_journal_byte_invariant(&probe);

    let mut options = RuntimeOptions::default();
    options.limits.maximum_journal_bytes = exact;
    let mut session = RuntimeSession::new(options);
    session.emit_log(RuntimeLogLevel::Info, "first").unwrap();
    session.emit_log(RuntimeLogLevel::Info, "second").unwrap();
    assert_eq!(session.outbound_journal_bytes, exact);
    assert_outbound_journal_byte_invariant(&session);
    assert!(matches!(
        session.emit_log(RuntimeLogLevel::Info, "third"),
        Err(RuntimeError::ResourceLimit(
            "outbound journal byte budget is exhausted"
        ))
    ));
    assert_eq!(session.outbound_journal_bytes, exact);
    assert_outbound_journal_byte_invariant(&session);
    session.state = SessionState::Active;
    session
        .handle_message(
            0,
            RuntimeMessage::Acknowledge(era_runtime_protocol::SequenceAcknowledgement {
                through_sequence: 0,
            }),
        )
        .unwrap();
    assert!(session.outbound_journal_bytes < exact);
    session.emit_log(RuntimeLogLevel::Info, "third").unwrap();
    assert_eq!(session.outbound_journal_bytes, exact);
    assert_outbound_journal_byte_invariant(&session);
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
    assert!(codes.contains(&"runtime.legacy_configuration_migration"));
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
    assert_eq!(format_era_integer(12_345, "#####"), Ok("12345".into()));
    assert_eq!(format_era_integer(-7, "##"), Ok("-7".into()));
    assert_eq!(format_era_integer(0, "##"), Ok(String::new()));
    assert_eq!(format_era_integer(12_345, "###,###"), Ok("12,345".into()));
    assert_eq!(format_era_integer(12_345, "$#,###"), Ok("$12,345".into()));
    assert_eq!(format_era_integer(-12_345, "$#,###"), Ok("-$12,345".into()));
    assert_eq!(format_era_integer(-7, "D3"), Ok("-007".into()));
    assert_eq!(format_era_integer(255, "X4"), Ok("00FF".into()));
    assert_eq!(format_era_integer(12_345, "0克尔"), Ok("12345克尔".into()));
    assert_eq!(format_era_integer(-7, "0克尔"), Ok("-7克尔".into()));
    assert_eq!(format_era_integer(0, "0克尔"), Ok("0克尔".into()));
}

#[test]
fn tostr_custom_sections_select_positive_negative_and_zero_formats() {
    assert_eq!(format_era_integer(12, "+#0;-#0"), Ok("+12".into()));
    assert_eq!(format_era_integer(-12, "+#0;-#0"), Ok("-12".into()));
    assert_eq!(format_era_integer(0, "+#0;-#0"), Ok("+0".into()));
    assert_eq!(format_era_integer(12, "P#0;N#0;Z0"), Ok("P12".into()));
    assert_eq!(format_era_integer(-12, "P#0;N#0;Z0"), Ok("N12".into()));
    assert_eq!(format_era_integer(0, "P#0;N#0;Z0"), Ok("Z0".into()));
    assert_eq!(
        format_era_integer(0, "0;0;0;0"),
        Err("unsupported integer format")
    );
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
    assert_eq!(named_color("LightSalmon"), Some(0x00ff_a07a));
    assert_eq!(named_color("transparent"), None);
}
