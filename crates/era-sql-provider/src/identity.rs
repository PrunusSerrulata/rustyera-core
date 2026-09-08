//! Verify the linked engine, independently of the prebuild manifest or dependency features.

use std::collections::BTreeSet;

use era_runtime_protocol::SqlErrorCodeV1;
use rusqlite::Connection;

use crate::{ProviderError, Result};

const SOURCE_ID: &str =
    "2026-07-24 19:02:57 bf7c7f30031888f4e796e429ab3978879485813aaca6f641c7b33e4e09459bcc";
const LINK_IDENTITY: &str = env!("RUSTYERA_SQLITE_LINK_IDENTITY");
// Keep every required option aligned with tools/sqlite-native/build.mjs SQLITE_DEFINES.
const REQUIRED_OPTIONS: &[&str] = &[
    "THREADSAFE=1",
    "TEMP_STORE=3",
    "DQS=0",
    "DEFAULT_CACHE_SIZE=-16384",
    "DEFAULT_RECURSIVE_TRIGGERS",
    "DEFAULT_AUTOVACUUM",
    "DEFAULT_SECTOR_SIZE=4096",
    "MAX_MMAP_SIZE=0",
    "MAX_WORKER_THREADS=0",
    "ENABLE_API_ARMOR",
    "ENABLE_COLUMN_METADATA",
    "ENABLE_MATH_FUNCTIONS",
    "ENABLE_FTS5",
    "ENABLE_RTREE",
    "ENABLE_SESSION",
    "ENABLE_PREUPDATE_HOOK",
    "ENABLE_PERCENTILE",
    "ENABLE_OFFSET_SQL_FUNC",
    "ENABLE_DBSTAT_VTAB",
    "ENABLE_DBPAGE_VTAB",
    "ENABLE_BYTECODE_VTAB",
    "ENABLE_STMTVTAB",
    "ENABLE_UNKNOWN_SQL_FUNCTION",
    "USE_URI",
    "OMIT_LOAD_EXTENSION",
    "OMIT_SHARED_CACHE",
    "OMIT_DEPRECATED",
    "OMIT_UTF16",
];

/// Called by connection policy initialization before accepting any project SQL.
/// A private in-memory connection avoids authorizers or user-defined function overrides.
pub(crate) fn verify() -> Result<()> {
    let connection = Connection::open_in_memory().map_err(query_error)?;
    let (version, source): (String, String) = connection
        .query_row("SELECT sqlite_version(), sqlite_source_id()", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(query_error)?;
    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .map_err(query_error)?;
    let options = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(query_error)?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(query_error)?;
    require(&version, &source, &options)
}

fn require(version: &str, source: &str, options: &BTreeSet<String>) -> Result<()> {
    let missing: Vec<_> = REQUIRED_OPTIONS
        .iter()
        .copied()
        .filter(|option| !options.contains(*option))
        .collect();
    let forbidden: Vec<_> = options
        .iter()
        .filter(|option| {
            let name = option.split('=').next().unwrap_or_default();
            matches!(
                name,
                "OMIT_DESERIALIZE"
                    | "DEFAULT_FOREIGN_KEYS"
                    | "HAS_CODEC"
                    | "OMIT_COMPILEOPTION_DIAGS"
            )
        })
        .collect();
    if version != "3.53.4" || source != SOURCE_ID || !missing.is_empty() || !forbidden.is_empty() {
        return Err(ProviderError::new(
            SqlErrorCodeV1::Unsupported,
            format!(
                "SQLite engine identity mismatch: version={version}, source={source}, missing={missing:?}, forbidden={forbidden:?}, prebuild={LINK_IDENTITY}; dependency feature merging must not select bundled/SQLCipher/extension engines"
            ),
        ));
    }
    Ok(())
}

fn query_error(error: rusqlite::Error) -> ProviderError {
    let mut result: ProviderError = error.into();
    result.code = SqlErrorCodeV1::Unsupported;
    result.message = format!(
        "SQLite identity query failed (prebuild={LINK_IDENTITY}): {}",
        result.message
    );
    result
}

#[cfg(test)]
mod tests;
