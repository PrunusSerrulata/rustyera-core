//! Immutable SQL blobs and host-owned optimistic current pointers.
use crate::{CommitOutcome, ProviderError, Result, SqlStorage};
use era_protocol::ProtocolBytes;
use era_runtime_protocol::{
    FrontendIoErrorKind, SQL_DATABASE_FORMAT_VERSION, SQL_SQLITE_VERSION, SqlDatabaseIdentityV1,
    SqlDatabaseSourceV1, SqlErrorCodeV1, SqlLimitsV1, SqlOpenRevisionV1, SqlRevisionV1,
    StorageNamespace, StorageOperation, StoragePrecondition, StorageRequest, StorageResult,
};
use sha2::{Digest, Sha256};
use std::fmt::Write;

fn validate_open(
    identity: &SqlDatabaseIdentityV1,
    logical_name: &str,
    revision: &SqlOpenRevisionV1,
) -> Result<()> {
    if identity.sqlite_version != SQL_SQLITE_VERSION
        || identity.format_version != SQL_DATABASE_FORMAT_VERSION
    {
        return Err(error(
            SqlErrorCodeV1::Unsupported,
            "unsupported SQL identity",
        ));
    }
    if logical_name.is_empty()
        || logical_name.len() > 64
        || !logical_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(error(
            SqlErrorCodeV1::InvalidName,
            "invalid SQL logical name",
        ));
    }
    if let SqlOpenRevisionV1::Exact(value) = revision {
        revision_hex(value)?;
    }
    Ok(())
}

const ANCHOR: &str = "3.53.0\0";
const MAX_BYTES: u64 = SqlLimitsV1::FIXED.maximum_database_bytes;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChainKind {
    Memory,
    Resource,
}

#[derive(Clone, Debug)]
pub struct SqlChain {
    pub kind: ChainKind,
    pub identity: String,
    pub current_database_revision: Option<SqlRevisionV1>,
    pub current_storage_revision: Option<String>,
}

#[derive(Debug)]
pub struct OpenMaterial {
    pub bytes: Option<Vec<u8>>,
    pub durable_revision: Option<SqlRevisionV1>,
    pub chain: SqlChain,
}

#[derive(Default)]
pub struct RevisionStore {
    next_request: u64,
}

