//! Versioned safe-SQL service payloads.
//!
//! These records expose only project-scoped resource identities and logical handles. They never
//! carry operating-system paths, connection strings after validation, or provider-native handles.

use era_protocol::{ProtocolBytes, ProtocolVersion};
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

pub const SQL_OPERATION: &str = erabasic_compat::SQL_SERVICE_CONTRACT_NAME;
pub const SQL_OPERATION_VERSION: ProtocolVersion =
    ProtocolVersion::new(erabasic_compat::SQL_SERVICE_CONTRACT_VERSION, 0);
pub const SQL_LIMITS_POLICY_VERSION: u32 = erabasic_compat::SQL_LIMITS_CONTRACT_VERSION;
pub const SQL_DATABASE_FORMAT_VERSION: u32 = 1;
pub const SQL_SQLITE_VERSION: &str = "3.53.0";

/// Fixed limits shared by every batch-3 provider.
#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlLimitsV1 {
    #[n(0)]
    pub maximum_connections: u32,
    #[n(1)]
    pub maximum_readers: u32,
    #[n(2)]
    pub maximum_sql_bytes: u32,
    #[n(3)]
    pub maximum_parameters: u32,
    #[n(4)]
    pub maximum_parameter_bytes: u64,
    #[n(5)]
    pub maximum_cell_bytes: u32,
    #[n(6)]
    pub maximum_database_bytes: u64,
    #[n(7)]
    pub maximum_map_rows: u32,
    #[n(8)]
    pub maximum_map_bytes: u64,
    #[n(9)]
    pub maximum_reader_rows: u64,
    #[n(10)]
    pub execution_budget_ms: u32,
}

