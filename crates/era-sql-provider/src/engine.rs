use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    SQL_DATABASE_FORMAT_VERSION, SQL_SQLITE_VERSION, SqlConnectionHandleV1, SqlDatabaseStateV1,
    SqlErrorCodeV1, SqlErrorContextV1, SqlErrorV1, SqlExecuteModeV1, SqlLimitsV1, SqlMapRowV1,
    SqlOperationKindV1, SqlOperationV1, SqlProviderHandleV1, SqlReaderHandleV1, SqlReaderStateV1,
    SqlReaderStatusV1, SqlRequestV1, SqlResponseV1, SqlResultV1, SqlRevisionV1, SqlValueV1,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::cursor::Cursor;
use crate::policy::{Control, Policy};
use crate::values;
use crate::{CommitOutcome, ProviderError, Result, RevisionStore, SqlChain, SqlStorage};

type Key = (u64, u64);

#[derive(Default)]
pub(crate) struct Engine {
    providers: BTreeMap<Key, Provider>,
    live: Option<Key>,
    candidate: Option<Key>,
    highest_registered: Key,
    control: Control,
}

#[derive(Default)]
struct Provider {
    connections: BTreeMap<Key, Database>,
    readers: BTreeMap<Key, Reader>,
    next_reader: u64,
    revisions: RevisionStore,
    control: Control,
}

struct Database {
    handle: SqlConnectionHandleV1,
    db: Rc<Connection>,
    policy: Policy,
    chain: SqlChain,
    durable_revision: Option<SqlRevisionV1>,
    durable_bytes: Option<Vec<u8>>,
    poisoned: bool,
    ordinary_tables: Option<Arc<BTreeSet<String>>>,
}

struct Reader {
    handle: SqlReaderHandleV1,
    connection: Key,
    cursor: Option<Cursor>,
    readonly: bool,
    status: SqlReaderStatusV1,
    rows_read: u64,
    original_types: Vec<Option<i32>>,
}

impl Engine {
    pub(crate) fn new(control: Control) -> Self {
        Self {
            control,
            ..Self::default()
        }
    }

    pub(crate) fn reset(&mut self) {
        self.providers.clear();
        self.live = None;
        self.candidate = None;
        // Retired identifiers cannot be registered again during this owner lifetime.
    }

    pub(crate) fn register(
        &mut self,
        handle: SqlProviderHandleV1,
        role: crate::ProviderRole,
    ) -> Result<()> {
        let key = (handle.service_epoch, handle.id);
        if handle.service_epoch == 0 || handle.id == 0 || key <= self.highest_registered {
            return Err(ProviderError::new(
                SqlErrorCodeV1::StaleEpoch,
                "provider registration is stale",
            ));
        }
        let slot = match role {
            crate::ProviderRole::Live => &mut self.live,
            crate::ProviderRole::Candidate => &mut self.candidate,
        };
        if slot.is_some() {
            return Err(ProviderError::new(
                SqlErrorCodeV1::ConnectionLimit,
                "provider role is already occupied",
            ));
        }
        *slot = Some(key);
        self.highest_registered = key;
        self.providers.insert(
            key,
            Provider {
                control: self.control.clone(),
                ..Provider::default()
            },
        );
        Ok(())
    }

    pub(crate) fn retire(&mut self, handle: SqlProviderHandleV1) -> Result<()> {
        let key = (handle.service_epoch, handle.id);
        if self.providers.remove(&key).is_none() {
            return Err(ProviderError::new(
                SqlErrorCodeV1::StaleEpoch,
                "provider is not registered",
            ));
        }
        if self.live == Some(key) {
            self.live = None;
        }
        if self.candidate == Some(key) {
            self.candidate = None;
        }
        Ok(())
    }

    pub(crate) fn promote_candidate(&mut self, handle: SqlProviderHandleV1) -> Result<()> {
        let key = (handle.service_epoch, handle.id);
        if self.candidate != Some(key) {
            return Err(ProviderError::new(
                SqlErrorCodeV1::StaleEpoch,
                "provider is not the restore candidate",
            ));
        }
        if let Some(live) = self.live.replace(key) {
            self.providers.remove(&live);
        }
        self.candidate = None;
        Ok(())
    }

