use crate::{ProviderError, Result, cursor::Cursor};
use era_runtime_protocol::{
    SqlErrorCodeV1, SqlExecuteModeV1, SqlReaderCellV1, SqlReaderValueModeV1, SqlValueV1,
};
use rusqlite::ffi::{SQLITE_INTEGER, SQLITE_NULL, SQLITE_TEXT};

const CELL_BYTES: usize = 1024 * 1024;

pub(crate) fn native(
    cursor: &mut Cursor,
    column: usize,
    original: Option<i32>,
) -> Result<SqlValueV1> {
    match original.unwrap_or(cursor.column_type(column)?) {
        SQLITE_NULL => Ok(SqlValueV1::Null),
        SQLITE_INTEGER => Ok(SqlValueV1::Integer(cursor.integer(column)?)),
        SQLITE_TEXT => Ok(cursor
            .text(column, CELL_BYTES)?
            .map_or(SqlValueV1::Null, SqlValueV1::String)),
        _ => Err(ProviderError::new(
            SqlErrorCodeV1::TypeMismatch,
            "SQL value type is not supported by v1",
        )),
    }
}

pub(crate) fn scalar(cursor: &mut Cursor, mode: SqlExecuteModeV1) -> Result<SqlValueV1> {
    let value = native(cursor, 0, None)?;
    if mode != SqlExecuteModeV1::ScalarInteger {
        return Ok(value);
    }
    let SqlValueV1::String(text) = value else {
        return Ok(value);
    };
    let text = text.trim_matches(js_whitespace);
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProviderError::new(
            SqlErrorCodeV1::TypeMismatch,
            "SQL scalar is not an integer",
        ));
    }
    text.parse::<i64>().map(SqlValueV1::Integer).map_err(|_| {
        ProviderError::new(
            SqlErrorCodeV1::TypeMismatch,
            "SQL scalar integer is out of range",
        )
    })
}

// ECMAScript WhiteSpace/LineTerminator, not Rust's broader Unicode whitespace set.
fn js_whitespace(value: char) -> bool {
    matches!(value, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' |
        '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub(crate) fn reader(
    cursor: &mut Cursor,
    column: usize,
    mode: SqlReaderValueModeV1,
    original: Option<i32>,
) -> Result<SqlValueV1> {
    if original.unwrap_or(cursor.column_type(column)?) == SQLITE_NULL {
        return Ok(SqlValueV1::Null);
    }
    match mode {
        SqlReaderValueModeV1::Integer => Ok(SqlValueV1::Integer(cursor.integer(column)?)),
        SqlReaderValueModeV1::String => Ok(cursor
            .text(column, CELL_BYTES)?
            .map_or(SqlValueV1::Null, SqlValueV1::String)),
    }
}

pub(crate) fn project(cursor: &mut Cursor, originals: &[Option<i32>]) -> Vec<SqlReaderCellV1> {
    let mut bytes = 0usize;
    let mut cells = Vec::new();
    for (column, original) in originals.iter().enumerate() {
        bytes += 16;
        if bytes > 64 * 1024 {
            break;
        }
        let mut cell = SqlReaderCellV1 {
            integer: None,
            string: None,
            is_null: None,
        };
        if let Some(kind) = original {
            if *kind == SQLITE_NULL {
                cell.is_null = Some(true);
                cell.integer = Some(0);
                cell.string = Some(String::new());
            } else if cursor
                .bytes(column)
                .is_ok_and(|length| length <= 64 * 1024 - bytes)
            {
                if let Ok(SqlValueV1::String(text)) =
                    reader(cursor, column, SqlReaderValueModeV1::String, *original)
                {
                    if text.len() <= 64 * 1024 - bytes {
                        bytes += text.len();
                        cell.string = Some(text);
                    }
                    cell.is_null = Some(false);
                }
                if let Ok(SqlValueV1::Integer(value)) =
                    reader(cursor, column, SqlReaderValueModeV1::Integer, *original)
                {
                    cell.integer = Some(value);
                }
            }
        }
        cells.push(cell);
    }
    cells
}
