use super::*;
use crate::NativeSqlProvider;
use crate::revision::tests::{Ack, MemoryIo, old_seed};
use era_runtime_protocol::{
    FrontendIoError, FrontendIoErrorKind, SqlDatabaseIdentityV1, SqlDatabaseSourceV1,
    SqlOpenRevisionV1, SqlReaderCellV1, SqlReaderValueModeV1, SqlResourceSeedV1, StorageOperation,
    StorageRequest, StorageResponse, StorageResult,
};

const CONNECTION: SqlConnectionHandleV1 = SqlConnectionHandleV1 {
    service_epoch: 1,
    id: 1,
};

fn engine() -> Engine {
    let mut engine = Engine::default();
    engine
        .register(
            SqlProviderHandleV1 {
                service_epoch: 1,
                id: 1,
            },
            crate::ProviderRole::Live,
        )
        .unwrap();
    engine
}

fn request(operation: SqlOperationV1) -> SqlRequestV1 {
    SqlRequestV1 {
        provider: SqlProviderHandleV1 {
            service_epoch: 1,
            id: 1,
        },
        operation,
    }
}

fn open(source: SqlDatabaseSourceV1, revision: SqlOpenRevisionV1) -> SqlOperationV1 {
    SqlOperationV1::Open {
        connection: CONNECTION,
        logical_name: "test".into(),
        identity: SqlDatabaseIdentityV1 {
            source,
            sqlite_version: SQL_SQLITE_VERSION.into(),
            format_version: SQL_DATABASE_FORMAT_VERSION,
        },
        revision,
        limits: SqlLimitsV1::FIXED,
    }
}

fn execute(sql: &str, mode: SqlExecuteModeV1) -> SqlOperationV1 {
    SqlOperationV1::Execute {
        connection: CONNECTION,
        sql: sql.into(),
        mode,
        parameters: Vec::new(),
    }
}

fn call(engine: &mut Engine, io: &mut MemoryIo, operation: SqlOperationV1) -> SqlResponseV1 {
    engine.handle(&request(operation), 2, io)
}

fn assert_success(response: &SqlResponseV1) {
    assert!(
        !matches!(response.result, SqlResultV1::Error { .. }),
        "{response:?}"
    );
}

fn integer(response: SqlResponseV1) -> i64 {
    match response.result {
        SqlResultV1::Scalar {
            value: SqlValueV1::Integer(value),
        }
        | SqlResultV1::ReusableScalar {
            value: SqlValueV1::Integer(value),
        } => value,
        other => panic!("expected integer, got {other:?}"),
    }
}

#[test]
fn transaction_rollback_and_exact_restore_preserve_durable_bytes() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    let committed = call(
        &mut engine,
        &mut io,
        execute(
            "CREATE TABLE t(x); INSERT INTO t VALUES(7)",
            SqlExecuteModeV1::NonQuery,
        ),
    );
    assert_success(&committed);
    let revision = committed.database.unwrap().durable_revision.unwrap();
    let pending = call(
        &mut engine,
        &mut io,
        execute("BEGIN; UPDATE t SET x=9", SqlExecuteModeV1::NonQuery),
    );
    assert_success(&pending);
    let state = pending.database.unwrap();
    assert!(state.transaction_active);
    assert_eq!(state.durable_revision, Some(revision.clone()));
    assert_success(&call(
        &mut engine,
        &mut io,
        execute("ROLLBACK", SqlExecuteModeV1::NonQuery),
    ));
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute("SELECT x FROM t", SqlExecuteModeV1::ScalarInteger)
        )),
        7
    );
    assert_success(&call(
        &mut engine,
        &mut io,
        SqlOperationV1::Disconnect {
            connection: CONNECTION,
        },
    ));
    assert_success(&call(
        &mut engine,
        &mut io,
        open(
            SqlDatabaseSourceV1::Memory,
            SqlOpenRevisionV1::Exact(revision),
        ),
    ));
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute("SELECT x FROM t", SqlExecuteModeV1::ScalarInteger)
        )),
        7
    );
}