    pub(crate) fn handle(
        &mut self,
        request: &SqlRequestV1,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> SqlResponseV1 {
        let key = (request.provider.service_epoch, request.provider.id);
        if let Some(state) = self.providers.get_mut(&key) {
            return state.handle(request, minor, storage);
        }
        SqlResponseV1 {
            provider: request.provider,
            database: None,
            reader: None,
            result: wire_error(
                ProviderError::new(
                    SqlErrorCodeV1::StaleEpoch,
                    "provider is retired or unregistered",
                ),
                operation_kind(&request.operation),
            ),
        }
    }
}

impl Database {
    fn state(&self) -> SqlDatabaseStateV1 {
        SqlDatabaseStateV1 {
            connection: self.handle,
            connected: !self.poisoned,
            transaction_active: !self.poisoned && !self.db.is_autocommit(),
            durable_revision: self.durable_revision.clone(),
        }
    }
}

impl Reader {
    fn state(&self) -> SqlReaderStateV1 {
        SqlReaderStateV1 {
            reader: self.handle,
            status: self.status,
            rows_read: self.rows_read,
        }
    }
}

impl Provider {
    fn handle(
        &mut self,
        request: &SqlRequestV1,
        minor: u16,
        storage: &mut dyn SqlStorage,
    ) -> SqlResponseV1 {
        let operation_kind = operation_kind(&request.operation);
        let reader_key = operation_reader(&request.operation);
        let connection_key = operation_connection(&request.operation).or_else(|| {
            reader_key.and_then(|key| self.readers.get(&key).map(|reader| reader.connection))
        });
        let mut database = connection_key.and_then(|key| self.connections.remove(&key));
        let closing_reader = if matches!(request.operation, SqlOperationV1::ReaderClose { .. }) {
            reader_key.and_then(|key| {
                self.readers.get(&key).map(|reader| SqlReaderStateV1 {
                    status: SqlReaderStatusV1::Closed,
                    ..reader.state()
                })
            })
        } else {
            None
        };
        let result = self.run(request, minor, storage, &mut database);
        if result.is_err()
            && let Some(database) = database.as_mut()
        {
            self.recover_unpublished(database);
        }
        let mut state = database.as_ref().map(Database::state);
        if result.is_ok()
            && let SqlOperationV1::Disconnect { connection } = &request.operation
        {
            state.get_or_insert(SqlDatabaseStateV1 {
                connection: *connection,
                connected: false,
                transaction_active: false,
                durable_revision: None,
            });
        }
        let mut reader = reader_key
            .and_then(|key| self.readers.get(&key).map(Reader::state))
            .or_else(|| {
                if let Ok(SqlResultV1::ReaderOpened { reader }) = &result {
                    self.readers
                        .get(&(reader.service_epoch, reader.id))
                        .map(Reader::state)
                } else {
                    None
                }
            })
            .or(closing_reader);
        if let Some(database) = database {
            if database.poisoned
                || (result.is_ok()
                    && matches!(request.operation, SqlOperationV1::Disconnect { .. }))
            {
                if let Some(reader) = reader.as_mut() {
                    reader.status = SqlReaderStatusV1::Closed;
                }
                let key = (database.handle.service_epoch, database.handle.id);
                self.readers.retain(|_, reader| reader.connection != key);
                if let Some(state) = state.as_mut() {
                    state.connected = false;
                    state.transaction_active = false;
                }
            } else {
                self.connections.insert(
                    (database.handle.service_epoch, database.handle.id),
                    database,
                );
            }
        }
        SqlResponseV1 {
            provider: request.provider,
            database: state,
            reader,
            result: result.unwrap_or_else(|error| wire_error(error, operation_kind)),
        }
    }

