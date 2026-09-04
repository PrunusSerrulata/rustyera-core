//! Runtime-owned state and validation for the safe SQL host service.

use std::collections::{BTreeMap, BTreeSet};

use era_runtime_protocol::{
    SqlConnectionHandleV1, SqlDatabaseIdentityV1, SqlDatabaseSourceV1, SqlProviderHandleV1,
    SqlReaderHandleV1, SqlReaderStatusV1, SqlRevisionV1,
};

use crate::runtime_snapshot::{SqlConnectionSnapshot, SqlRuntimeSnapshot};

#[derive(Clone, Debug)]
pub(crate) struct SqlConnection {
    pub(crate) logical_name: String,
    pub(crate) identity: SqlDatabaseIdentityV1,
    pub(crate) handle: SqlConnectionHandleV1,
    /// BLAKE3 identity from the loaded project Resource graph. This is intentionally distinct
    /// from the SHA-256 seed identity sent to the SQL provider.
    pub(crate) resource_digest: Option<[u8; 32]>,
    pub(crate) transaction_active: bool,
    pub(crate) durable_revision: Option<SqlRevisionV1>,
}

#[derive(Clone, Debug)]
pub(crate) struct SqlReader {
    pub(crate) connection: String,
    pub(crate) handle: SqlReaderHandleV1,
    pub(crate) status: SqlReaderStatusV1,
    pub(crate) rows_read: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlSnapshotBlocker {
    Inflight,
    Reader,
    Transaction,
    RevisionMissing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SqlOpeningSource {
    Memory,
    Resource(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlOpenReservationError {
    Duplicate,
    Limit,
    Exhausted,
}

#[derive(Clone, Debug)]
struct SqlOpening {
    source: SqlOpeningSource,
    handle: SqlConnectionHandleV1,
}

#[derive(Clone, Debug)]
pub(crate) struct SqlRuntimeState {
    service_epoch: u64,
    next_connection_id: u64,
    next_reader_id: i64,
    connections: BTreeMap<String, SqlConnection>,
    readers: BTreeMap<i64, SqlReader>,
    busy_connections: BTreeSet<String>,
    opening: BTreeMap<String, SqlOpening>,
}

impl Default for SqlRuntimeState {
    fn default() -> Self {
        Self {
            service_epoch: 1,
            next_connection_id: 1,
            next_reader_id: 1,
            connections: BTreeMap::new(),
            readers: BTreeMap::new(),
            busy_connections: BTreeSet::new(),
            opening: BTreeMap::new(),
        }
    }
}

impl SqlRuntimeState {
    pub(crate) const fn service_epoch(&self) -> u64 {
        self.service_epoch
    }

    pub(crate) const fn provider(&self) -> SqlProviderHandleV1 {
        SqlProviderHandleV1 {
            service_epoch: self.service_epoch,
            id: 1,
        }
    }

    pub(crate) fn connection_mut(&mut self, key: &str) -> Option<&mut SqlConnection> {
        self.connections.get_mut(key)
    }

    pub(crate) fn connection_by_key(&self, key: &str) -> Option<&SqlConnection> {
        self.connections.get(key)
    }

    pub(crate) fn connections(&self) -> impl Iterator<Item = (&String, &SqlConnection)> {
        self.connections.iter()
    }

    pub(crate) fn cleanup_handles(&self) -> Vec<SqlConnectionHandleV1> {
        self.connections
            .values()
            .map(|connection| connection.handle)
            .chain(self.opening.values().map(|opening| opening.handle))
            .collect()
    }

    fn allocate_connection_handle(&mut self) -> Option<SqlConnectionHandleV1> {
        let id = self.next_connection_id;
        self.next_connection_id = self.next_connection_id.checked_add(1)?;
        Some(SqlConnectionHandleV1 {
            service_epoch: self.service_epoch,
            id,
        })
    }

    pub(crate) fn reserve_open(
        &mut self,
        key: String,
        source: SqlOpeningSource,
    ) -> Result<SqlConnectionHandleV1, SqlOpenReservationError> {
        if self.connections.contains_key(&key) || self.opening.contains_key(&key) {
            return Err(SqlOpenReservationError::Duplicate);
        }
        if self.connections.len() + self.opening.len()
            >= era_runtime_protocol::SqlLimitsV1::FIXED.maximum_connections as usize
        {
            return Err(SqlOpenReservationError::Limit);
        }
        let handle = self
            .allocate_connection_handle()
            .ok_or(SqlOpenReservationError::Exhausted)?;
        self.opening.insert(key, SqlOpening { source, handle });
        Ok(handle)
    }

    pub(crate) fn opening_matches(
        &self,
        key: &str,
        source: &SqlOpeningSource,
        handle: SqlConnectionHandleV1,
    ) -> bool {
        self.opening
            .get(key)
            .is_some_and(|opening| opening.source == *source && opening.handle == handle)
    }

    pub(crate) fn release_open(&mut self, key: &str, handle: SqlConnectionHandleV1) -> bool {
        if self
            .opening
            .get(key)
            .is_some_and(|opening| opening.handle == handle)
        {
            self.opening.remove(key);
            true
        } else {
            false
        }
    }

    pub(crate) fn insert_connection(&mut self, connection: SqlConnection) -> bool {
        let Some(key) = normalize_sql_name(&connection.logical_name) else {
            return false;
        };
        match self.connections.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(connection);
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        }
    }

    pub(crate) fn remove_connection(&mut self, key: &str) -> Option<SqlConnection> {
        self.busy_connections.remove(key);
        self.readers.retain(|_, reader| reader.connection != key);
        self.connections.remove(key)
    }

    pub(crate) fn reserve_connection(&mut self, key: &str) -> bool {
        self.connections.contains_key(key) && self.busy_connections.insert(key.to_owned())
    }

    pub(crate) fn release_connection(&mut self, key: &str) -> bool {
        self.busy_connections.remove(key)
    }

    pub(crate) fn allocate_reader_id(&mut self) -> Option<i64> {
        if self.readers.len() >= era_runtime_protocol::SqlLimitsV1::FIXED.maximum_readers as usize {
            return None;
        }
        let id = self.next_reader_id;
        self.next_reader_id = self.next_reader_id.checked_add(1)?;
        Some(id)
    }

    pub(crate) fn reader(&self, id: i64) -> Option<&SqlReader> {
        self.readers.get(&id)
    }

    pub(crate) fn reader_mut(&mut self, id: i64) -> Option<&mut SqlReader> {
        self.readers.get_mut(&id)
    }

    pub(crate) fn insert_reader(&mut self, id: i64, reader: SqlReader) -> bool {
        match self.readers.entry(id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(reader);
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        }
    }

    pub(crate) fn remove_reader(&mut self, id: i64) -> Option<SqlReader> {
        self.readers.remove(&id)
    }

    pub(crate) fn has_inflight(&self) -> bool {
        !self.opening.is_empty() || !self.busy_connections.is_empty()
    }

    pub(crate) fn has_active_readers(&self) -> bool {
        !self.readers.is_empty()
    }

    pub(crate) fn has_active_transactions(&self) -> bool {
        self.connections
            .values()
            .any(|connection| connection.transaction_active)
    }

    pub(crate) fn snapshot(&self) -> Result<SqlRuntimeSnapshot, SqlSnapshotBlocker> {
        if self.has_inflight() {
            return Err(SqlSnapshotBlocker::Inflight);
        }
        if self.has_active_readers() {
            return Err(SqlSnapshotBlocker::Reader);
        }
        if self.has_active_transactions() {
            return Err(SqlSnapshotBlocker::Transaction);
        }
        let connections = self
            .connections
            .values()
            .map(|connection| {
                Ok(SqlConnectionSnapshot {
                    logical_name: connection.logical_name.clone(),
                    identity: connection.identity.clone(),
                    durable_revision: connection
                        .durable_revision
                        .clone()
                        .ok_or(SqlSnapshotBlocker::RevisionMissing)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SqlRuntimeSnapshot { connections })
    }

    /// Invalidate every provider handle at a project/session ownership boundary.
    pub(crate) fn reset_for_project_boundary(&mut self) {
        self.service_epoch = self
            .service_epoch
            .checked_add(1)
            .expect("SQL service epoch exhausted");
        self.next_connection_id = 1;
        self.next_reader_id = 1;
        self.connections.clear();
        self.readers.clear();
        self.busy_connections.clear();
        self.opening.clear();
    }
}

pub(crate) fn normalize_sql_name(name: &str) -> Option<String> {
    valid_ascii_token(name, false).then(|| name.to_ascii_lowercase())
}

pub(crate) fn validate_sql_table_name(name: &str) -> bool {
    valid_ascii_token(name, true)
}

fn valid_ascii_token(value: &str, identifier_start: bool) -> bool {
    if value.is_empty() || value.len() > 64 || !value.is_ascii() {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if identifier_start && index == 0 {
            byte.is_ascii_alphabetic() || byte == b'_'
        } else if identifier_start {
            byte.is_ascii_alphanumeric() || byte == b'_'
        } else {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
        }
    })
}

pub(crate) fn parse_resource_connection_string(value: &str) -> Option<String> {
    let path = value.strip_prefix("Data Source=")?;
    if path.is_empty()
        || path.contains(';')
        || path.starts_with("file:")
        || path.contains(':')
        || path.starts_with('/')
        || path.starts_with('\\')
    {
        return None;
    }
    era_runtime_protocol::validate_relative_path(path).ok()
}

pub(crate) fn database_source_resource(
    identity: &SqlDatabaseIdentityV1,
) -> Option<&era_runtime_protocol::SqlResourceSeedV1> {
    match &identity.source {
        SqlDatabaseSourceV1::Memory => None,
        SqlDatabaseSourceV1::ResourceSeed(seed) => Some(seed),
    }
}

pub(crate) fn opening_source(identity: &SqlDatabaseIdentityV1) -> SqlOpeningSource {
    match &identity.source {
        SqlDatabaseSourceV1::Memory => SqlOpeningSource::Memory,
        SqlDatabaseSourceV1::ResourceSeed(seed) => {
            SqlOpeningSource::Resource(seed.resource_id.to_ascii_lowercase())
        }
    }
}

#[cfg(test)]
mod tests {
    use era_protocol::ProtocolBytes;
    use era_runtime_protocol::{
        SQL_DATABASE_FORMAT_VERSION, SQL_SQLITE_VERSION, SqlDatabaseSourceV1,
    };

    use super::*;

    fn memory_identity() -> SqlDatabaseIdentityV1 {
        SqlDatabaseIdentityV1 {
            source: SqlDatabaseSourceV1::Memory,
            sqlite_version: SQL_SQLITE_VERSION.into(),
            format_version: SQL_DATABASE_FORMAT_VERSION,
        }
    }

    fn revision(byte: u8) -> SqlRevisionV1 {
        SqlRevisionV1 {
            sha256: ProtocolBytes::new(vec![byte; 32]),
        }
    }

    fn insert_connection(
        state: &mut SqlRuntimeState,
        name: &str,
        durable_revision: Option<SqlRevisionV1>,
    ) -> SqlConnectionHandleV1 {
        let handle = state
            .allocate_connection_handle()
            .expect("test connection limit is available");
        assert!(state.insert_connection(SqlConnection {
            logical_name: name.into(),
            identity: memory_identity(),
            handle,
            resource_digest: None,
            transaction_active: false,
            durable_revision,
        }));
        handle
    }

    #[test]
    fn validates_sql_names_connection_strings_and_table_names() {
        assert_eq!(normalize_sql_name("Main.DB-1"), Some("main.db-1".into()));
        assert_eq!(normalize_sql_name("_"), Some("_".into()));
        assert!(normalize_sql_name("").is_none());
        assert!(normalize_sql_name(&"a".repeat(65)).is_none());
        assert!(normalize_sql_name("database/name").is_none());
        assert!(normalize_sql_name("data库").is_none());

        assert!(validate_sql_table_name("map"));
        assert!(validate_sql_table_name("_translation_2"));
        assert!(!validate_sql_table_name(""));
        assert!(!validate_sql_table_name("2map"));
        assert!(!validate_sql_table_name("map-name"));
        assert!(!validate_sql_table_name("map.name"));
        assert!(!validate_sql_table_name("マップ"));

        assert_eq!(
            parse_resource_connection_string("Data Source=plugins/qol_data.db"),
            Some("plugins/qol_data.db".into())
        );
        for invalid in [
            "",
            "data source=plugins/qol_data.db",
            "Data Source=",
            "Data Source=../qol_data.db",
            "Data Source=/plugins/qol_data.db",
            "Data Source=\\plugins\\qol_data.db",
            "Data Source=C:/plugins/qol_data.db",
            "Data Source=file:plugins/qol_data.db",
            "Data Source=plugins/qol_data.db;Mode=ReadOnly",
        ] {
            assert_eq!(parse_resource_connection_string(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn snapshot_reports_reader_transaction_inflight_and_revision_blockers() {
        let mut stable = SqlRuntimeState::default();
        insert_connection(&mut stable, "Main", Some(revision(1)));
        let snapshot = stable.snapshot().expect("stable SQL state is snapshotable");
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].logical_name, "Main");
        assert_eq!(snapshot.connections[0].durable_revision, revision(1));

        let mut busy = stable.clone();
        assert!(busy.reserve_connection("main"));
        assert_eq!(busy.snapshot(), Err(SqlSnapshotBlocker::Inflight));

        let mut opening = stable.clone();
        opening
            .reserve_open("other".into(), SqlOpeningSource::Memory)
            .unwrap();
        assert_eq!(opening.snapshot(), Err(SqlSnapshotBlocker::Inflight));

        let mut reading = stable.clone();
        let reader_id = reading
            .allocate_reader_id()
            .expect("test reader limit is available");
        assert!(reading.insert_reader(
            reader_id,
            SqlReader {
                connection: "main".into(),
                handle: SqlReaderHandleV1 {
                    service_epoch: reading.service_epoch(),
                    id: 7,
                },
                status: SqlReaderStatusV1::BeforeFirst,
                rows_read: 0,
            },
        ));
        assert_eq!(reading.snapshot(), Err(SqlSnapshotBlocker::Reader));

        let mut transaction = stable.clone();
        transaction
            .connection_mut("main")
            .expect("test connection exists")
            .transaction_active = true;
        assert_eq!(transaction.snapshot(), Err(SqlSnapshotBlocker::Transaction));

        let mut revision_missing = SqlRuntimeState::default();
        insert_connection(&mut revision_missing, "main", None);
        assert_eq!(
            revision_missing.snapshot(),
            Err(SqlSnapshotBlocker::RevisionMissing)
        );
    }

    #[test]
    fn project_boundary_invalidates_epochs_and_clears_transient_state() {
        let mut state = SqlRuntimeState::default();
        let old_provider = state.provider();
        let old_connection = insert_connection(&mut state, "main", Some(revision(2)));
        let reader_id = state
            .allocate_reader_id()
            .expect("test reader limit is available");
        let old_reader = SqlReaderHandleV1 {
            service_epoch: state.service_epoch(),
            id: 9,
        };
        assert!(state.insert_reader(
            reader_id,
            SqlReader {
                connection: "main".into(),
                handle: old_reader,
                status: SqlReaderStatusV1::Row,
                rows_read: 1,
            },
        ));
        assert!(state.reserve_connection("main"));
        state
            .reserve_open("other".into(), SqlOpeningSource::Memory)
            .unwrap();

        state.reset_for_project_boundary();

        assert_ne!(state.provider().service_epoch, old_provider.service_epoch);
        assert_ne!(state.service_epoch(), old_connection.service_epoch);
        assert_ne!(state.service_epoch(), old_reader.service_epoch);
        assert!(state.connection_by_key("main").is_none());
        assert!(state.reader(reader_id).is_none());
        assert!(!state.has_inflight());
        assert!(!state.has_active_readers());
        assert!(!state.has_active_transactions());

        let new_connection = state
            .allocate_connection_handle()
            .expect("connection allocator restarts after a project boundary");
        assert_eq!(new_connection.id, 1);
        assert_eq!(new_connection.service_epoch, state.service_epoch());
        assert_ne!(new_connection, old_connection);
        assert_eq!(state.allocate_reader_id(), Some(1));
    }

    #[test]
    fn opening_reservations_are_named_exact_and_count_toward_the_limit() {
        let mut state = SqlRuntimeState::default();
        let first = state
            .reserve_open("db".into(), SqlOpeningSource::Memory)
            .unwrap();
        assert_eq!(
            state.reserve_open("db".into(), SqlOpeningSource::Memory),
            Err(SqlOpenReservationError::Duplicate)
        );
        assert!(!state.release_open(
            "db",
            SqlConnectionHandleV1 {
                id: first.id + 1,
                ..first
            }
        ));
        assert!(state.opening_matches("db", &SqlOpeningSource::Memory, first));

        for index in 1..era_runtime_protocol::SqlLimitsV1::FIXED.maximum_connections {
            state
                .reserve_open(format!("db{index}"), SqlOpeningSource::Memory)
                .unwrap();
        }
        assert_eq!(
            state.reserve_open("overflow".into(), SqlOpeningSource::Memory),
            Err(SqlOpenReservationError::Limit)
        );
        assert!(state.release_open("db", first));
        assert!(!state.opening_matches("db", &SqlOpeningSource::Memory, first));
    }
}