impl SqlLimitsV1 {
    pub const FIXED: Self = Self {
        maximum_connections: 8,
        maximum_readers: 32,
        maximum_sql_bytes: 256 * 1024,
        maximum_parameters: 64,
        maximum_parameter_bytes: 8 * 1024 * 1024,
        maximum_cell_bytes: 1024 * 1024,
        maximum_database_bytes: 64 * 1024 * 1024,
        maximum_map_rows: 100_000,
        maximum_map_bytes: 8 * 1024 * 1024,
        maximum_reader_rows: 1_000_000,
        execution_budget_ms: 5_000,
    };
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlProviderHandleV1 {
    #[n(0)]
    pub service_epoch: u64,
    #[n(1)]
    pub id: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlConnectionHandleV1 {
    #[n(0)]
    pub service_epoch: u64,
    #[n(1)]
    pub id: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlReaderHandleV1 {
    #[n(0)]
    pub service_epoch: u64,
    #[n(1)]
    pub id: u64,
}

/// Immutable Resource seed identity. `resource_id` is a validated project-relative identifier,
/// not an operating-system path.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlResourceSeedV1 {
    #[n(0)]
    pub resource_id: String,
    #[n(1)]
    pub sha256: ProtocolBytes,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqlDatabaseSourceV1 {
    #[n(0)]
    Memory,
    #[n(1)]
    ResourceSeed(#[n(0)] SqlResourceSeedV1),
}

/// Complete immutable database-chain identity. Providers reject any `SQLite` or format version
/// other than the values fixed by the negotiated SQL operation version.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlDatabaseIdentityV1 {
    #[n(0)]
    pub source: SqlDatabaseSourceV1,
    #[n(1)]
    pub sqlite_version: String,
    #[n(2)]
    pub format_version: u32,
}

/// Content-addressed immutable database revision.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlRevisionV1 {
    #[n(0)]
    pub sha256: ProtocolBytes,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "revision", rename_all = "snake_case")]
pub enum SqlOpenRevisionV1 {
    #[n(0)]
    Current,
    #[n(1)]
    Exact(#[n(0)] SqlRevisionV1),
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SqlValueV1 {
    #[n(0)]
    Null,
    #[n(1)]
    Integer(#[n(0)] i64),
    #[n(2)]
    String(#[n(0)] String),
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SqlExecuteModeV1 {
    #[n(0)]
    NonQuery,
    #[n(1)]
    ScalarInteger,
    #[n(2)]
    ScalarString,
    #[n(3)]
    Reader,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlMapRowV1 {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqlOperationV1 {
    #[n(0)]
    Open {
        #[n(0)]
        connection: SqlConnectionHandleV1,
        #[n(1)]
        logical_name: String,
        #[n(2)]
        identity: SqlDatabaseIdentityV1,
        #[n(3)]
        revision: SqlOpenRevisionV1,
        #[n(4)]
        limits: SqlLimitsV1,
    },
    #[n(1)]
    Execute {
        #[n(0)]
        connection: SqlConnectionHandleV1,
        #[n(1)]
        mode: SqlExecuteModeV1,
        #[n(2)]
        sql: String,
        #[n(3)]
        parameters: Vec<SqlValueV1>,
    },
    #[n(2)]
    ReaderRead {
        #[n(0)]
        reader: SqlReaderHandleV1,
    },
    #[n(3)]
    ReaderGet {
        #[n(0)]
        reader: SqlReaderHandleV1,
        #[n(1)]
        column: u32,
    },
    #[n(4)]
    ReaderIsNull {
        #[n(0)]
        reader: SqlReaderHandleV1,
        #[n(1)]
        column: u32,
    },
    #[n(5)]
    ReaderClose {
        #[n(0)]
        reader: SqlReaderHandleV1,
    },
    #[n(6)]
    ImportMapRows {
        #[n(0)]
        connection: SqlConnectionHandleV1,
        #[n(1)]
        table: String,
        #[n(2)]
        rows: Vec<SqlMapRowV1>,
    },
    #[n(7)]
    Disconnect {
        #[n(0)]
        connection: SqlConnectionHandleV1,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlRequestV1 {
    #[n(0)]
    pub provider: SqlProviderHandleV1,
    #[n(1)]
    pub operation: SqlOperationV1,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SqlReaderStatusV1 {
    #[n(0)]
    BeforeFirst,
    #[n(1)]
    Row,
    #[n(2)]
    Eof,
    #[n(3)]
    Closed,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlDatabaseStateV1 {
    #[n(0)]
    pub connection: SqlConnectionHandleV1,
    #[n(1)]
    pub connected: bool,
    #[n(2)]
    pub transaction_active: bool,
    #[n(3)]
    pub durable_revision: Option<SqlRevisionV1>,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlReaderStateV1 {
    #[n(0)]
    pub reader: SqlReaderHandleV1,
    #[n(1)]
    pub status: SqlReaderStatusV1,
    #[n(2)]
    pub rows_read: u64,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SqlOperationKindV1 {
    #[n(0)]
    Open,
    #[n(1)]
    Execute,
    #[n(2)]
    ReaderRead,
    #[n(3)]
    ReaderGet,
    #[n(4)]
    ReaderIsNull,
    #[n(5)]
    ReaderClose,
    #[n(6)]
    ImportMapRows,
    #[n(7)]
    Disconnect,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum SqlErrorCodeV1 {
    #[n(0)]
    InvalidRequest,
    #[n(1)]
    InvalidName,
    #[n(2)]
    InvalidSource,
    #[n(3)]
    InvalidConnectionString,
    #[n(4)]
    ConnectionLimit,
    #[n(5)]
    ConnectionConflict,
    #[n(6)]
    ConnectionNotFound,
    #[n(7)]
    ReaderLimit,
    #[n(8)]
    ReaderNotFound,
    #[n(9)]
    ColumnOutOfRange,
    #[n(10)]
    TypeMismatch,
    #[n(11)]
    SqlTooLarge,
    #[n(12)]
    ParameterLimit,
    #[n(13)]
    ParameterBytesLimit,
    #[n(14)]
    CellTooLarge,
    #[n(15)]
    DatabaseTooLarge,
    #[n(16)]
    MapRowLimit,
    #[n(17)]
    MapBytesLimit,
    #[n(18)]
    ReaderRowLimit,
    #[n(19)]
    ExecutionTimeout,
    #[n(20)]
    TransactionActive,
    #[n(21)]
    RevisionConflict,
    #[n(22)]
    RevisionMissing,
    #[n(23)]
    StorageFailure,
    #[n(24)]
    Sqlite,
    #[n(25)]
    Cancelled,
    #[n(26)]
    StaleEpoch,
    #[n(27)]
    InvalidTableName,
    #[n(28)]
    InvalidState,
    #[n(29)]
    Unsupported,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlErrorContextV1 {
    #[n(0)]
    pub key: String,
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlErrorV1 {
    #[n(0)]
    pub code: SqlErrorCodeV1,
    #[n(1)]
    pub operation: SqlOperationKindV1,
    #[n(2)]
    pub context: Vec<SqlErrorContextV1>,
    #[n(3)]
    pub sqlite_code: Option<i32>,
    /// Diagnostic-only provider text. Callers must never classify errors from this field.
    #[n(4)]
    pub sqlite_message: Option<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SqlResultV1 {
    #[n(0)]
    Opened {
        #[n(0)]
        sqlite_version: String,
        #[n(1)]
        limits: SqlLimitsV1,
    },
    #[n(1)]
    NonQuery {
        #[n(0)]
        affected_rows: i64,
    },
    #[n(2)]
    Scalar {
        #[n(0)]
        value: SqlValueV1,
    },
    #[n(3)]
    ReaderOpened {
        #[n(0)]
        reader: SqlReaderHandleV1,
    },
    #[n(4)]
    ReaderAdvanced {
        #[n(0)]
        has_row: bool,
    },
    #[n(5)]
    ReaderValue {
        #[n(0)]
        value: SqlValueV1,
    },
    #[n(6)]
    ReaderNull {
        #[n(0)]
        is_null: bool,
    },
    #[n(7)]
    ReaderClosed,
    #[n(8)]
    MapImported {
        #[n(0)]
        rows: u32,
    },
    #[n(9)]
    Disconnected,
    #[n(10)]
    Error {
        #[n(0)]
        error: SqlErrorV1,
    },
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct SqlResponseV1 {
    #[n(0)]
    pub provider: SqlProviderHandleV1,
    #[n(1)]
    pub database: Option<SqlDatabaseStateV1>,
    #[n(2)]
    pub reader: Option<SqlReaderStateV1>,
    #[n(3)]
    pub result: SqlResultV1,
}