impl RevisionStore {
    /// Open a chain without changing any existing immutable revision.
    ///
    /// # Errors
    /// Rejects invalid identities, corrupt/missing blobs and failed storage operations.
    pub fn open(
        &mut self,
        identity: &SqlDatabaseIdentityV1,
        logical_name: &str,
        revision: &SqlOpenRevisionV1,
        io: &mut dyn SqlStorage,
        mut validate_seed: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<OpenMaterial> {
        validate_open(identity, logical_name, revision)?;
        let (key, seed) = match &identity.source {
            SqlDatabaseSourceV1::Memory => (
                identity_hex(
                    "rustyera.sql.memory.v1\0",
                    &logical_name.to_ascii_lowercase(),
                    &[],
                )?,
                None,
            ),
            SqlDatabaseSourceV1::ResourceSeed(source) => {
                let bytes = self.read_seed(source, io)?;
                validate_seed(&bytes)?;
                (
                    identity_hex(
                        "rustyera.sql.identity.v1\0",
                        &source.resource_id,
                        source.sha256.as_slice(),
                    )?,
                    Some(bytes),
                )
            }
        };
        let mut chain = SqlChain {
            kind: if seed.is_some() {
                ChainKind::Resource
            } else {
                ChainKind::Memory
            },
            identity: key,
            current_database_revision: None,
            current_storage_revision: None,
        };
        if chain.kind == ChainKind::Resource
            && let Some((database, storage)) = self.read_current(&chain.identity, io)?
        {
            chain.current_database_revision = Some(database);
            chain.current_storage_revision = Some(storage);
        }
        if let SqlOpenRevisionV1::Exact(value) = revision {
            let bytes = self.read_blob(&chain.identity, value, io)?;
            if chain.current_database_revision.is_none() {
                chain.current_database_revision = Some(value.clone());
            }
            return Ok(OpenMaterial {
                bytes: Some(bytes),
                durable_revision: Some(value.clone()),
                chain,
            });
        }
        if let Some(value) = &chain.current_database_revision {
            let bytes = self.read_blob(&chain.identity, value, io)?;
            return Ok(OpenMaterial {
                bytes: Some(bytes),
                durable_revision: Some(value.clone()),
                chain,
            });
        }
        if let Some(bytes) = seed {
            let value = hash(&bytes);
            // A competing initializer with the identical seed is equivalent, but
            // never silently switch to an unrelated concurrently published revision.
            if let Err(failure) = self.publish(&mut chain, None, &bytes, &value, io) {
                if failure.code != SqlErrorCodeV1::RevisionConflict {
                    return Err(failure);
                }
                let Some((actual, storage)) = self.read_current(&chain.identity, io)? else {
                    return Err(failure);
                };
                if actual != value {
                    return Err(failure);
                }
                chain.current_database_revision = Some(actual);
                chain.current_storage_revision = Some(storage);
            }
            return Ok(OpenMaterial {
                bytes: Some(bytes),
                durable_revision: Some(value),
                chain,
            });
        }
        Ok(OpenMaterial {
            bytes: None,
            durable_revision: None,
            chain,
        })
    }

    fn read_seed(
        &mut self,
        source: &era_runtime_protocol::SqlResourceSeedV1,
        io: &mut dyn SqlStorage,
    ) -> Result<Vec<u8>> {
        safe_resource(&source.resource_id)?;
        if source.sha256.as_slice().len() != 32 {
            return Err(error(SqlErrorCodeV1::InvalidSource, "invalid seed digest"));
        }
        let result = self.call(
            StorageNamespace::Resource,
            &source.resource_id,
            StorageOperation::Read,
            io,
        )?;
        let StorageResult::Read { data, .. } = result else {
            return Err(error(SqlErrorCodeV1::InvalidSource, "cannot read SQL seed"));
        };
        let bytes = data.into_inner();
        check_size(bytes.len())?;
        if hash(&bytes).sha256 != source.sha256 {
            return Err(error(SqlErrorCodeV1::InvalidSource, "seed digest mismatch"));
        }
        Ok(bytes)
    }

    /// Publish an immutable blob, then CAS the resource pointer (memory has none).
    ///
    /// # Errors
    /// `commit_outcome` distinguishes rollback-safe rejection from an acknowledged
    /// commit or an indeterminate acknowledgement that readback could not resolve.
    pub fn publish(
        &mut self,
        chain: &mut SqlChain,
        expected: Option<&SqlRevisionV1>,
        bytes: &[u8],
        revision: &SqlRevisionV1,
        io: &mut dyn SqlStorage,
    ) -> Result<()> {
        require_hex(&chain.identity)?;
        check_size(bytes.len())?;
        if let Some(value) = expected {
            revision_hex(value)?;
        }
        if chain.current_database_revision.as_ref() != expected {
            return Err(error(
                SqlErrorCodeV1::RevisionConflict,
                "SQL current revision changed",
            ));
        }
        let digest = revision_hex(revision)?;
        if hash(bytes) != *revision {
            return Err(error(
                SqlErrorCodeV1::InvalidState,
                "publication digest mismatch",
            ));
        }
        self.quota(&chain.identity, &digest, bytes.len(), io)?;
        self.write_blob(&chain.identity, revision, bytes, io)?;
        if chain.kind == ChainKind::Memory {
            chain.current_database_revision = Some(revision.clone());
            return Ok(());
        }
        let condition = chain
            .current_storage_revision
            .clone()
            .map_or(StoragePrecondition::Missing, StoragePrecondition::Revision);
        let result = self.call(
            StorageNamespace::Data,
            &current_path(&chain.identity),
            write(format!("{digest}\n").into_bytes(), condition),
            io,
        );
        match result {
            Ok(StorageResult::Written { revision: storage }) => {
                chain.current_database_revision = Some(revision.clone());
                chain.current_storage_revision = None;
                if let Some(storage) = storage.filter(|s| !s.is_empty()) {
                    chain.current_storage_revision = Some(storage);
                    return Ok(());
                }
                self.resolve_ack(chain, revision, CommitOutcome::Committed, io)
            }
            Ok(StorageResult::Error { error: failure })
                if failure.kind == FrontendIoErrorKind::Conflict =>
            {
                Err(error(
                    SqlErrorCodeV1::RevisionConflict,
                    "SQL pointer CAS rejected",
                ))
            }
            Ok(StorageResult::Error { error: failure })
                if matches!(
                    failure.kind,
                    FrontendIoErrorKind::PermissionDenied
                        | FrontendIoErrorKind::ReadOnly
                        | FrontendIoErrorKind::InvalidData
                        | FrontendIoErrorKind::NotFound
                        | FrontendIoErrorKind::AlreadyExists
                ) =>
            {
                Err(error(
                    SqlErrorCodeV1::StorageFailure,
                    "SQL pointer write rejected",
                ))
            }
            _ => self.resolve_ack(chain, revision, CommitOutcome::Unknown, io),
        }
    }

    fn resolve_ack(
        &mut self,
        chain: &mut SqlChain,
        intended: &SqlRevisionV1,
        outcome: CommitOutcome,
        io: &mut dyn SqlStorage,
    ) -> Result<()> {
        if let Ok(Some((actual, storage))) = self.read_current(&chain.identity, io)
            && actual == *intended
        {
            chain.current_database_revision = Some(actual);
            chain.current_storage_revision = Some(storage);
            return Ok(());
        }
        // A later writer can replace even a successfully committed pointer. A
        // different readback therefore cannot prove that our CAS never committed.
        let mut failure = error(
            SqlErrorCodeV1::StorageFailure,
            "SQL publication acknowledgement unresolved",
        );
        failure.commit_outcome = outcome;
        Err(failure)
    }

    fn call(
        &mut self,
        namespace: StorageNamespace,
        path: &str,
        operation: StorageOperation,
        io: &mut dyn SqlStorage,
    ) -> Result<StorageResult> {
        self.next_request = self.next_request.checked_add(1).ok_or_else(|| {
            error(
                SqlErrorCodeV1::StorageFailure,
                "storage request IDs exhausted",
            )
        })?;
        let request_id = self.next_request;
        let response = io.handle(StorageRequest {
            request_id,
            namespace,
            relative_path: path.into(),
            operation,
            // No automatic retries: an empty key disables host ACK caching. Request
            // IDs restart for each store and must not identify writes across stores.
            idempotency_key: String::new(),
            deadline_ns: None,
        });
        if response.request_id != request_id {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "storage response ID mismatch",
            ));
        }
        Ok(response.result)
    }

    fn read_current(
        &mut self,
        key: &str,
        io: &mut dyn SqlStorage,
    ) -> Result<Option<(SqlRevisionV1, String)>> {
        let result = self.call(
            StorageNamespace::Data,
            &current_path(key),
            StorageOperation::Read,
            io,
        )?;
        if is_error(&result, FrontendIoErrorKind::NotFound) {
            return Ok(None);
        }
        let StorageResult::Read {
            data,
            revision: Some(storage),
        } = result
        else {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "cannot read SQL pointer identity",
            ));
        };
        let bytes = data.as_slice();
        if bytes.len() != 65 || bytes[64] != b'\n' || storage.is_empty() {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "malformed SQL pointer",
            ));
        }
        let text = std::str::from_utf8(&bytes[..64])
            .map_err(|_| error(SqlErrorCodeV1::StorageFailure, "malformed SQL pointer"))?;
        let value = parse_revision(text)?;
        Ok(Some((value, storage)))
    }

    fn read_blob(
        &mut self,
        key: &str,
        revision: &SqlRevisionV1,
        io: &mut dyn SqlStorage,
    ) -> Result<Vec<u8>> {
        let result = self.call(
            StorageNamespace::Data,
            &blob_path(key, &revision_hex(revision)?),
            StorageOperation::Read,
            io,
        )?;
        if is_error(&result, FrontendIoErrorKind::NotFound) {
            return Err(error(
                SqlErrorCodeV1::RevisionMissing,
                "SQL revision missing",
            ));
        }
        let StorageResult::Read { data, .. } = result else {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "cannot read SQL blob",
            ));
        };
        if data.as_slice().len() as u64 > MAX_BYTES || hash(data.as_slice()) != *revision {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "SQL revision corrupt",
            ));
        }
        Ok(data.into_inner())
    }

    fn write_blob(
        &mut self,
        key: &str,
        revision: &SqlRevisionV1,
        bytes: &[u8],
        io: &mut dyn SqlStorage,
    ) -> Result<()> {
        let result = self.call(
            StorageNamespace::Data,
            &blob_path(key, &revision_hex(revision)?),
            write(bytes.to_vec(), StoragePrecondition::Missing),
            io,
        );
        if !matches!(result, Ok(StorageResult::Written { .. })) {
            // A lost ACK is recoverable only after verifying the immutable blob's digest.
            self.read_blob(key, revision, io)?;
        }
        Ok(())
    }

    fn quota(
        &mut self,
        key: &str,
        revision: &str,
        size: usize,
        io: &mut dyn SqlStorage,
    ) -> Result<()> {
        let result = self.call(
            StorageNamespace::Data,
            &format!("sql/v1/{key}/revisions"),
            StorageOperation::List {
                pattern: None,
                recursive: false,
            },
            io,
        )?;
        if is_error(&result, FrontendIoErrorKind::NotFound) {
            return Ok(());
        }
        let StorageResult::Listed { entries } = result else {
            return Err(error(
                SqlErrorCodeV1::StorageFailure,
                "cannot inspect SQL quota",
            ));
        };
        let mut total = 0_u64;
        let mut present = false;
        for entry in entries {
            total = total
                .checked_add(entry.byte_length)
                .ok_or_else(|| error(SqlErrorCodeV1::DatabaseTooLarge, "SQL quota overflow"))?;
            present |= entry.relative_path == format!("{revision}.sqlite3")
                || entry.relative_path == blob_path(key, revision);
        }
        if !present {
            total = total.saturating_add(size as u64);
        }
        if total > MAX_BYTES {
            return Err(error(
                SqlErrorCodeV1::DatabaseTooLarge,
                "SQL chain quota exceeded",
            ));
        }
        Ok(())
    }
}