#[test]
fn reader_is_lazy_and_projection_matches_ordinary_getters() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    let response = call(
        &mut engine,
        &mut io,
        execute(
            "SELECT NULL, '12tail', 3 UNION ALL SELECT NULL, '4', 5",
            SqlExecuteModeV1::Reader,
        ),
    );
    let SqlResultV1::ReaderOpened { reader } = response.result else {
        panic!("{response:?}");
    };
    assert_eq!(
        response.reader.unwrap().status,
        SqlReaderStatusV1::BeforeFirst
    );
    let response = call(&mut engine, &mut io, SqlOperationV1::ReaderRead { reader });
    assert_eq!(response.reader.unwrap().rows_read, 1);
    let SqlResultV1::ReaderRow { cells } = response.result else {
        panic!("{response:?}");
    };
    assert_eq!(
        cells[0],
        SqlReaderCellV1 {
            integer: Some(0),
            string: Some(String::new()),
            is_null: Some(true)
        }
    );
    assert_eq!(cells[1].integer, Some(12));
    assert_eq!(cells[1].string.as_deref(), Some("12tail"));
    let response = call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderGet {
            reader,
            column: 1,
            mode: SqlReaderValueModeV1::Integer,
        },
    );
    assert!(matches!(
        response.result,
        SqlResultV1::ReaderValue {
            value: SqlValueV1::Integer(12)
        }
    ));
    assert_success(&call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderClose { reader },
    ));
    let response = call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderGet {
            reader,
            column: 1,
            mode: SqlReaderValueModeV1::Integer,
        },
    );
    assert!(matches!(
        response.result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::ReaderNotFound,
                ..
            }
        }
    ));
}

#[test]
fn historical_seed_is_readable_on_exact_engine() {
    let bytes = old_seed();
    let source = SqlDatabaseSourceV1::ResourceSeed(SqlResourceSeedV1 {
        resource_id: "seed.db".into(),
        sha256: ProtocolBytes::new(Sha256::digest(&bytes).to_vec()),
    });
    let mut io = MemoryIo::seed(bytes);
    let mut engine = engine();
    let response = call(
        &mut engine,
        &mut io,
        open(source, SqlOpenRevisionV1::Current),
    );
    assert_success(&response);
    assert!(
        matches!(response.result, SqlResultV1::Opened { sqlite_version, .. } if sqlite_version == SQL_SQLITE_VERSION)
    );
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute(
                "SELECT version FROM seed_marker",
                SqlExecuteModeV1::ScalarInteger
            )
        )),
        1
    );
}

#[test]
fn actor_owns_connections_and_reset_discards_handles() {
    let mut owner = NativeSqlProvider::new().unwrap();
    owner
        .register(
            SqlProviderHandleV1 {
                service_epoch: 1,
                id: 1,
            },
            crate::ProviderRole::Live,
        )
        .unwrap();
    let mut io = MemoryIo::default();
    assert_success(
        &owner
            .handle(
                request(open(
                    SqlDatabaseSourceV1::Memory,
                    SqlOpenRevisionV1::Current,
                )),
                2,
                &mut io,
            )
            .unwrap(),
    );
    assert_eq!(
        integer(
            owner
                .handle(
                    request(execute("SELECT 42", SqlExecuteModeV1::ScalarInteger)),
                    2,
                    &mut io
                )
                .unwrap()
        ),
        42
    );
    owner.reset().unwrap();
    let response = owner
        .handle(
            request(execute("SELECT 42", SqlExecuteModeV1::ScalarInteger)),
            2,
            &mut io,
        )
        .unwrap();
    assert!(matches!(
        response.result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::StaleEpoch,
                ..
            }
        }
    ));
}

#[test]
fn missing_handles_preserve_response_contract() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    let response = call(
        &mut engine,
        &mut io,
        SqlOperationV1::Disconnect {
            connection: CONNECTION,
        },
    );
    assert_eq!(
        response.database.unwrap(),
        SqlDatabaseStateV1 {
            connection: CONNECTION,
            connected: false,
            transaction_active: false,
            durable_revision: None
        }
    );
    let response = call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderGet {
            reader: SqlReaderHandleV1 {
                service_epoch: 1,
                id: 999,
            },
            column: 0,
            mode: SqlReaderValueModeV1::Integer,
        },
    );
    assert!(matches!(
        response.result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::ReaderNotFound,
                ..
            }
        }
    ));
}

#[test]
fn nonquery_binds_first_parameterized_statement_and_steps_once() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    let operation = SqlOperationV1::Execute {
        connection: CONNECTION,
        sql: "CREATE TABLE t(x); INSERT INTO t VALUES(@0); INSERT INTO t VALUES(@0)".into(),
        mode: SqlExecuteModeV1::NonQuery,
        parameters: vec![SqlValueV1::Integer(7)],
    };
    assert_success(&call(&mut engine, &mut io, operation));
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute(
                "SELECT count(*) FROM t WHERE x=7",
                SqlExecuteModeV1::ScalarInteger
            )
        )),
        1
    );
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute(
                "SELECT count(*) FROM t WHERE x IS NULL",
                SqlExecuteModeV1::ScalarInteger
            )
        )),
        1
    );
    assert_success(&call(
        &mut engine,
        &mut io,
        execute(
            "SELECT 1 UNION ALL SELECT abs(-9223372036854775808)",
            SqlExecuteModeV1::NonQuery,
        ),
    ));
    assert_success(&call(
        &mut engine,
        &mut io,
        execute(
            "INSERT INTO t VALUES(8),(9) RETURNING x",
            SqlExecuteModeV1::NonQuery,
        ),
    ));
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute("SELECT count(*) FROM t", SqlExecuteModeV1::ScalarInteger)
        )),
        4
    );
}

