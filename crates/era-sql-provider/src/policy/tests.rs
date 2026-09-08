use super::*;

fn authorize_action(action: AuthAction<'_>, state: &Mutex<OperationState>) -> Authorization {
    authorize(
        AuthContext {
            action,
            database_name: None,
            accessor: None,
        },
        state,
    )
}

fn installed() -> (Connection, Policy) {
    let connection = Connection::open_in_memory().unwrap();
    let policy = Policy::install(&connection, Control::default()).unwrap();
    (connection, policy)
}

fn execute(connection: &Connection, policy: &Policy, sql: &str) -> Result<()> {
    let _scope = policy.scope(sql)?;

    connection
        .execute_batch(sql)
        .map_err(|error| policy.error(error))
}

#[test]
fn engine_identity_requires_both_exact_values() {
    require_engine("3.53.4", 3_053_004).unwrap();
    for (version, number) in [
        ("3.53.0", 3_053_000),
        ("3.53.4", 3_053_002),
        ("3.53.2", 3_053_004),
    ] {
        assert_eq!(
            require_engine(version, number).unwrap_err().code,
            SqlErrorCodeV1::Unsupported
        );
    }
    let connection = Connection::open_in_memory().unwrap();
    let result = Policy::install(&connection, Control::default());
    if rusqlite::version() == "3.53.4" && rusqlite::version_number() == 3_053_004 {
        assert!(result.is_ok());
    } else {
        assert!(matches!(result, Err(error) if error.code == SqlErrorCodeV1::Unsupported));
    }
}