fn error(code: SqlErrorCodeV1, message: &str) -> ProviderError {
    ProviderError::new(code, message)
}
fn hash(bytes: &[u8]) -> SqlRevisionV1 {
    SqlRevisionV1 {
        sha256: ProtocolBytes::new(Sha256::digest(bytes).to_vec()),
    }
}
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
fn revision_hex(revision: &SqlRevisionV1) -> Result<String> {
    if revision.sha256.as_slice().len() != 32 {
        return Err(error(
            SqlErrorCodeV1::InvalidState,
            "invalid SQL revision digest",
        ));
    }
    Ok(hex(revision.sha256.as_slice()))
}
fn require_hex(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            SqlErrorCodeV1::StorageFailure,
            "invalid canonical SQL digest",
        ));
    }
    Ok(())
}
fn parse_revision(value: &str) -> Result<SqlRevisionV1> {
    require_hex(value)?;
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
            digit(pair[0]) * 16 + digit(pair[1])
        })
        .collect::<Vec<_>>();
    Ok(SqlRevisionV1 {
        sha256: ProtocolBytes::new(bytes),
    })
}
fn identity_hex(prefix: &str, name: &str, seed: &[u8]) -> Result<String> {
    let length = u32::try_from(name.len())
        .map_err(|_| error(SqlErrorCodeV1::InvalidSource, "SQL identity too long"))?;
    let mut digest = Sha256::new();
    digest.update(prefix.as_bytes());
    digest.update(length.to_be_bytes());
    digest.update(name.as_bytes());
    digest.update(seed);
    digest.update(ANCHOR.as_bytes());
    digest.update(SQL_DATABASE_FORMAT_VERSION.to_be_bytes());
    Ok(hex(&digest.finalize()))
}
fn safe_resource(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', '\0', ':'])
        || !unicode_normalization::is_nfc(path)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(error(
            SqlErrorCodeV1::InvalidSource,
            "unsafe SQL resource path",
        ));
    }
    Ok(())
}
fn check_size(size: usize) -> Result<()> {
    if size as u64 > MAX_BYTES {
        return Err(error(
            SqlErrorCodeV1::DatabaseTooLarge,
            "SQL database too large",
        ));
    }
    Ok(())
}
fn current_path(key: &str) -> String {
    format!("sql/v1/{key}/current")
}
fn blob_path(key: &str, digest: &str) -> String {
    format!("sql/v1/{key}/revisions/{digest}.sqlite3")
}
fn write(data: Vec<u8>, precondition: StoragePrecondition) -> StorageOperation {
    StorageOperation::Write {
        data: ProtocolBytes::new(data),
        atomic_replace: true,
        precondition,
    }
}
fn is_error(result: &StorageResult, kind: FrontendIoErrorKind) -> bool {
    matches!(result, StorageResult::Error { error } if error.kind == kind)
}

#[cfg(test)]
pub(crate) mod tests;
