//! The explicitly authorized `SQLite` cursor FFI boundary.
//!
//! All handles stay on their owner thread. `Rc` keeps the connection alive and makes this
//! type neither Send nor Sync. No raw handle or borrowed `SQLite` allocation escapes this module.
//! Connection policy (authorizer, progress, limits) remains the provider's responsibility.

use std::ffi::{CStr, CString};
use std::ptr::{self, NonNull};
use std::rc::Rc;

use era_runtime_protocol::{SqlLimitsV1, SqlValueV1};
use rusqlite::{Connection, Error, Result, ffi};

const MAX_CELL_BYTES: usize = SqlLimitsV1::FIXED.maximum_cell_bytes as usize;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Ready,
    Row,
    Done,
    Failed,
}

/// An independently owned prepared statement; never resets or reads ahead implicitly.
pub struct Cursor {
    connection: Rc<Connection>,
    statement: Option<NonNull<ffi::sqlite3_stmt>>,
    position: Position,
}

impl Cursor {
    /// Prepare the first statement and return the number of UTF-8 input bytes consumed.
    /// Comments/whitespace may consume bytes without producing a cursor.
    pub fn prepare(connection: Rc<Connection>, sql: &str) -> Result<(Option<Self>, usize)> {
        let length = i32::try_from(sql.len()).map_err(|_| failure(ffi::SQLITE_TOOBIG))?;
        let sql = CString::new(sql).map_err(Error::NulError)?;
        let mut statement = ptr::null_mut();
        let mut tail = ptr::null();
        // SAFETY: Rc owns the live connection. The terminated SQL allocation and both output
        // slots remain valid for the synchronous call; SQLite copies the prepared SQL.
        let code = unsafe {
            ffi::sqlite3_prepare_v2(
                connection.handle(),
                sql.as_ptr(),
                length,
                &raw mut statement,
                &raw mut tail,
            )
        };
        let cursor = NonNull::new(statement).map(|statement| Self {
            connection: Rc::clone(&connection),
            statement: Some(statement),
            position: Position::Ready,
        });
        if code != ffi::SQLITE_OK {
            // Capture the original diagnostic before the optional cursor is finalized.
            return Err(database_error(&connection, code));
        }
        // SQLite returns a tail in the input allocation. Check the numeric range as well,
        // avoiding pointer subtraction UB even if this external invariant were violated.
        let consumed = (tail as usize)
            .checked_sub(sql.as_ptr() as usize)
            .filter(|offset| *offset <= sql.as_bytes().len())
            .ok_or_else(|| failure(ffi::SQLITE_MISUSE))?;
        drop(connection);
        Ok((cursor, consumed))
    }

