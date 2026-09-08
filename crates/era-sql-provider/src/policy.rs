//! Connection-local SQL policy. Callers own the connection and all storage publication.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use era_runtime_protocol::{SQL_SQLITE_VERSION, SqlErrorCodeV1, SqlLimitsV1, SqlValueV1};
use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;

use crate::{ProviderError, Result};

#[cfg(test)]
mod tests;

const LIMITS: SqlLimitsV1 = SqlLimitsV1::FIXED;

/// Shared owner lifetime cancellation and the current transport deadline. Cancellation
/// is terminal: neither a new request nor a SQL scope can reset it.
#[derive(Clone, Default)]
pub(crate) struct Control {
    cancelled: Arc<AtomicBool>,
    deadline: Arc<Mutex<Option<Instant>>>,
}

impl Control {
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn expired(&self) -> bool {
        if self.cancelled.load(Ordering::Acquire) {
            return true;
        }
        if self.deadline.lock().map_or(true, |deadline| {
            deadline.is_some_and(|deadline| Instant::now() >= deadline)
        }) {
            self.cancel();
        }
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn checkpoint(&self) -> Result<()> {
        if self.expired() {
            Err(ProviderError::new(
                SqlErrorCodeV1::ExecutionTimeout,
                "native SQL owner cancelled or transport deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn begin_request(&self, deadline: Instant) -> Result<()> {
        self.checkpoint()?;
        let mut current = self.deadline.lock().map_err(|_| {
            ProviderError::new(SqlErrorCodeV1::InvalidState, "SQL control state poisoned")
        })?;
        if current.is_some() {
            return Err(ProviderError::new(
                SqlErrorCodeV1::InvalidState,
                "SQL request already active",
            ));
        }
        *current = Some(deadline);
        drop(current);
        self.checkpoint()
    }

    pub(crate) fn finish_request(&self) -> Result<()> {
        self.checkpoint()?;
        let mut deadline = self.deadline.lock().map_err(|_| {
            ProviderError::new(SqlErrorCodeV1::InvalidState, "SQL control state poisoned")
        })?;
        // Do not erase a deadline that expired between checkpoint and taking the lock.
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.cancel();
        }
        *deadline = None;
        drop(deadline);
        self.checkpoint()
    }
}

#[derive(Default)]
struct OperationState {
    control: Control,
    virtual_tables: BTreeSet<String>,
    deadline: Option<Instant>,
    allow_vacuum_attach: bool,
    timed_out: bool,
    scalar_proof: Option<ScalarProof>,
}

struct ScalarProof {
    ordinary_tables: Arc<BTreeSet<String>>,
    valid: bool,
    observed_select: bool,
}

/// Owns the state captured by the connection's single authorizer and progress callbacks.
/// Install after loading a database; reinstall when replacing its connection or image.
#[derive(Clone)]
pub(crate) struct Policy {
    state: Arc<Mutex<OperationState>>,
}

/// Owns callback state only; it never borrows a connection or a database container.
#[must_use]
pub(crate) struct Scope {
    policy: Policy,
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.policy.finish();
    }
}

impl Policy {
    pub(crate) fn install(connection: &Connection, control: Control) -> Result<Self> {
        // Bound initialization, including untrusted seed schema decoding, before any
        // SQL is prepared. Inventory never selects from a virtual table.
        let state = Arc::new(Mutex::new(OperationState {
            control,
            ..OperationState::default()
        }));
        let progress_state = Arc::clone(&state);
        connection
            .progress_handler(1_000, Some(move || expired(&progress_state)))
            .map_err(sqlite_error)?;
        let policy = Self { state };
        let initialization_scope = policy.scope("")?;
        crate::identity::verify()?;
        require_engine(rusqlite::version(), rusqlite::version_number())?;
        // identity::verify requires OMIT_LOAD_EXTENSION; the engine has no loadable
        // extension entrypoint. Do not enable rusqlite's load_extension feature for a flag.
        connection
            .busy_timeout(Duration::ZERO)
            .map_err(sqlite_error)?;
        // SQLITE_LIMIT_LENGTH also limits entire rows, not individual result cells. Keep
        // the established database-sized engine limit; check_cell enforces wire cell sizes.
        for (limit, value) in [
            (
                Limit::SQLITE_LIMIT_SQL_LENGTH,
                i64::from(LIMITS.maximum_sql_bytes),
            ),
            (
                Limit::SQLITE_LIMIT_VARIABLE_NUMBER,
                i64::from(LIMITS.maximum_parameters),
            ),
            (Limit::SQLITE_LIMIT_LENGTH, 64 * 1024 * 1024),
        ] {
            let value = i32::try_from(value).map_err(|_| {
                ProviderError::new(
                    SqlErrorCodeV1::InvalidState,
                    "SQL policy limit exceeds SQLite range",
                )
            })?;
            connection.set_limit(limit, value).map_err(sqlite_error)?;
        }
        connection
            .execute_batch(
                "PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=MEMORY;",
            )
            .map_err(|error| policy.error(error))?;
        let page_size: i64 = connection
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .map_err(|error| policy.error(error))?;
        let page_size = u64::try_from(page_size)
            .ok()
            .filter(|size| *size > 0)
            .ok_or_else(|| {
                ProviderError::new(SqlErrorCodeV1::InvalidSource, "invalid SQLite page size")
            })?;
        let maximum_pages = LIMITS.maximum_database_bytes / page_size;
        if maximum_pages == 0 {
            return Err(ProviderError::new(
                SqlErrorCodeV1::DatabaseTooLarge,
                "SQLite page exceeds database budget",
            ));
        }
        let actual: u32 = connection
            .query_row(
                &format!("PRAGMA max_page_count={maximum_pages}"),
                [],
                |row| row.get(0),
            )
            .map_err(|error| policy.error(error))?;
        if u64::from(actual) > maximum_pages {
            return Err(ProviderError::new(
                SqlErrorCodeV1::DatabaseTooLarge,
                "SQLite database exceeds its limit",
            ));
        }
        let virtual_tables = inventory(connection, &policy)?;
        policy
            .state
            .lock()
            .map_err(|_| {
                ProviderError::new(SqlErrorCodeV1::InvalidState, "SQL policy state poisoned")
            })?
            .virtual_tables = virtual_tables;
        let authorizer_state = Arc::clone(&policy.state);
        connection
            .authorizer(Some(move |context: AuthContext<'_>| {
                authorize(context, &authorizer_state)
            }))
            .map_err(sqlite_error)?;
        policy.checkpoint()?;
        drop(initialization_scope);
        Ok(policy)
    }

    pub(crate) fn scope(&self, sql: &str) -> Result<Scope> {
        self.begin(sql)?;
        Ok(Scope {
            policy: self.clone(),
        })
    }

    pub(crate) fn scalar_scope(&self, sql: &str, tables: &Arc<BTreeSet<String>>) -> Result<Scope> {
        self.begin_scalar(sql, tables)?;
        Ok(Scope {
            policy: self.clone(),
        })
    }

    /// Begin one prepare/step operation. Never nest or reset this budget inside a step loop.
    /// Reader continuations use their original SQL, never a caller-supplied VACUUM marker.
    pub(crate) fn begin(&self, sql: &str) -> Result<()> {
        self.begin_operation(sql, None)
    }

    /// Prove only this scalar preparation, not a cached statement or a previous operation.
    /// Table names must be the exact names from the current main `sqlite_schema`. The caller
    /// must additionally require statement readonly and invalidate its table list on writes.
    pub(crate) fn begin_scalar(
        &self,
        sql: &str,
        ordinary_tables: &Arc<BTreeSet<String>>,
    ) -> Result<()> {
        self.begin_operation(
            sql,
            Some(ScalarProof {
                ordinary_tables: Arc::clone(ordinary_tables),
                valid: true,
                observed_select: false,
            }),
        )
    }

    fn begin_operation(&self, sql: &str, scalar_proof: Option<ScalarProof>) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| {
            ProviderError::new(SqlErrorCodeV1::InvalidState, "SQL policy state poisoned")
        })?;
        state.control.checkpoint()?;
        if state.deadline.is_some() {
            return Err(ProviderError::new(
                SqlErrorCodeV1::InvalidState,
                "SQL operation already active",
            ));
        }
        if sql.len() > LIMITS.maximum_sql_bytes as usize {
            return Err(ProviderError::new(
                SqlErrorCodeV1::SqlTooLarge,
                "SQL text exceeds its limit",
            ));
        }
        if sql.as_bytes().contains(&0) {
            return Err(ProviderError::new(
                SqlErrorCodeV1::InvalidRequest,
                "SQL text contains NUL",
            ));
        }
        state.deadline =
            Some(Instant::now() + Duration::from_millis(u64::from(LIMITS.execution_budget_ms)));
        state.allow_vacuum_attach = is_bare_vacuum(sql);
        state.timed_out = false;
        state.scalar_proof = scalar_proof;
        Ok(())
    }

    /// Must run on success and error, after translating an error with `error`.
    pub(crate) fn finish(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.deadline = None;
            state.allow_vacuum_attach = false;
            state.timed_out = false;
            state.scalar_proof = None;
        }
    }

    /// Host checkpoints bound work outside `SQLite`'s VM (binding and result conversion).
    pub(crate) fn checkpoint(&self) -> Result<()> {
        if expired(&self.state) {
            Err(ProviderError::new(
                SqlErrorCodeV1::ExecutionTimeout,
                "SQL execution budget exceeded",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn error(&self, error: rusqlite::Error) -> ProviderError {
        let timed_out = expired(&self.state)
            || self.state.lock().map_or(true, |mut state| {
                if let Some(proof) = &mut state.scalar_proof {
                    proof.valid = false;
                }
                state.timed_out
            });
        let mut mapped = sqlite_error(error);
        if timed_out {
            mapped.code = SqlErrorCodeV1::ExecutionTimeout;
        }
        mapped
    }

    /// Read before finish, after successful preparation/execution and the readonly check.
    pub(crate) fn reusable(&self) -> bool {
        if expired(&self.state) {
            return false;
        }
        self.state.lock().is_ok_and(|state| {
            !state.timed_out
                && state
                    .deadline
                    .is_some_and(|deadline| Instant::now() < deadline)
                && state
                    .scalar_proof
                    .as_ref()
                    .is_some_and(|proof| proof.valid && proof.observed_select)
        })
    }
}

pub(crate) fn check_cell(bytes: usize) -> Result<()> {
    if bytes > LIMITS.maximum_cell_bytes as usize {
        Err(ProviderError::new(
            SqlErrorCodeV1::CellTooLarge,
            "SQL cell exceeds its limit",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn check_parameters(values: &[SqlValueV1]) -> Result<()> {
    if values.len() > LIMITS.maximum_parameters as usize {
        return Err(ProviderError::new(
            SqlErrorCodeV1::ParameterLimit,
            "SQL parameter count exceeds its limit",
        ));
    }
    let mut bytes = 0_u64;
    for value in values {
        let size = match value {
            SqlValueV1::Null => 0,
            SqlValueV1::Integer(_) => 8,
            SqlValueV1::String(value) => value.len(),
        };
        check_cell(size)?;
        bytes = bytes.saturating_add(u64::try_from(size).unwrap_or(u64::MAX));
    }
    if bytes > LIMITS.maximum_parameter_bytes {
        return Err(ProviderError::new(
            SqlErrorCodeV1::ParameterBytesLimit,
            "SQL parameter bytes exceed their limit",
        ));
    }
    Ok(())
}

fn require_engine(version: &str, number: i32) -> Result<()> {
    if version != SQL_SQLITE_VERSION || number != 3_053_004 {
        return Err(ProviderError::new(
            SqlErrorCodeV1::Unsupported,
            format!("SQLite engine mismatch: {version} ({number}); expected {SQL_SQLITE_VERSION}"),
        ));
    }
    Ok(())
}

fn inventory(connection: &Connection, policy: &Policy) -> Result<BTreeSet<String>> {
    let checkpoint = || policy.checkpoint();
    // sqlite_schema is an ordinary SQLite-owned btree. Do not use table_list,
    // table_xinfo, or SELECT against seed tables: those can connect virtual modules.
    for sql in [
        "SELECT sql FROM main.sqlite_schema",
        "SELECT sql FROM temp.sqlite_schema",
    ] {
        checkpoint()?;
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| policy.error(error))?;
        let mut rows = statement.query([]).map_err(|error| policy.error(error))?;
        while let Some(row) = rows.next().map_err(|error| policy.error(error))? {
            checkpoint()?;
            if row
                .get::<_, Option<String>>(0)
                .map_err(sqlite_error)?
                .is_some_and(|sql| is_virtual_definition(&sql))
            {
                return Err(ProviderError::new(
                    SqlErrorCodeV1::Unsupported,
                    "seed database contains a virtual table",
                ));
            }
        }
    }
    // Include every registered module, including eponymous-only modules. The fixed
    // names also remain denied when a module is absent from this SQLite build.
    let mut modules: BTreeSet<String> = [
        "sqlite_dbpage",
        "sqlite_dbdata",
        "sqlite_dbptr",
        "dbstat",
        "bytecode",
        "tables_used",
        "stmt",
        "sqlite_stmt",
        "json_each",
        "json_tree",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut statement = connection
        .prepare("PRAGMA module_list")
        .map_err(|error| policy.error(error))?;
    let mut rows = statement.query([]).map_err(|error| policy.error(error))?;
    while let Some(row) = rows.next().map_err(|error| policy.error(error))? {
        checkpoint()?;
        modules.insert(
            row.get::<_, String>(0)
                .map_err(sqlite_error)?
                .to_ascii_lowercase(),
        );
    }
    checkpoint()?;
    Ok(modules)
}

fn is_virtual_definition(mut sql: &str) -> bool {
    // SQLite normalizes CREATE's prefix in stored schema SQL. Also handle whitespace
    // and comments explicitly so a crafted seed cannot hide the VIRTUAL keyword.
    for expected in ["CREATE", "VIRTUAL", "TABLE"] {
        loop {
            sql = sql.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            if let Some(comment) = sql.strip_prefix("--") {
                sql = comment.find('\n').map_or("", |end| &comment[end + 1..]);
            } else if let Some(comment) = sql.strip_prefix("/*") {
                sql = comment.find("*/").map_or("", |end| &comment[end + 2..]);
            } else {
                break;
            }
        }
        let end = sql
            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
            .unwrap_or(sql.len());
        if !sql[..end].eq_ignore_ascii_case(expected) {
            return false;
        }
        sql = &sql[end..];
    }
    true
}

fn sqlite_error(error: rusqlite::Error) -> ProviderError {
    error.into()
}

fn expired(state: &Mutex<OperationState>) -> bool {
    let Ok(mut state) = state.lock() else {
        return true;
    };
    if state.control.expired()
        || state
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    {
        state.timed_out = true;
        state.allow_vacuum_attach = false;
    }
    state.timed_out
}

fn is_bare_vacuum(sql: &str) -> bool {
    let trim = |character: char| matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c');
    let sql = sql.trim_matches(trim);
    sql.strip_suffix(';')
        .unwrap_or(sql)
        .trim_matches(trim)
        .eq_ignore_ascii_case("vacuum")
}

fn authorize(context: AuthContext<'_>, state: &Mutex<OperationState>) -> Authorization {
    use AuthAction as A;
    if expired(state) {
        return Authorization::Deny;
    }
    {
        let Ok(mut state) = state.lock() else {
            return Authorization::Deny;
        };
        if let Some(proof) = &mut state.scalar_proof {
            // Proof only narrows cache eligibility. It never changes authorization or installs
            // a second callback which could silently displace the security policy.
            let eligible = context.accessor.is_none()
                && match context.action {
                    A::Select => {
                        proof.observed_select = true;
                        true
                    }
                    A::Read { table_name, .. } => {
                        context.database_name == Some("main")
                            && proof.ordinary_tables.contains(table_name)
                    }
                    _ => false,
                };
            proof.valid &= eligible;
        }
        let table = match context.action {
            A::Read { table_name, .. }
            | A::Insert { table_name }
            | A::Update { table_name, .. }
            | A::Delete { table_name } => Some(table_name),
            _ => None,
        };
        if table.is_some_and(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("pragma_") || state.virtual_tables.contains(&name)
        }) {
            if let Some(proof) = &mut state.scalar_proof {
                proof.valid = false;
            }
            return Authorization::Deny;
        }
    }
    let allowed = match context.action {
        A::Attach { filename } => {
            let Ok(mut state) = state.lock() else {
                return Authorization::Deny;
            };
            let allowed = filename.is_empty()
                && state.allow_vacuum_attach
                && state
                    .deadline
                    .is_some_and(|deadline| Instant::now() < deadline);
            // A single internal empty ATTACH is sufficient for bare VACUUM.
            state.allow_vacuum_attach = false;
            allowed
        }
        A::Function { function_name } => !matches!(
            function_name.to_ascii_lowercase().as_str(),
            "load_extension" | "readfile" | "writefile" | "eval" | "fts3_tokenizer"
        ),
        A::Pragma {
            pragma_name,
            pragma_value,
        } => safe_pragma(pragma_name, pragma_value),
        A::CreateIndex { .. }
        | A::CreateTable { .. }
        | A::CreateTempIndex { .. }
        | A::CreateTempTable { .. }
        | A::CreateTempTrigger { .. }
        | A::CreateTempView { .. }
        | A::CreateTrigger { .. }
        | A::CreateView { .. }
        | A::Delete { .. }
        | A::DropIndex { .. }
        | A::DropTable { .. }
        | A::DropTempIndex { .. }
        | A::DropTempTable { .. }
        | A::DropTempTrigger { .. }
        | A::DropTempView { .. }
        | A::DropTrigger { .. }
        | A::DropView { .. }
        | A::Insert { .. }
        | A::Read { .. }
        | A::Select
        | A::Transaction { .. }
        | A::Update { .. }
        | A::AlterTable { .. }
        | A::Reindex { .. }
        | A::Analyze { .. }
        | A::Savepoint { .. }
        | A::Recursive => true,
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}

fn safe_pragma(name: &str, value: Option<&str>) -> bool {
    match name.to_ascii_lowercase().as_str() {
        "table_info"
        | "table_xinfo"
        | "index_info"
        | "index_xinfo"
        | "index_list"
        | "foreign_key_list"
        | "foreign_key_check"
        | "integrity_check"
        | "quick_check"
        | "user_version"
        | "application_id"
        | "foreign_keys"
        | "defer_foreign_keys"
        | "recursive_triggers"
        | "read_uncommitted"
        | "case_sensitive_like" => true,
        "schema_version" | "page_size" | "page_count" | "freelist_count" | "max_page_count"
        | "journal_mode" | "temp_store" | "trusted_schema" | "compile_options"
        | "database_list" | "collation_list" | "function_list" | "pragma_list" => value.is_none(),
        _ => false,
    }
}