#[test]
fn limits_and_memory_configuration_are_fixed() {
    let (connection, _policy) = installed();
    assert_eq!(
        connection.limit(Limit::SQLITE_LIMIT_SQL_LENGTH).unwrap(),
        256 * 1024
    );
    assert_eq!(
        connection
            .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
            .unwrap(),
        64
    );
    assert_eq!(
        connection.limit(Limit::SQLITE_LIMIT_LENGTH).unwrap(),
        64 * 1024 * 1024
    );
    assert_eq!(
        connection
            .query_row("PRAGMA trusted_schema", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "memory"
    );
    let pages: u32 = connection
        .query_row("PRAGMA max_page_count", [], |row| row.get(0))
        .unwrap();
    let size: u32 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap();
    assert!(u64::from(pages) * u64::from(size) <= LIMITS.maximum_database_bytes);
    assert!(connection.prepare("SELECT ?65").is_err());
}

#[test]
fn malicious_sql_cannot_change_policy_or_attach_databases() {
    let (connection, policy) = installed();
    for sql in [
        "ATTACH ':memory:' AS extra",
        "ATTACH '' AS extra",
        "DETACH main",
        "VACUUM INTO ':memory:'",
        "VACUUM; ATTACH '' AS extra",
        "PRAGMA trusted_schema=ON",
        "PRAGMA writable_schema=ON",
        "PRAGMA temp_store=FILE",
        "PRAGMA journal_mode=WAL",
        "PRAGMA max_page_count=2147483647",
        "PRAGMA page_size=65536",
        "PRAGMA busy_timeout=600000",
        "PRAGMA mmap_size=2147483647",
        "PRAGMA temp_store_directory='.'",
        "PRAGMA data_store_directory='.'",
        "PRAGMA schema_version=99",
        "CREATE VIRTUAL TABLE evil USING fts5(value)",
        "SELECT load_extension('missing')",
        "SELECT readfile('missing')",
        "SELECT writefile('missing', 'payload')",
    ] {
        assert!(execute(&connection, &policy, sql).is_err(), "{sql}");
        assert!(!policy.state.lock().unwrap().allow_vacuum_attach);
    }
    execute(
        &connection,
        &policy,
        "CREATE TABLE safe(value); INSERT INTO safe VALUES(1)",
    )
    .unwrap();
}

#[test]
fn authorizer_denies_path_functions_even_if_registered() {
    let state = Mutex::new(OperationState::default());
    for function_name in [
        "load_extension",
        "READFILE",
        "writefile",
        "eval",
        "fts3_tokenizer",
    ] {
        assert_eq!(
            authorize_action(AuthAction::Function { function_name }, &state),
            Authorization::Deny
        );
    }
    for filename in ["", ":memory:", "file:private.db", "/tmp/private.db"] {
        assert_eq!(
            authorize_action(AuthAction::Attach { filename }, &state),
            Authorization::Deny
        );
    }
}

#[test]
fn only_bare_vacuum_receives_one_temporary_empty_attach_permission() {
    let (connection, policy) = installed();
    execute(
        &connection,
        &policy,
        "CREATE TABLE rows(value); INSERT INTO rows VALUES(1)",
    )
    .unwrap();
    execute(&connection, &policy, " \tVaCuUm ;\n").unwrap();
    assert!(!policy.reusable());
    policy.begin("VACUUM").unwrap();
    assert_eq!(
        authorize_action(AuthAction::Attach { filename: "" }, &policy.state),
        Authorization::Allow
    );
    assert_eq!(
        authorize_action(AuthAction::Attach { filename: "" }, &policy.state),
        Authorization::Deny
    );
    policy.finish();
    for sql in [
        "VACUUM INTO ''",
        "VACUUM main",
        "VACUUM;;",
        "/*x*/VACUUM",
        "VACUUM\0",
        "VACUUM\u{a0}",
    ] {
        assert!(!is_bare_vacuum(sql), "{sql:?}");
    }
    policy.begin("VACUUM").unwrap();
    policy.finish();
    assert_eq!(
        authorize_action(AuthAction::Attach { filename: "" }, &policy.state),
        Authorization::Deny
    );
}

#[test]
fn expired_progress_interrupts_without_sleep_and_finish_resets_budget() {
    let (connection, policy) = installed();
    let sql = "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n";
    policy.begin(sql).unwrap();
    assert_eq!(
        policy.begin(sql).unwrap_err().code,
        SqlErrorCodeV1::InvalidState
    );
    policy.state.lock().unwrap().deadline = Some(Instant::now());
    let error = connection
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(policy.error(error).code, SqlErrorCodeV1::ExecutionTimeout);
    assert_eq!(
        policy.checkpoint().unwrap_err().code,
        SqlErrorCodeV1::ExecutionTimeout
    );
    policy.finish();
    execute(&connection, &policy, "SELECT 1").unwrap();
}

#[test]
fn wire_cell_and_parameter_limits_do_not_limit_whole_rows_to_one_cell() {
    check_cell(1024 * 1024).unwrap();
    assert_eq!(
        check_cell(1024 * 1024 + 1).unwrap_err().code,
        SqlErrorCodeV1::CellTooLarge
    );
    check_parameters(&vec![SqlValueV1::Null; 64]).unwrap();
    assert_eq!(
        check_parameters(&vec![SqlValueV1::Null; 65])
            .unwrap_err()
            .code,
        SqlErrorCodeV1::ParameterLimit
    );
    let large = SqlValueV1::String("x".repeat(1024 * 1024));
    check_parameters(&vec![large.clone(); 8]).unwrap();
    assert_eq!(
        check_parameters(&vec![large; 9]).unwrap_err().code,
        SqlErrorCodeV1::ParameterBytesLimit
    );
    let (_, policy) = installed();
    assert_eq!(
        policy.begin(&" ".repeat(256 * 1024 + 1)).unwrap_err().code,
        SqlErrorCodeV1::SqlTooLarge
    );
}

#[test]
fn scalar_proof_accepts_only_direct_ordinary_main_reads_and_constants() {
    let (connection, policy) = installed();
    execute(
        &connection,
        &policy,
        "CREATE TABLE ExactName(value); INSERT INTO ExactName VALUES(7); \
         CREATE VIEW indirect AS SELECT value FROM ExactName; \
         CREATE TEMP TABLE scratch(value); INSERT INTO scratch VALUES(9)",
    )
    .unwrap();
    let tables = Arc::new(BTreeSet::from(["ExactName".to_owned()]));
    for (sql, expected) in [
        ("SELECT value FROM ExactName", true),
        ("SELECT 1 + 2", true),
        ("SELECT abs(value) FROM ExactName", false),
        ("SELECT random()", false),
        ("SELECT value FROM indirect", false),
        ("SELECT value FROM temp.scratch", false),
        ("SELECT name FROM sqlite_schema", false),
        ("PRAGMA user_version", false),
        ("UPDATE ExactName SET value=8 RETURNING value", false),
    ] {
        policy.begin_scalar(sql, &tables).unwrap();
        assert!(!policy.reusable(), "no preparation evidence yet: {sql}");
        let statement = connection.prepare(sql).unwrap();
        assert_eq!(policy.reusable() && statement.readonly(), expected, "{sql}");
        drop(statement);
        policy.finish();
        assert!(!policy.reusable());
    }
}

#[test]
fn scalar_proof_is_exact_scoped_and_sticky_after_disqualifying_actions() {
    let (_, policy) = installed();
    let mut tables = Arc::new(BTreeSet::from(["ExactName".to_owned()]));
    let select = AuthContext {
        action: AuthAction::Select,
        database_name: None,
        accessor: None,
    };
    for (database_name, table_name, accessor) in [
        (Some("temp"), "ExactName", None),
        (Some("other"), "ExactName", None),
        (Some("main"), "exactname", None),
        (Some("main"), "virtual_table", None),
        (Some("main"), "ExactName", Some("a_view")),
        (Some("main"), "ExactName", Some("a_trigger")),
    ] {
        policy
            .begin_scalar("SELECT value FROM ExactName", &tables)
            .unwrap();
        assert_eq!(authorize(select, &policy.state), Authorization::Allow);
        assert!(policy.reusable());
        assert_eq!(
            authorize(
                AuthContext {
                    action: AuthAction::Read {
                        table_name,
                        column_name: "value"
                    },
                    database_name,
                    accessor,
                },
                &policy.state
            ),
            Authorization::Allow
        );
        assert!(!policy.reusable());
        assert_eq!(authorize(select, &policy.state), Authorization::Allow);
        assert!(!policy.reusable());
        policy.finish();
    }
    policy
        .begin_scalar("SELECT value FROM ExactName", &tables)
        .unwrap();
    assert!(Arc::ptr_eq(
        &policy
            .state
            .lock()
            .unwrap()
            .scalar_proof
            .as_ref()
            .unwrap()
            .ordinary_tables,
        &tables,
    ));
    Arc::make_mut(&mut tables).clear();
    assert_eq!(authorize(select, &policy.state), Authorization::Allow);
    assert_eq!(
        authorize(
            AuthContext {
                action: AuthAction::Read {
                    table_name: "ExactName",
                    column_name: "value"
                },
                database_name: Some("main"),
                accessor: None,
            },
            &policy.state
        ),
        Authorization::Allow
    );
    assert!(policy.reusable());
    policy.finish();
    policy.begin("").unwrap();
    assert_eq!(authorize(select, &policy.state), Authorization::Allow);
    assert!(!policy.reusable());
    assert_eq!(
        authorize_action(AuthAction::Attach { filename: "" }, &policy.state),
        Authorization::Deny
    );
    policy.finish();
}

#[test]
fn scalar_proof_never_replaces_or_relaxes_the_security_authorizer() {
    let (connection, policy) = installed();
    let tables = Arc::new(BTreeSet::new());
    for sql in [
        "ATTACH ':memory:' AS extra",
        "PRAGMA trusted_schema=ON",
        "VACUUM INTO ':memory:'",
    ] {
        policy.begin_scalar(sql, &tables).unwrap();
        let failure = connection.execute_batch(sql).unwrap_err();
        let _mapped = policy.error(failure);
        assert!(!policy.reusable());
        policy.finish();
    }
    policy.begin_scalar("SELECT 1", &tables).unwrap();
    assert_eq!(
        authorize_action(AuthAction::Select, &policy.state),
        Authorization::Allow
    );
    assert!(policy.reusable());
    assert_eq!(
        authorize_action(
            AuthAction::Function {
                function_name: "load_extension"
            },
            &policy.state
        ),
        Authorization::Deny
    );
    assert!(!policy.reusable());
    policy.finish();
    policy.begin_scalar("SELECT 1", &tables).unwrap();
    let statement = connection.prepare("SELECT 1").unwrap();
    assert!(policy.reusable() && statement.readonly());
    drop(statement);
    policy.finish();
    execute(&connection, &policy, "VACUUM").unwrap();
    assert!(!policy.reusable());
}

#[test]
fn owned_scope_releases_budget_on_prepare_failure_and_unwind() {
    let (connection, policy) = installed();
    assert!(execute(&connection, &policy, "SELECT FROM").is_err());
    assert!(policy.state.lock().unwrap().deadline.is_none());
    execute(&connection, &policy, "SELECT 1").unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = policy.scope("VACUUM").unwrap();
        panic!("exercise scope unwinding");
    }));
    assert!(result.is_err());
    let state = policy.state.lock().unwrap();
    assert!(state.deadline.is_none());
    assert!(!state.allow_vacuum_attach);
    drop(state);
    // Scope has no lifetime tied to the policy or database container.
    let scope = policy
        .scalar_scope("SELECT 1", &Arc::new(BTreeSet::new()))
        .unwrap();
    let moved_policy = policy;
    let statement = connection.prepare("SELECT 1").unwrap();
    assert!(moved_policy.reusable());
    drop(statement);
    drop(scope);
    assert!(!moved_policy.reusable());
    drop(moved_policy.scope("SELECT 2").unwrap());
}

