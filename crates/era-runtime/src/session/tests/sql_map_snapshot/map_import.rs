#[test]
fn sql_map_import_uses_resource_digest_and_preserves_rows() {
    let xml = br#"<map><p><k>a&amp;b</k><v>head<b x="1">mid</b>tail</v></p><p><k>dup</k><v>first</v></p><p><k>dup</k><v><i>second</i></v></p><p><k>missing</k></p></map>"#.to_vec();
    let mut fixture = map_fixture(xml.clone());
    let (_, connection) = fixture.answer_memory_open(Some(revision(1)));
    let storage = fixture.take_storage_request();
    assert_eq!(storage.namespace, StorageNamespace::Resource);
    assert_eq!(storage.relative_path, "maps/test.xml");
    assert!(matches!(&storage.operation, StorageOperation::Read));
    fixture.respond_storage(&storage, storage_read(&xml));

    let (service, payload) = fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows {
        connection: imported_connection,
        table,
        rows,
    } = &payload.operation
    else {
        panic!("expected SQL ImportMapRows request")
    };
    assert_eq!(*imported_connection, connection);
    assert_eq!(table, "translations");
    assert_eq!(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("a&b", "head<b x=\"1\">mid</b>tail"),
            ("dup", "first"),
            ("dup", "<i>second</i>"),
        ]
    );
    answer_map_import(&mut fixture, &service, &payload, connection, 3);
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(fixture.integer(0), 0);
    assert_eq!(fixture.integer(1), 1);

    let mut mismatch = map_fixture(xml);
    mismatch.answer_memory_open(Some(revision(1)));
    let storage = mismatch.take_storage_request();
    mismatch.respond_storage(&storage, storage_read(b"<map></map>"));
    mismatch.assert_faulted();
    assert!(mismatch.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

#[test]
fn sql_map_import_rejects_invalid_utf8_and_oversize_resources() {
    let mut invalid_utf8 = map_fixture(vec![0xff]);
    invalid_utf8.answer_memory_open(Some(revision(1)));
    let storage = invalid_utf8.take_storage_request();
    invalid_utf8.respond_storage(&storage, storage_read(&[0xff]));
    invalid_utf8.assert_faulted();

    let maximum_bytes =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_bytes).expect("MAP byte limit fits usize");
    let oversize = vec![b' '; maximum_bytes + 1];
    let mut oversized = map_fixture(oversize.clone());
    oversized.answer_memory_open(Some(revision(1)));
    let storage = oversized.take_storage_request();
    oversized.respond_storage(&storage, storage_read(&oversize));
    oversized.assert_faulted();
    assert!(oversized.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

#[test]
#[allow(clippy::too_many_lines)]
fn sql_map_import_accepts_exact_size_and_row_limits_but_rejects_the_next_row() {
    let maximum_bytes =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_bytes).expect("MAP byte limit fits usize");
    let mut exact_bytes = b"<map>".to_vec();
    exact_bytes.resize(maximum_bytes - b"</map>".len(), b' ');
    exact_bytes.extend_from_slice(b"</map>");
    assert_eq!(exact_bytes.len(), maximum_bytes);
    let mut bytes_fixture = map_fixture(exact_bytes.clone());
    let (_, connection) = bytes_fixture.answer_memory_open(Some(revision(1)));
    let storage = bytes_fixture.take_storage_request();
    bytes_fixture.respond_storage(&storage, storage_read(&exact_bytes));
    let (service, payload) = bytes_fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows { rows, .. } = &payload.operation else {
        panic!("expected exact-size MAP import")
    };
    assert!(rows.is_empty());
    answer_map_import(&mut bytes_fixture, &service, &payload, connection, 0);
    assert_eq!(bytes_fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(bytes_fixture.integer(1), 1);

    let row = "<p><k></k><v></v></p>";
    let maximum_rows =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_rows).expect("MAP row limit fits usize");
    let exact_rows = format!("<map>{}</map>", row.repeat(maximum_rows)).into_bytes();
    assert!(exact_rows.len() < maximum_bytes);
    let mut rows_fixture = map_fixture(exact_rows.clone());
    let (_, connection) = rows_fixture.answer_memory_open(Some(revision(1)));
    let storage = rows_fixture.take_storage_request();
    rows_fixture.respond_storage(&storage, storage_read(&exact_rows));
    let (service, payload) = rows_fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows { rows, .. } = &payload.operation else {
        panic!("expected exact-row MAP import")
    };
    assert_eq!(rows.len(), maximum_rows);
    answer_map_import(
        &mut rows_fixture,
        &service,
        &payload,
        connection,
        SqlLimitsV1::FIXED.maximum_map_rows,
    );
    assert_eq!(rows_fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(rows_fixture.integer(1), 1);

    let too_many_rows = format!("<map>{}</map>", row.repeat(maximum_rows + 1)).into_bytes();
    assert!(too_many_rows.len() < maximum_bytes);
    let mut rejected = map_fixture(too_many_rows.clone());
    rejected.answer_memory_open(Some(revision(1)));
    let storage = rejected.take_storage_request();
    rejected.respond_storage(&storage, storage_read(&too_many_rows));
    rejected.assert_faulted();
    assert!(rejected.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

fn snapshot_reasons(
    fixture: &mut SqlHostFixture,
    message_id: u64,
) -> Vec<SnapshotIneligibleReason> {
    fixture
        .session
        .export_state(
            message_id,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .expect("request SQL snapshot eligibility");
    let messages = drain(&mut fixture.session);
    let results = messages
        .into_iter()
        .filter_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: StateExportKind::VmSnapshot,
                result: StateExportResult::Ineligible { reasons },
            }) => Some(reasons),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    results
        .into_iter()
        .next()
        .expect("snapshot ineligible result")
}

fn export_traditional_save_bytes(fixture: &mut SqlHostFixture, message_id: u64) -> Vec<u8> {
    fixture
        .session
        .export_state(
            message_id,
            StateExportRequest {
                kind: StateExportKind::TraditionalSave,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .expect("request traditional save export");
    let messages = drain(&mut fixture.session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: StateExportKind::TraditionalSave,
                result: StateExportResult::Ready { .. },
            })
        )),
        "{messages:#?}"
    );
    let bytes = fixture
        .session
        .outbound_transfer
        .take()
        .expect("traditional save transfer bytes")
        .bytes;
    bytes.copy_range(0..bytes.len())
}