#[test]
fn closed_write_reader_publishes_once_and_does_not_close_other_reader() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    assert_success(&call(
        &mut engine,
        &mut io,
        execute("CREATE TABLE t(x)", SqlExecuteModeV1::NonQuery),
    ));
    let SqlResultV1::ReaderOpened { reader: other } = call(
        &mut engine,
        &mut io,
        execute("SELECT 7 UNION ALL SELECT 8", SqlExecuteModeV1::Reader),
    )
    .result
    else {
        panic!("reader");
    };
    let SqlResultV1::ReaderOpened { reader } = call(
        &mut engine,
        &mut io,
        execute(
            "INSERT INTO t VALUES(1),(2) RETURNING x",
            SqlExecuteModeV1::Reader,
        ),
    )
    .result
    else {
        panic!("reader");
    };
    assert_success(&call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderRead { reader },
    ));
    let response = call(&mut engine, &mut io, SqlOperationV1::ReaderClose { reader });
    assert_success(&response);
    assert_eq!(response.reader.unwrap().status, SqlReaderStatusV1::Closed);
    let files = io.published_files().clone();
    assert!(matches!(
        call(&mut engine, &mut io, SqlOperationV1::ReaderRead { reader }).result,
        SqlResultV1::ReaderAdvanced { has_row: false }
    ));
    for operation in [
        SqlOperationV1::ReaderGet {
            reader,
            column: 0,
            mode: SqlReaderValueModeV1::Integer,
        },
        SqlOperationV1::ReaderIsNull { reader, column: 0 },
    ] {
        assert!(matches!(
            call(&mut engine, &mut io, operation).result,
            SqlResultV1::Error {
                error: SqlErrorV1 {
                    code: SqlErrorCodeV1::ReaderNotFound,
                    ..
                }
            }
        ));
    }
    let closed = call(&mut engine, &mut io, SqlOperationV1::ReaderClose { reader });
    assert_success(&closed);
    assert!(closed.reader.is_none());
    assert!(closed.database.is_none());
    assert_eq!(io.published_files(), &files);
    assert_success(&call(
        &mut engine,
        &mut io,
        SqlOperationV1::ReaderRead { reader: other },
    ));
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute("SELECT count(*) FROM t", SqlExecuteModeV1::ScalarInteger)
        )),
        2
    );
}

#[test]
fn map_rejects_oversized_cell_before_mutation() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    let files = io.published_files().clone();
    let response = call(
        &mut engine,
        &mut io,
        SqlOperationV1::ImportMapRows {
            connection: CONNECTION,
            table: "t".into(),
            rows: vec![SqlMapRowV1 {
                key: "k".into(),
                value: "x".repeat(1024 * 1024 + 1),
            }],
        },
    );
    assert!(matches!(
        response.result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::CellTooLarge,
                ..
            }
        }
    ));
    assert_eq!(io.published_files(), &files);
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute(
                "SELECT count(*) FROM sqlite_schema WHERE name='t'",
                SqlExecuteModeV1::ScalarInteger
            )
        )),
        0
    );
}

#[test]
fn provider_registration_is_bounded_and_candidate_retirement_preserves_live() {
    let mut engine = engine();
    let mut io = MemoryIo::default();
    assert_success(&call(
        &mut engine,
        &mut io,
        open(SqlDatabaseSourceV1::Memory, SqlOpenRevisionV1::Current),
    ));
    let candidate = SqlProviderHandleV1 {
        service_epoch: 2,
        id: 1,
    };
    engine
        .register(candidate, crate::ProviderRole::Candidate)
        .unwrap();
    assert!(
        engine
            .register(
                SqlProviderHandleV1 {
                    service_epoch: 3,
                    id: 1
                },
                crate::ProviderRole::Candidate
            )
            .is_err()
    );
    engine.retire(candidate).unwrap();
    assert!(
        engine
            .register(candidate, crate::ProviderRole::Candidate)
            .is_err()
    );
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute("SELECT 5", SqlExecuteModeV1::ScalarInteger)
        )),
        5
    );
    let candidate = SqlProviderHandleV1 {
        service_epoch: 3,
        id: 1,
    };
    engine
        .register(candidate, crate::ProviderRole::Candidate)
        .unwrap();
    engine.promote_candidate(candidate).unwrap();
    assert!(matches!(
        call(
            &mut engine,
            &mut io,
            execute("SELECT 5", SqlExecuteModeV1::ScalarInteger)
        )
        .result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::StaleEpoch,
                ..
            }
        }
    ));
    engine.reset();
    assert!(
        engine
            .register(candidate, crate::ProviderRole::Live)
            .is_err()
    );
    assert!(engine.providers.is_empty());
}