#[test]
fn cancellation_and_transport_expiry_survive_scope_finish_and_new_requests() {
    let (connection, policy) = installed();
    let control = policy.state.lock().unwrap().control.clone();
    control
        .begin_request(Instant::now() + Duration::from_secs(30))
        .unwrap();
    let scope = policy.scope("SELECT 1").unwrap();
    assert_eq!(LIMITS.execution_budget_ms, 5_000);
    control.cancel();
    assert!(connection.prepare("SELECT 1").is_err());
    assert_eq!(
        policy.checkpoint().unwrap_err().code,
        SqlErrorCodeV1::ExecutionTimeout
    );
    drop(scope);
    assert!(policy.scope("SELECT 2").is_err());
    assert!(control.finish_request().is_err());
    assert!(
        control
            .begin_request(Instant::now() + Duration::from_secs(30))
            .is_err()
    );

    let control = Control::default();
    assert!(control.begin_request(Instant::now()).is_err());
    assert!(control.expired());
    assert!(
        control
            .begin_request(Instant::now() + Duration::from_secs(30))
            .is_err()
    );
}

#[test]
fn initialization_finishes_its_scope_and_refuses_cancelled_control() {
    let (_, policy) = installed();
    assert!(policy.state.lock().unwrap().deadline.is_none());
    let connection = Connection::open_in_memory().unwrap();
    let control = Control::default();
    control.cancel();
    assert!(matches!(Policy::install(&connection, control),
        Err(error) if error.code == SqlErrorCodeV1::ExecutionTimeout));
}