    /// Bind only supplied @0, @1, ... names. Omitted names retain `SQLite`'s NULL default
    /// (or an earlier binding); supplied names absent from the statement are errors like OO1.
    pub fn bind(&mut self, values: &[SqlValueV1]) -> Result<()> {
        if self.position != Position::Ready {
            return Err(failure(ffi::SQLITE_MISUSE));
        }
        for (offset, value) in values.iter().enumerate() {
            let name = CString::new(format!("@{offset}")).map_err(Error::NulError)?;
            // SAFETY: statement is live and name is a terminated string for this call.
            let index = unsafe { ffi::sqlite3_bind_parameter_index(self.raw(), name.as_ptr()) };
            if index == 0 {
                return Err(Error::InvalidParameterName(
                    name.to_string_lossy().into_owned(),
                ));
            }
            // SAFETY: index was resolved on this statement; no step has occurred. TRANSIENT
            // makes SQLite copy string bytes before returning, including embedded NUL bytes.
            let code = unsafe {
                match value {
                    SqlValueV1::Null => ffi::sqlite3_bind_null(self.raw(), index),
                    SqlValueV1::Integer(value) => {
                        ffi::sqlite3_bind_int64(self.raw(), index, *value)
                    }
                    SqlValueV1::String(value) => {
                        let length =
                            i32::try_from(value.len()).map_err(|_| failure(ffi::SQLITE_TOOBIG))?;
                        ffi::sqlite3_bind_text(
                            self.raw(),
                            index,
                            value.as_ptr().cast(),
                            length,
                            ffi::SQLITE_TRANSIENT(),
                        )
                    }
                }
            };
            if code != ffi::SQLITE_OK {
                return Err(database_error(&self.connection, code));
            }
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<bool> {
        match self.position {
            Position::Done => return Ok(false),
            Position::Failed => return Err(failure(ffi::SQLITE_MISUSE)),
            Position::Ready | Position::Row => {}
        }
        // SAFETY: exclusively accessed live statement on its owner thread. No SQLite value
        // reference escapes a previous accessor, so advancing cannot invalidate Rust borrows.
        let code = unsafe { ffi::sqlite3_step(self.raw()) };
        match code {
            ffi::SQLITE_ROW => {
                self.position = Position::Row;
                Ok(true)
            }
            ffi::SQLITE_DONE => {
                self.position = Position::Done;
                Ok(false)
            }
            _ => {
                self.position = Position::Failed;
                Err(database_error(&self.connection, code))
            }
        }
    }

    pub fn column_count(&self) -> usize {
        // SAFETY: metadata is valid on a live statement even before stepping.
        usize::try_from(unsafe { ffi::sqlite3_column_count(self.raw()) })
            .expect("SQLite column count is nonnegative")
    }

    /// Return `SQLite`'s current type, without caching or coercion. Capture original types in
    /// the provider before projections; `SQLite` type observations after coercion stay native.
    pub fn column_type(&self, index: usize) -> Result<i32> {
        let index = self.row_index(index)?;
        // SAFETY: row_index verifies SQLITE_ROW state and the column range.
        Ok(unsafe { ffi::sqlite3_column_type(self.raw(), index) })
    }

    /// `SQLite` coercion, not the protocol's separate strict scalar-integer parser.
    pub fn integer(&mut self, index: usize) -> Result<i64> {
        let index = self.row_index(index)?;
        // SAFETY: checked current row/column. No borrowed column buffers escape this method.
        Ok(unsafe { ffi::sqlite3_column_int64(self.raw(), index) })
    }

    /// `SQLite`'s byte-length accessor can itself convert numeric/UTF-16 values to UTF-8.
    pub fn bytes(&mut self, index: usize) -> Result<usize> {
        let index = self.row_index(index)?;
        // SAFETY: checked current row/column; no prior column pointer is retained.
        let length = unsafe { ffi::sqlite3_column_bytes(self.raw(), index) };
        self.check_conversion_oom()?;
        usize::try_from(length).map_err(|_| failure(ffi::SQLITE_MISUSE))
    }

    /// Match current OO1 `column_text`: decode exactly `column_bytes`, preserving embedded NUL.
    /// `TextDecoder` replaces malformed UTF-8 and strips one initial UTF-8 BOM. The resulting
    /// UTF-8 byte length is bounded by both the caller's budget and the protocol's 1 MiB cap.
    pub fn text(&mut self, index: usize, max_bytes: usize) -> Result<Option<String>> {
        let index = self.row_index(index)?;
        // SAFETY: checked row/index. SQLite owns this buffer until conversion/step/finalize.
        let text = unsafe { ffi::sqlite3_column_text(self.raw(), index) };
        self.check_conversion_oom()?;
        if text.is_null() {
            return Ok(None);
        }
        // SAFETY: text followed by bytes is SQLite's documented non-invalidating UTF-8 pair.
        let length = unsafe { ffi::sqlite3_column_bytes(self.raw(), index) };
        self.check_conversion_oom()?;
        let length = usize::try_from(length).map_err(|_| failure(ffi::SQLITE_MISUSE))?;
        // SAFETY: non-null SQLite UTF-8 allocation contains exactly length readable bytes.
        // There are no further SQLite calls while this slice is borrowed, and none escapes.
        let bytes = unsafe { std::slice::from_raw_parts(text, length) };
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let maximum = max_bytes.min(MAX_CELL_BYTES);
        // Lossy decoding cannot shrink these bytes after BOM removal. Bound before allocating.
        if bytes.len() > maximum {
            return Err(failure(ffi::SQLITE_TOOBIG));
        }
        let decoded = String::from_utf8_lossy(bytes);
        if decoded.len() > maximum {
            return Err(failure(ffi::SQLITE_TOOBIG));
        }
        Ok(Some(decoded.into_owned()))
    }

    pub fn readonly(&self) -> bool {
        // SAFETY: metadata access on a live statement.
        unsafe { ffi::sqlite3_stmt_readonly(self.raw()) != 0 }
    }

    pub fn parameter_count(&self) -> usize {
        // SAFETY: metadata access on a live statement.
        usize::try_from(unsafe { ffi::sqlite3_bind_parameter_count(self.raw()) })
            .expect("SQLite parameter count is nonnegative")
    }

    /// `SQLite` parameter positions are one-based; anonymous parameters have no name.
    #[cfg(test)]
    pub fn parameter_name(&self, index: usize) -> Result<Option<String>> {
        if index == 0 || index > self.parameter_count() {
            return Err(failure(ffi::SQLITE_RANGE));
        }
        let index = i32::try_from(index).map_err(|_| failure(ffi::SQLITE_RANGE))?;
        // SAFETY: checked parameter position on a live statement.
        let name = unsafe { ffi::sqlite3_bind_parameter_name(self.raw(), index) };
        if name.is_null() {
            return Ok(None);
        }
        // SAFETY: SQLite owns the NUL-terminated name until finalization; copy before returning.
        Ok(Some(
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned(),
        ))
    }

    pub fn finalize(mut self) -> Result<()> {
        self.finish()
    }

    fn finish(&mut self) -> Result<()> {
        let Some(statement) = self.statement.take() else {
            return Ok(());
        };
        // SAFETY: take removes the sole owned statement pointer before finalization, so Drop
        // cannot finalize twice. The Rc connection is still alive throughout this call.
        let code = unsafe { ffi::sqlite3_finalize(statement.as_ptr()) };
        if code == ffi::SQLITE_OK {
            Ok(())
        } else {
            Err(database_error(&self.connection, code))
        }
    }

    fn raw(&self) -> *mut ffi::sqlite3_stmt {
        self.statement.expect("cursor already finalized").as_ptr()
    }

    fn row_index(&self, index: usize) -> Result<i32> {
        if self.position != Position::Row {
            return Err(failure(ffi::SQLITE_MISUSE));
        }
        if index >= self.column_count() {
            return Err(Error::InvalidColumnIndex(index));
        }
        i32::try_from(index).map_err(|_| Error::InvalidColumnIndex(index))
    }

    fn check_conversion_oom(&self) -> Result<()> {
        // SAFETY: live connection; this check immediately follows the conversion call.
        let code = unsafe { ffi::sqlite3_errcode(self.connection.handle()) };
        if code == ffi::SQLITE_NOMEM {
            Err(database_error(&self.connection, code))
        } else {
            Ok(())
        }
    }
}

impl Drop for Cursor {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn failure(code: i32) -> Error {
    Error::SqliteFailure(ffi::Error::new(code), None)
}

fn database_error(connection: &Connection, code: i32) -> Error {
    // SAFETY: live connection with no concurrent access; SQLite owns the terminated message.
    let message = unsafe { CStr::from_ptr(ffi::sqlite3_errmsg(connection.handle())) };
    Error::SqliteFailure(
        ffi::Error::new(code),
        Some(message.to_string_lossy().into_owned()),
    )
}

#[cfg(test)]
mod tests;