    fn run(
        &mut self,
        request: &SqlRequestV1,
        minor: u16,
        storage: &mut dyn SqlStorage,
        database: &mut Option<Database>,
    ) -> Result<SqlResultV1> {
        let epoch = request.provider.service_epoch;
        if epoch == 0 || request.provider.id == 0 || minor > 2 {
            return Err(ProviderError::new(
                SqlErrorCodeV1::InvalidRequest,
                "invalid SQL provider identity or version",
            ));
        }
        if let Some((handle_epoch, id)) = operation_connection(&request.operation)
            .or_else(|| operation_reader(&request.operation))
            && (handle_epoch != epoch || id == 0)
        {
            return Err(ProviderError::new(
                SqlErrorCodeV1::StaleEpoch,
                "SQL handle belongs to another provider epoch",
            ));
        }
        match &request.operation {
            operation @ SqlOperationV1::Open { .. } => {
                self.open_connection(operation, database, storage)
            }
            SqlOperationV1::Disconnect { connection } => {
                let key = (connection.service_epoch, connection.id);
                self.readers.retain(|_, reader| reader.connection != key);
                Ok(SqlResultV1::Disconnected)
            }
            SqlOperationV1::ReaderRead { reader }
                if !self
                    .readers
                    .contains_key(&(reader.service_epoch, reader.id)) =>
            {
                Ok(SqlResultV1::ReaderAdvanced { has_row: false })
            }
            SqlOperationV1::ReaderClose { reader }
                if !self
                    .readers
                    .contains_key(&(reader.service_epoch, reader.id)) =>
            {
                Ok(SqlResultV1::ReaderClosed)
            }
            SqlOperationV1::ReaderGet { reader, .. }
            | SqlOperationV1::ReaderIsNull { reader, .. }
                if !self
                    .readers
                    .contains_key(&(reader.service_epoch, reader.id)) =>
            {
                Err(missing_reader())
            }
            operation => {
                let database = database.as_mut().ok_or_else(|| {
                    ProviderError::new(
                        SqlErrorCodeV1::ConnectionNotFound,
                        "SQL connection not found",
                    )
                })?;
                if database.poisoned {
                    return Err(ProviderError::new(
                        SqlErrorCodeV1::InvalidState,
                        "SQL publication outcome is unknown",
                    ));
                }
                self.run_connected(operation, database, minor, storage)
            }
        }
    }

    fn open_connection(
        &mut self,
        operation: &SqlOperationV1,
        database: &mut Option<Database>,
        storage: &mut dyn SqlStorage,
    ) -> Result<SqlResultV1> {
        let SqlOperationV1::Open {
            connection,
            logical_name,
            identity,
            revision,
            limits,
        } = operation
        else {
            return Err(ProviderError::new(
                SqlErrorCodeV1::InvalidRequest,
                "expected open operation",
            ));
        };
        if database.is_some() {
            return Err(ProviderError::new(
                SqlErrorCodeV1::ConnectionConflict,
                "SQL connection already exists",
            ));
        }
        if *limits != SqlLimitsV1::FIXED
            || identity.sqlite_version != SQL_SQLITE_VERSION
            || identity.format_version != SQL_DATABASE_FORMAT_VERSION
        {
            return Err(ProviderError::new(
                SqlErrorCodeV1::Unsupported,
                "SQL version or limits mismatch",
            ));
        }
        if self.connections.len() >= 8 {
            return Err(ProviderError::new(
                SqlErrorCodeV1::ConnectionLimit,
                "SQL connection limit",
            ));
        }
        let material = self
            .revisions
            .open(identity, logical_name, revision, storage, |bytes| {
                open_database(Some(bytes), self.control.clone())
                    .map(|_| ())
                    .map_err(|mut error| {
                        error.code = SqlErrorCodeV1::InvalidSource;
                        error
                    })
            })?;
        let (db, policy) = open_database(material.bytes.as_deref(), self.control.clone())?;
        let mut opened = Database {
            handle: *connection,
            db,
            policy,
            chain: material.chain,
            durable_revision: material.durable_revision,
            durable_bytes: material.bytes,
            poisoned: false,
            ordinary_tables: None,
        };
        // Hydration must not rewrite an immutable historical revision.
        if opened.durable_revision.is_none() {
            self.publish(&mut opened, storage)?;
        }
        *database = Some(opened);
        Ok(SqlResultV1::Opened {
            sqlite_version: rusqlite::version().into(),
            limits: SqlLimitsV1::FIXED,
        })
    }
    fn publish(&mut self, database: &mut Database, storage: &mut dyn SqlStorage) -> Result<()> {
        if !database.db.is_autocommit() {
            return Ok(());
        }
        let bytes = database.db.serialize("main")?.to_vec();
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(ProviderError::new(
                SqlErrorCodeV1::DatabaseTooLarge,
                "SQL database limit",
            ));
        }
        let revision = SqlRevisionV1 {
            sha256: ProtocolBytes::new(Sha256::digest(&bytes).to_vec()),
        };
        if database.durable_revision.as_ref() == Some(&revision) {
            return Ok(());
        }
        let result = self.revisions.publish(
            &mut database.chain,
            database.durable_revision.as_ref(),
            &bytes,
            &revision,
            storage,
        );
        if result.as_ref().map_or_else(
            |error| error.commit_outcome == CommitOutcome::Committed,
            |()| true,
        ) {
            database.durable_revision = Some(revision);
            database.durable_bytes = Some(bytes);
        }
        if result
            .as_ref()
            .is_err_and(|error| error.commit_outcome != CommitOutcome::NotCommitted)
        {
            database.poisoned = true;
        }
        result
    }