#[test]
fn shared_request_cancellation_interrupts_an_already_prepared_vm() {
    let (connection, policy) = installed();
    let control = policy.state.lock().unwrap().control.clone();
    control
        .begin_request(Instant::now() + Duration::from_secs(30))
        .unwrap();
    let sql = "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n";
    let scope = policy.scope(sql).unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    control.cancel();
    let error = statement
        .query_row([], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert_eq!(policy.error(error).code, SqlErrorCodeV1::ExecutionTimeout);
    drop(statement);
    drop(scope);
    assert!(policy.scope("SELECT 1").is_err());
}

#[test]
fn seed_virtual_definitions_are_rejected_without_connecting_the_module() {
    for definition in [
        "CREATE VIRTUAL TABLE forbidden USING nonexistent_module(value)",
        "create /* seed */ virtual\n table forbidden USING nonexistent_module(value)",
    ] {
        assert!(is_virtual_definition(definition));
        let connection = Connection::open_in_memory().unwrap();
        // Inject a persisted virtual schema record without registering or invoking
        // any module. Inventory must inspect schema text, never connect this table.
        connection
            .execute_batch("PRAGMA writable_schema=ON")
            .unwrap();
        connection.execute(
            "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql) VALUES('table','forbidden','forbidden',0,?1)",
            [definition],
        ).unwrap();
        connection
            .execute_batch("PRAGMA writable_schema=OFF")
            .unwrap();
        assert!(matches!(Policy::install(&connection, Control::default()),
            Err(error) if error.code == SqlErrorCodeV1::Unsupported));
    }
    assert!(!is_virtual_definition(
        "CREATE TABLE ordinary(value TEXT DEFAULT 'CREATE VIRTUAL TABLE')"
    ));
}

#[test]
fn every_eponymous_module_is_denied_by_the_security_and_scalar_authorizer() {
    let (connection, policy) = installed();
    for table_name in [
        "sqlite_dbpage",
        "dbstat",
        "bytecode",
        "stmt",
        "sqlite_stmt",
        "json_each",
        "json_tree",
        "pragma_table_info",
    ] {
        let tables = Arc::new(BTreeSet::from([table_name.to_owned()]));
        let _scope = policy.scalar_scope("SELECT 1", &tables).unwrap();
        for action in [
            AuthAction::Read {
                table_name,
                column_name: "",
            },
            AuthAction::Insert { table_name },
            AuthAction::Update {
                table_name,
                column_name: "value",
            },
            AuthAction::Delete { table_name },
        ] {
            assert_eq!(
                authorize(
                    AuthContext {
                        action,
                        database_name: Some("main"),
                        accessor: None
                    },
                    &policy.state
                ),
                Authorization::Deny,
                "{table_name}"
            );
            assert!(!policy.reusable());
        }
    }
    for sql in [
        "SELECT * FROM json_each('[1]')",
        "SELECT count(*) FROM json_tree('{}')",
        "SELECT * FROM sqlite_dbpage",
        "SELECT * FROM dbstat",
        "SELECT * FROM bytecode('SELECT 1')",
        "SELECT * FROM sqlite_stmt",
        "SELECT * FROM pragma_table_info('ordinary')",
    ] {
        assert!(execute(&connection, &policy, sql).is_err(), "{sql}");
    }
    execute(
        &connection,
        &policy,
        "CREATE VIEW indirect_virtual AS SELECT * FROM json_each('[1]')",
    )
    .unwrap();
    assert!(execute(&connection, &policy, "SELECT * FROM indirect_virtual").is_err());
    execute(&connection, &policy, "SELECT 1").unwrap();
}
