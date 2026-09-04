#[test]
fn structured_sql_error_applies_authoritative_state_before_vm_fault() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        SQL_EXECUTE_NONQUERY \"db\", \"BEGIN broken\"\n\
        WAIT\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    let execute = take_sql_request(messages);
    let response = execute_response(
        &execute,
        true,
        revision(2),
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::Sqlite,
                operation: SqlOperationKindV1::Execute,
                context: vec![SqlErrorContextV1 {
                    key: "statement".into(),
                    value: "begin".into(),
                }],
                sqlite_code: Some(1),
                sqlite_message: Some("diagnostic text is not classification".into()),
            },
        },
    );
    let messages = harness.respond(&execute, response);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    let connection = harness
        .session
        .sql
        .connection_by_key("db")
        .expect("SQL error keeps the authoritative connection");
    assert!(connection.transaction_active);
    assert_eq!(connection.durable_revision.as_ref(), Some(&revision(2)));
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn storage_failure_keeps_the_last_provider_revision_and_faults_the_vm() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        SQL_EXECUTE_NONQUERY \"db\", \"UPDATE durable\"\n\
        WAIT\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    let execute = take_sql_request(messages);
    let response = execute_response(
        &execute,
        false,
        revision(1),
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::StorageFailure,
                operation: SqlOperationKindV1::Execute,
                context: vec![SqlErrorContextV1 {
                    key: "stage".into(),
                    value: "persist".into(),
                }],
                sqlite_code: None,
                sqlite_message: Some("provider-local diagnostic".into()),
            },
        },
    );
    let messages = harness.respond(&execute, response);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault { code: FaultCode::VmFault, message, .. })
            if message.contains("rustyera.sql/execute/storage_failure")
                && !message.contains("provider-local diagnostic")
    )));
    let connection = harness
        .session
        .sql
        .connection_by_key("db")
        .expect("storage failure retains the authoritative connection");
    assert_eq!(connection.durable_revision.as_ref(), Some(&revision(1)));
    assert!(!connection.transaction_active);
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn concurrent_same_name_open_has_one_reservation_and_one_provider_request() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"setup\"\n\
        IF 0\n\
        CALL OPEN_FIRST\n\
        CALL OPEN_SECOND\n\
        ENDIF\n\
        WAIT\n\
        RETURN\n\
        @OPEN_FIRST\n\
        SQL_CONNECT \"race\"\n\
        RETURN\n\
        @OPEN_SECOND\n\
        SQL_CONNECT \"RACE\"\n\
        RETURN\n";
    let (mut harness, setup_open) = SqlHarness::start(source);
    let messages = harness.respond(&setup_open, open_response(&setup_open, revision(1)));
    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

    drop(spawn_entries(&mut harness, &["OPEN_FIRST", "OPEN_SECOND"]));
    let messages = harness.drive_to_boundary();
    let requests = take_sql_requests(messages);
    assert_eq!(
        requests.len(),
        1,
        "only the reserved Open reaches the provider"
    );
    assert!(matches!(
        &requests[0].payload.operation,
        SqlOperationV1::Open { logical_name, .. } if logical_name == "race"
    ));
    assert!(harness.session.sql.has_inflight());
    assert!(harness.session.sql.connection_by_key("race").is_none());

    let messages = harness.respond(&requests[0], open_response(&requests[0], revision(2)));
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert!(harness.session.sql.connection_by_key("race").is_some());
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn disconnect_during_execute_is_rejected_without_reaching_the_provider() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        IF 0\n\
        CALL EXECUTE_PENDING\n\
        CALL DISCONNECT_PENDING\n\
        ENDIF\n\
        WAIT\n\
        RETURN\n\
        @EXECUTE_PENDING\n\
        SQL_EXECUTE_NONQUERY \"db\", \"UPDATE pending\"\n\
        RETURN\n\
        @DISCONNECT_PENDING\n\
        SQL_DISCONNECT \"db\"\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

    drop(spawn_entries(
        &mut harness,
        &["EXECUTE_PENDING", "DISCONNECT_PENDING"],
    ));
    let messages = harness.drive_to_boundary();
    let requests = take_sql_requests(messages);
    assert_eq!(
        requests.len(),
        1,
        "disconnect must remain local while Execute is pending"
    );
    assert!(matches!(
        &requests[0].payload.operation,
        SqlOperationV1::Execute { sql, .. } if sql == "UPDATE pending"
    ));
    assert!(harness.session.sql.has_inflight());
    assert!(harness.session.sql.connection_by_key("db").is_some());

    let messages = harness.respond(
        &requests[0],
        execute_response(
            &requests[0],
            false,
            revision(2),
            SqlResultV1::NonQuery { affected_rows: 1 },
        ),
    );
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert_no_sql_request(&messages);
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn stale_provider_response_is_rejected_and_the_real_request_can_retry() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nRESULT:0 = 1\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let mut stale = open_response(&open, revision(1));
    stale.provider.id = stale
        .provider
        .id
        .checked_add(1)
        .expect("provider id remains bounded");
    let messages = harness.respond(&open, stale);

    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::StaleRequest,
            ..
        })
    )));
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingExternal);
    assert_no_sql_request(&messages);

    let messages = harness.respond(&open, open_response(&open, revision(1)));
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::WaitingInput,
        "{messages:#?}"
    );
    assert_eq!(harness.integer(0), 1);
    assert!(harness.session.sql.connection_by_key("db").is_some());
}

