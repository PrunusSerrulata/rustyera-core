#[test]
fn cleanup_emission_fault_retains_the_exact_provider_handle_for_retry() {
    let mut fixture = SqlHostFixture::new("", Vec::new());
    fixture.session.options.limits.maximum_pending_requests = 0;
    let provider = fixture.session.sql.provider();
    let connection = SqlConnectionHandleV1 {
        service_epoch: provider.service_epoch,
        id: 77,
    };
    assert!(
        fixture
            .session
            .emit_sql_cleanup_for(provider, std::slice::from_ref(&connection))
            .is_err()
    );
    assert_eq!(
        fixture.session.sql_cleanup_queue,
        vec![PendingSqlCleanup {
            provider,
            connection,
        }]
    );
}

fn next_snake_project(revision: u64) -> ProjectManifest {
    let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
    ProjectManifest {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
        project_revision: revision,
        files: vec![
            profile_configuration_file(profile),
            SubmittedFile {
                relative_path: "next.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
        ],
    }
}

fn open_two_connections(tail: &str) -> (SqlHostFixture, Vec<(String, SqlConnectionHandleV1)>) {
    let mut fixture = SqlHostFixture::new(
        &format!("RESULT:0 = SQL_CONNECT(\"beta\")\nRESULT:1 = SQL_CONNECT(\"alpha\")\n{tail}"),
        Vec::new(),
    );
    let first = fixture.answer_memory_open(Some(revision(1)));
    let second = fixture.answer_memory_open(Some(revision(2)));
    assert_eq!(first.0, "beta");
    assert_eq!(second.0, "alpha");
    (fixture, vec![first, second])
}

fn cleanup_connections(messages: &[RuntimeMessage]) -> Vec<(usize, SqlRequestV1)> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::Sql && request.operation == SQL_OPERATION =>
            {
                let payload: SqlRequestV1 =
                    decode_canonical(request.payload.as_slice()).expect("decode cleanup SQL");
                matches!(&payload.operation, SqlOperationV1::Disconnect { .. })
                    .then_some((index, payload))
            }
            _ => None,
        })
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn cold_switch_and_shutdown_emit_sorted_disconnects_before_their_terminal_messages() {
    let (mut cold, opened) = open_two_connections("THROW switch-ready");
    assert_eq!(
        cold.session.phase(),
        RuntimePhase::Faulted,
        "{:#?}",
        cold.messages
    );
    let old_provider = cold.session.sql.provider();
    let beta = opened[0].1;
    let alpha = opened[1].1;
    cold.messages.clear();
    cold.submit_message(RuntimeMessage::ProjectManifest(next_snake_project(2)));
    assert_eq!(
        cold.session.phase(),
        RuntimePhase::Ready,
        "{:#?}",
        cold.messages
    );
    let cleanup = cleanup_connections(&cold.messages);
    assert_eq!(cleanup.len(), 2, "{:#?}", cold.messages);
    assert_eq!(cleanup[0].1.provider, old_provider);
    assert_eq!(cleanup[1].1.provider, old_provider);
    assert!(matches!(
        &cleanup[0].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == alpha
    ));
    assert!(matches!(
        &cleanup[1].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == beta
    ));
    let report = cold
        .messages
        .iter()
        .position(|message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success))
        .expect("cold project load report");
    assert!(cleanup.iter().all(|(index, _)| *index < report));
    assert!(cold.session.sql.connections().next().is_none());
    assert_ne!(cold.session.sql.provider(), old_provider);

    let (mut shutdown, opened) = open_two_connections("");
    assert_eq!(shutdown.session.phase(), RuntimePhase::WaitingInput);
    let old_provider = shutdown.session.sql.provider();
    let beta = opened[0].1;
    let alpha = opened[1].1;
    shutdown.messages.clear();
    shutdown.submit_message(RuntimeMessage::ShutdownRequest(ShutdownRequest {
        graceful: true,
    }));
    assert_eq!(shutdown.session.phase(), RuntimePhase::Stopped);
    let cleanup = cleanup_connections(&shutdown.messages);
    assert_eq!(cleanup.len(), 2, "{:#?}", shutdown.messages);
    assert_eq!(cleanup[0].1.provider, old_provider);
    assert_eq!(cleanup[1].1.provider, old_provider);
    assert!(matches!(
        &cleanup[0].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == alpha
    ));
    assert!(matches!(
        &cleanup[1].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == beta
    ));
    let ready = shutdown
        .messages
        .iter()
        .position(|message| matches!(message, RuntimeMessage::ShutdownReady(_)))
        .expect("shutdown ready message");
    assert!(cleanup.iter().all(|(index, _)| *index < ready));
    let RuntimeMessage::ShutdownReady(ready_message) = &shutdown.messages[ready] else {
        unreachable!()
    };
    assert_eq!(ready_message.pending_operations_cancelled, 1);
    assert!(shutdown.session.sql.connections().next().is_none());
    assert_ne!(shutdown.session.sql.provider(), old_provider);
}