fn seeded_engine() -> (Engine, MemoryIo) {
    let bytes = old_seed();
    let source = SqlDatabaseSourceV1::ResourceSeed(SqlResourceSeedV1 {
        resource_id: "seed.db".into(),
        sha256: ProtocolBytes::new(Sha256::digest(&bytes).to_vec()),
    });
    let mut engine = engine();
    let mut io = MemoryIo::seed(bytes);
    assert_success(&call(
        &mut engine,
        &mut io,
        open(source, SqlOpenRevisionV1::Current),
    ));
    (engine, io)
}

#[test]
fn rejected_publication_restores_live_database_but_committed_unresolved_closes_it() {
    let (mut engine, mut io) = seeded_engine();
    io.set_ack(Ack::Rejected);
    let response = call(
        &mut engine,
        &mut io,
        execute(
            "UPDATE seed_marker SET version=10",
            SqlExecuteModeV1::NonQuery,
        ),
    );
    assert!(matches!(response.result, SqlResultV1::Error { .. }));
    assert!(response.database.unwrap().connected);
    assert_eq!(
        integer(call(
            &mut engine,
            &mut io,
            execute(
                "SELECT version FROM seed_marker",
                SqlExecuteModeV1::ScalarInteger
            )
        )),
        1
    );
    io.set_ack(Ack::OmittedUnreadable);
    let response = call(
        &mut engine,
        &mut io,
        execute(
            "UPDATE seed_marker SET version=11",
            SqlExecuteModeV1::NonQuery,
        ),
    );
    assert!(!response.database.as_ref().unwrap().connected);
    let SqlResultV1::Error { error } = response.result else {
        panic!("publication must fail closed");
    };
    assert!(
        error
            .context
            .iter()
            .any(|field| field.key == "commit_outcome" && field.value == "committed")
    );
    assert!(
        engine
            .providers
            .get(&(1, 1))
            .unwrap()
            .connections
            .is_empty()
    );
    assert!(matches!(
        call(
            &mut engine,
            &mut io,
            execute("SELECT 1", SqlExecuteModeV1::ScalarInteger)
        )
        .result,
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::ConnectionNotFound,
                ..
            }
        }
    ));
}

#[test]
fn unknown_publication_closes_connection_and_all_readers_without_restoring_old_bytes() {
    let (mut engine, mut io) = seeded_engine();
    let SqlResultV1::ReaderOpened { reader } = call(
        &mut engine,
        &mut io,
        execute("SELECT 1", SqlExecuteModeV1::Reader),
    )
    .result
    else {
        panic!("reader");
    };
    io.set_ack(Ack::Lost);
    let mut writing = false;
    let mut host = |request: StorageRequest| {
        if request.relative_path.ends_with("/current") {
            if matches!(request.operation, StorageOperation::Write { .. }) {
                writing = true;
            } else if writing && matches!(request.operation, StorageOperation::Read) {
                return StorageResponse {
                    request_id: request.request_id,
                    result: StorageResult::Error {
                        error: FrontendIoError {
                            kind: FrontendIoErrorKind::Interrupted,
                            message: "unknown readback".into(),
                            platform_code: None,
                        },
                    },
                };
            }
        }
        io.handle(request)
    };
    let response = engine.handle(
        &request(execute(
            "UPDATE seed_marker SET version=12",
            SqlExecuteModeV1::NonQuery,
        )),
        2,
        &mut host,
    );
    assert!(!response.database.unwrap().connected);
    let SqlResultV1::Error { error } = response.result else {
        panic!("publication must fail closed");
    };
    assert!(
        error
            .context
            .iter()
            .any(|field| field.key == "commit_outcome" && field.value == "unknown")
    );
    assert!(engine.providers.get(&(1, 1)).unwrap().readers.is_empty());
    assert!(matches!(
        call(&mut engine, &mut io, SqlOperationV1::ReaderRead { reader }).result,
        SqlResultV1::ReaderAdvanced { has_row: false }
    ));
}