#[test]
fn project_boundary_cancels_the_request_and_rejects_its_old_epoch_completion() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let old_epoch = open.payload.provider.service_epoch;
    let old_response = open_response(&open, revision(1));
    let shutdown = harness.submit_message(RuntimeMessage::ShutdownRequest(ShutdownRequest {
        graceful: true,
    }));
    assert!(shutdown.iter().any(|message| matches!(
        message,
        RuntimeMessage::CancelExternalRequest(cancel)
            if cancel.request_id == open.wire.request_id
    )));
    assert!(
        shutdown
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ShutdownReady(_)))
    );
    assert_eq!(harness.session.phase(), RuntimePhase::Stopped);
    assert_ne!(harness.session.sql.service_epoch(), old_epoch);

    let messages = harness.respond_to_request_id(open.wire.request_id, old_response);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::StaleRequest,
            ..
        })
    )));
    assert!(harness.session.sql.connection_by_key("db").is_none());
}

#[test]
fn snake_project_without_sql_capability_never_creates_a_vm() {
    let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
    let mut session = negotiated_session_without_sql();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::for_profile(profile),
            project_revision: 1,
            files: vec![
                profile_configuration_file(profile),
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nRESULT = SQL_CONNECT(\"db\")\nRETURN\n".into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("reject the snake fixture before compilation");
    let messages = drain(&mut session);

    assert!(session.vm.is_none());
    assert!(session.project_snapshot.is_none());
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report)
            if !report.success && report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "runtime.missing_sql_service"
            })
    )));
    assert_no_sql_request(&messages);
}

#[test]
fn ninth_connection_is_rejected_by_the_connection_and_opening_limit() {
    let mut source = String::from("@SYSTEM_TITLE\n");
    for index in 0..9 {
        source.push_str("RESULT:");
        source.push_str(&index.to_string());
        source.push_str(" = SQL_CONNECT(\"db");
        source.push_str(&index.to_string());
        source.push_str("\")\n");
    }
    source.push_str("WAIT\nRETURN\n");
    let (mut harness, mut open) = SqlHarness::start(&source);
    let mut final_messages = None;

    for index in 0..8_u8 {
        let expected = format!("db{index}");
        assert!(matches!(
            &open.payload.operation,
            SqlOperationV1::Open { logical_name, .. } if logical_name == &expected
        ));
        let messages = harness.respond(&open, open_response(&open, revision(index + 1)));
        if index < 7 {
            open = take_sql_request(messages);
        } else {
            final_messages = Some(messages);
        }
    }
    let final_messages = final_messages.expect("eighth completion resumes the ninth connect");

    assert_no_sql_request(&final_messages);
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{final_messages:#?}"
    );
    assert!(final_messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert_eq!(harness.session.sql.connections().count(), 8);
    assert!(!harness.session.sql.has_inflight());
    for index in 0..8 {
        assert_eq!(harness.integer(index), 0);
    }
}

#[test]
fn malformed_success_response_faults_without_committing_the_open() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let malformed = SqlResponseV1 {
        provider: open.payload.provider,
        database: None,
        reader: None,
        result: SqlResultV1::Opened {
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            limits: era_runtime_protocol::SqlLimitsV1::FIXED,
        },
    };
    let messages = harness.respond(&open, malformed);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            message,
            ..
        }) if message.contains("authoritative database state")
    )));
    assert!(harness.session.sql.connection_by_key("db").is_none());
    assert!(!harness.session.sql.has_inflight());
}