    fn recover_unpublished(&mut self, database: &mut Database) {
        if database.poisoned || !database.db.is_autocommit() {
            return;
        }
        let Some(bytes) = &database.durable_bytes else {
            return;
        };
        let changed = database
            .db
            .serialize("main")
            .map_or(true, |current| current.as_ref() != bytes.as_slice());
        if !changed {
            return;
        }
        let key = (database.handle.service_epoch, database.handle.id);
        self.readers.retain(|_, reader| reader.connection != key);
        match open_database(Some(bytes), self.control.clone()) {
            Ok((db, policy)) => {
                database.db = db;
                database.policy = policy;
                database.ordinary_tables = None;
            }
            Err(_) => database.poisoned = true,
        }
    }
}

fn open_database(bytes: Option<&[u8]>, control: Control) -> Result<(Rc<Connection>, Policy)> {
    crate::identity::verify()?;
    let mut db = Connection::open_in_memory()?;
    if let Some(bytes) = bytes {
        db.deserialize_read_exact("main", bytes, bytes.len(), false)?;
    }
    let policy = Policy::install(&db, control)?;
    Ok((Rc::new(db), policy))
}

fn wire_error(mut error: ProviderError, operation: SqlOperationKindV1) -> SqlResultV1 {
    error.context.push(SqlErrorContextV1 {
        key: "commit_outcome".into(),
        value: match error.commit_outcome {
            CommitOutcome::NotCommitted => "not_committed",
            CommitOutcome::Committed => "committed",
            CommitOutcome::Unknown => "unknown",
        }
        .into(),
    });
    SqlResultV1::Error {
        error: SqlErrorV1 {
            code: error.code,
            operation,
            context: error.context,
            sqlite_code: error.sqlite_code,
            sqlite_message: Some(error.message),
        },
    }
}

fn operation_connection(operation: &SqlOperationV1) -> Option<Key> {
    match operation {
        SqlOperationV1::Open { connection, .. }
        | SqlOperationV1::Execute { connection, .. }
        | SqlOperationV1::ImportMapRows { connection, .. }
        | SqlOperationV1::Disconnect { connection } => {
            Some((connection.service_epoch, connection.id))
        }
        _ => None,
    }
}

fn operation_reader(operation: &SqlOperationV1) -> Option<Key> {
    match operation {
        SqlOperationV1::ReaderRead { reader }
        | SqlOperationV1::ReaderGet { reader, .. }
        | SqlOperationV1::ReaderIsNull { reader, .. }
        | SqlOperationV1::ReaderClose { reader } => Some((reader.service_epoch, reader.id)),
        _ => None,
    }
}

fn operation_kind(operation: &SqlOperationV1) -> SqlOperationKindV1 {
    match operation {
        SqlOperationV1::Open { .. } => SqlOperationKindV1::Open,
        SqlOperationV1::Execute { .. } => SqlOperationKindV1::Execute,
        SqlOperationV1::ReaderRead { .. } => SqlOperationKindV1::ReaderRead,
        SqlOperationV1::ReaderGet { .. } => SqlOperationKindV1::ReaderGet,
        SqlOperationV1::ReaderIsNull { .. } => SqlOperationKindV1::ReaderIsNull,
        SqlOperationV1::ReaderClose { .. } => SqlOperationKindV1::ReaderClose,
        SqlOperationV1::ImportMapRows { .. } => SqlOperationKindV1::ImportMapRows,
        SqlOperationV1::Disconnect { .. } => SqlOperationKindV1::Disconnect,
    }
}

include!("engine/operations.rs");

#[cfg(test)]
mod tests;
