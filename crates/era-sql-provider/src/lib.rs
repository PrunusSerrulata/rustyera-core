//! Optional native SQL service. Runtime and VM remain independent of database backends.

mod actor;
#[allow(unsafe_code)]
mod cursor;
mod engine;
mod identity;
mod policy;
mod revision;
mod values;

use era_runtime_protocol::{SqlErrorCodeV1, SqlErrorContextV1, StorageRequest, StorageResponse};

pub use actor::{CancellationHandle, NativeSqlProvider};

/// At most one live provider and one detached restore candidate may coexist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderRole {
    Live,
    Candidate,
}
pub use revision::{OpenMaterial, RevisionStore, SqlChain};

impl From<rusqlite::Error> for ProviderError {
    fn from(error: rusqlite::Error) -> Self {
        let mut result = Self::new(SqlErrorCodeV1::Sqlite, error.to_string());
        if matches!(error, rusqlite::Error::InvalidColumnIndex(_)) {
            result.code = SqlErrorCodeV1::ColumnOutOfRange;
        }
        if let rusqlite::Error::SqliteFailure(failure, _) = error {
            result.sqlite_code = Some(failure.extended_code);
            result.code = match failure.code {
                rusqlite::ErrorCode::OperationInterrupted => SqlErrorCodeV1::ExecutionTimeout,
                rusqlite::ErrorCode::TooBig => SqlErrorCodeV1::CellTooLarge,
                rusqlite::ErrorCode::DiskFull => SqlErrorCodeV1::DatabaseTooLarge,
                _ => SqlErrorCodeV1::Sqlite,
            };
        }
        result
    }
}

/// A host-owned, project-scoped storage boundary; never an arbitrary filesystem path.
pub trait SqlStorage {
    fn handle(&mut self, request: StorageRequest) -> StorageResponse;
}

impl<F: FnMut(StorageRequest) -> StorageResponse> SqlStorage for F {
    fn handle(&mut self, request: StorageRequest) -> StorageResponse {
        self(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    NotCommitted,
    Committed,
    Unknown,
}

#[derive(Debug)]
pub struct ProviderError {
    pub code: SqlErrorCodeV1,
    pub message: String,
    pub context: Vec<SqlErrorContextV1>,
    pub sqlite_code: Option<i32>,
    pub commit_outcome: CommitOutcome,
}

impl ProviderError {
    #[must_use]
    pub fn new(code: SqlErrorCodeV1, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            context: Vec::new(),
            sqlite_code: None,
            commit_outcome: CommitOutcome::NotCommitted,
        }
    }
}

pub type Result<T> = std::result::Result<T, ProviderError>;
