impl Provider {
    fn run_connected(
        &mut self,
        operation: &SqlOperationV1,
        database: &mut Database,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> Result<SqlResultV1> {
        match operation {
            SqlOperationV1::Execute {
                sql,
                mode,
                parameters,
                ..
            } => self.execute_connected(sql, *mode, parameters, database, minor, storage),
            SqlOperationV1::ReaderRead { reader } => {
                self.read_row(*reader, database, minor, storage)
            }
            SqlOperationV1::ReaderGet {
                reader,
                column,
                mode,
            } => {
                let _scope = database.policy.scope("")?;
                let value = self
                    .readers
                    .get_mut(&(reader.service_epoch, reader.id))
                    .ok_or_else(missing_reader)?;
                if value.status != SqlReaderStatusV1::Row {
                    return Err(missing_reader());
                }
                let column = *column as usize;
                let original = value.original_types.get(column).copied().flatten();
                let cursor = value.cursor.as_mut().ok_or_else(missing_reader)?;
                Ok(SqlResultV1::ReaderValue {
                    value: values::reader(cursor, column, *mode, original)?,
                })
            }
            SqlOperationV1::ReaderIsNull { reader, column } => {
                let _scope = database.policy.scope("")?;
                let value = self
                    .readers
                    .get_mut(&(reader.service_epoch, reader.id))
                    .ok_or_else(missing_reader)?;
                if value.status != SqlReaderStatusV1::Row {
                    return Err(missing_reader());
                }
                let column = *column as usize;
                let original = value.original_types.get(column).copied().flatten();
                let cursor = value.cursor.as_mut().ok_or_else(missing_reader)?;
                Ok(SqlResultV1::ReaderNull {
                    is_null: values::native(cursor, column, original)? == SqlValueV1::Null,
                })
            }
            SqlOperationV1::ReaderClose { reader } => {
                let scope = database.policy.scope("")?;
                let mut value = self
                    .readers
                    .remove(&(reader.service_epoch, reader.id))
                    .ok_or_else(missing_reader)?;
                let mutating = !value.readonly;
                if let Some(cursor) = value.cursor.take() {
                    cursor
                        .finalize()
                        .map_err(|error| database.policy.error(error))?;
                }
                drop(value);
                drop(scope);
                if mutating {
                    database.ordinary_tables = None;
                    self.publish(database, storage)?;
                }
                Ok(SqlResultV1::ReaderClosed)
            }
            SqlOperationV1::ImportMapRows { table, rows, .. } => {
                import_map(database, table, rows)?;
                database.ordinary_tables = None;
                self.publish(database, storage)?;
                Ok(SqlResultV1::MapImported {
                    rows: u32::try_from(rows.len()).expect("bounded MAP rows"),
                })
            }
            _ => Err(ProviderError::new(
                SqlErrorCodeV1::InvalidRequest,
                "unexpected connected SQL operation",
            )),
        }
    }

    fn execute_connected(
        &mut self,
        sql: &str,
        mode: SqlExecuteModeV1,
        parameters: &[SqlValueV1],
        database: &mut Database,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> Result<SqlResultV1> {
        crate::policy::check_parameters(parameters)?;
        let scope = if matches!(
            mode,
            SqlExecuteModeV1::ScalarInteger | SqlExecuteModeV1::ScalarString
        ) && minor >= 1
        {
            if database.ordinary_tables.is_none() {
                let scope = database.policy.scope("")?;
                let inventory = ordinary_tables(&database.db);
                drop(scope);
                database.ordinary_tables = Some(Arc::new(inventory?));
            }
            database.policy.scalar_scope(
                sql,
                database
                    .ordinary_tables
                    .as_ref()
                    .expect("table inventory initialized"),
            )?
        } else {
            database.policy.scope(sql)?
        };
        let mut result = self.execute(database, sql, mode, parameters);
        if minor >= 1
            && database.policy.reusable()
            && let Ok((SqlResultV1::Scalar { value }, false)) = &result
        {
            result = Ok((
                SqlResultV1::ReusableScalar {
                    value: value.clone(),
                },
                false,
            ));
        }
        drop(scope);
        if result.is_err() {
            database.ordinary_tables = None;
        }
        if result.as_ref().is_ok_and(|(_, mutating)| *mutating) {
            database.ordinary_tables = None;
            self.publish(database, storage)?;
        }
        result.map(|(result, _)| result)
    }

    fn read_row(
        &mut self,
        reader: SqlReaderHandleV1,
        database: &mut Database,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> Result<SqlResultV1> {
        let value = self
            .readers
            .get_mut(&(reader.service_epoch, reader.id))
            .ok_or_else(missing_reader)?;
        if value.status == SqlReaderStatusV1::Eof {
            return Ok(SqlResultV1::ReaderAdvanced { has_row: false });
        }
        let scope = database.policy.scope("")?;
        let step = value
            .cursor
            .as_mut()
            .ok_or_else(missing_reader)?
            .step()
            .map_err(|error| database.policy.error(error));
        if step? {
            value.rows_read += 1;
            if value.rows_read > SqlLimitsV1::FIXED.maximum_reader_rows {
                return Err(ProviderError::new(
                    SqlErrorCodeV1::ReaderRowLimit,
                    "SQL reader row limit",
                ));
            }
            value.status = SqlReaderStatusV1::Row;
            let cursor = value.cursor.as_mut().ok_or_else(missing_reader)?;
            value.original_types.clear();
            if minor >= 2 {
                for column in 0..cursor.column_count().min(32) {
                    let kind = cursor.column_type(column)?;
                    value.original_types.push(
                        matches!(
                            kind,
                            rusqlite::ffi::SQLITE_INTEGER
                                | rusqlite::ffi::SQLITE_TEXT
                                | rusqlite::ffi::SQLITE_NULL
                        )
                        .then_some(kind),
                    );
                }
                Ok(SqlResultV1::ReaderRow {
                    cells: values::project(cursor, &value.original_types),
                })
            } else {
                Ok(SqlResultV1::ReaderAdvanced { has_row: true })
            }
        } else {
            value.status = SqlReaderStatusV1::Eof;
            if let Some(cursor) = value.cursor.take() {
                cursor
                    .finalize()
                    .map_err(|error| database.policy.error(error))?;
            }
            let mutating = !value.readonly;
            drop(scope);
            if mutating {
                database.ordinary_tables = None;
                self.publish(database, storage)?;
            }
            Ok(SqlResultV1::ReaderAdvanced { has_row: false })
        }
    }

    fn execute(
        &mut self,
        database: &Database,
        sql: &str,
        mode: SqlExecuteModeV1,
        parameters: &[SqlValueV1],
    ) -> Result<(SqlResultV1, bool)> {
        if mode == SqlExecuteModeV1::NonQuery {
            let before = database.db.total_changes();
            let mut remainder = sql;
            let mut needs_binding = !parameters.is_empty();
            while !remainder.is_empty() {
                database.policy.checkpoint()?;
                let (cursor, consumed) = Cursor::prepare(Rc::clone(&database.db), remainder)
                    .map_err(|error| database.policy.error(error))?;
                if consumed == 0 {
                    break;
                }
                remainder = &remainder[consumed..];
                if let Some(mut cursor) = cursor {
                    // OO1 DB.exec binds only the first statement that has parameters.
                    if needs_binding && cursor.parameter_count() != 0 {
                        cursor.bind(parameters)?;
                        needs_binding = false;
                    }
                    // Without a row callback OO1 steps once, then finalizes, even for SELECT.
                    cursor
                        .step()
                        .map_err(|error| database.policy.error(error))?;
                    cursor
                        .finalize()
                        .map_err(|error| database.policy.error(error))?;
                    database.policy.checkpoint()?;
                }
            }
            return Ok((
                SqlResultV1::NonQuery {
                    affected_rows: i64::try_from(database.db.total_changes() - before).map_err(
                        |_| {
                            ProviderError::new(
                                SqlErrorCodeV1::TypeMismatch,
                                "affected rows overflow",
                            )
                        },
                    )?,
                },
                true,
            ));
        }
        database.policy.checkpoint()?;
        let (cursor, _) = Cursor::prepare(Rc::clone(&database.db), sql)
            .map_err(|error| database.policy.error(error))?;
        let mut cursor = cursor.ok_or_else(|| {
            ProviderError::new(SqlErrorCodeV1::Sqlite, "SQL does not contain a statement")
        })?;
        cursor.bind(parameters)?;
        let readonly = cursor.readonly();
        if mode == SqlExecuteModeV1::Reader {
            if self.readers.len() >= 32 {
                return Err(ProviderError::new(
                    SqlErrorCodeV1::ReaderLimit,
                    "SQL reader limit",
                ));
            }
            self.next_reader = self.next_reader.checked_add(1).ok_or_else(|| {
                ProviderError::new(SqlErrorCodeV1::ReaderLimit, "SQL reader handle exhausted")
            })?;
            let handle = SqlReaderHandleV1 {
                service_epoch: database.handle.service_epoch,
                id: self.next_reader,
            };
            self.readers.insert(
                (handle.service_epoch, handle.id),
                Reader {
                    handle,
                    connection: (database.handle.service_epoch, database.handle.id),
                    cursor: Some(cursor),
                    readonly,
                    status: SqlReaderStatusV1::BeforeFirst,
                    rows_read: 0,
                    original_types: Vec::new(),
                },
            );
            return Ok((SqlResultV1::ReaderOpened { reader: handle }, false));
        }
        let value = if cursor
            .step()
            .map_err(|error| database.policy.error(error))?
            && cursor.column_count() != 0
        {
            values::scalar(&mut cursor, mode)?
        } else {
            SqlValueV1::Null
        };
        database.policy.checkpoint()?;
        cursor
            .finalize()
            .map_err(|error| database.policy.error(error))?;
        Ok((SqlResultV1::Scalar { value }, !readonly))
    }
}

fn missing_reader() -> ProviderError {
    ProviderError::new(
        SqlErrorCodeV1::ReaderNotFound,
        "SQL reader not positioned on a row",
    )
}

fn ordinary_tables(database: &Connection) -> Result<BTreeSet<String>> {
    let mut statement =
        database.prepare("SELECT name,sql FROM main.sqlite_schema WHERE type='table'")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut tables = BTreeSet::new();
    for row in rows {
        let (name, sql) = row?;
        if sql.is_some_and(|sql| {
            sql.trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE TABLE")
        }) {
            tables.insert(name);
        }
    }
    Ok(tables)
}

fn import_map(database: &mut Database, table: &str, rows: &[SqlMapRowV1]) -> Result<()> {
    let mut letters = table.bytes();
    if !letters
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        || !letters.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(ProviderError::new(
            SqlErrorCodeV1::InvalidTableName,
            "invalid SQL MAP table",
        ));
    }
    if rows.len() > 100_000 {
        return Err(ProviderError::new(
            SqlErrorCodeV1::MapRowLimit,
            "SQL MAP row limit",
        ));
    }
    for row in rows {
        crate::policy::check_cell(row.key.len())?;
        crate::policy::check_cell(row.value.len())?;
    }
    if rows
        .iter()
        .map(|row| row.key.len() + row.value.len())
        .sum::<usize>()
        > 8 * 1024 * 1024
    {
        return Err(ProviderError::new(
            SqlErrorCodeV1::MapBytesLimit,
            "SQL MAP bytes limit",
        ));
    }
    let scope = database.policy.scope("")?;
    let mut savepoint = false;
    let result = (|| -> Result<()> {
        database.db.execute_batch("SAVEPOINT rustyera_map_import")?;
        savepoint = true;
        database.db.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (k TEXT PRIMARY KEY, v TEXT); DELETE FROM {table}"
        ))?;
        let mut statement = database.db.prepare(&format!(
            "INSERT OR REPLACE INTO {table}(k,v) VALUES(?1,?2)"
        ))?;
        for row in rows {
            database.policy.checkpoint()?;
            statement
                .execute((&row.key, &row.value))
                .map_err(|error| database.policy.error(error))?;
        }
        database.db.execute_batch("RELEASE rustyera_map_import")?;
        Ok(())
    })();
    drop(scope);
    if result.is_err() && savepoint {
        let cleanup = (|| -> Result<()> {
            let _scope = database.policy.scope("")?;
            database
                .db
                .execute_batch("ROLLBACK TO rustyera_map_import; RELEASE rustyera_map_import")?;
            Ok(())
        })();
        if cleanup.is_err() {
            database.poisoned = true;
        }
    }
    result
}
