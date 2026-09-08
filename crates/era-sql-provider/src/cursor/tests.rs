use super::*;

fn connection() -> Rc<Connection> {
    Rc::new(Connection::open_in_memory().unwrap())
}

fn cursor(connection: &Rc<Connection>, sql: &str) -> Cursor {
    Cursor::prepare(Rc::clone(connection), sql)
        .unwrap()
        .0
        .unwrap()
}

#[test]
fn prepare_is_lazy_and_reports_multistatement_byte_tail() {
    let db = connection();
    let sql = "SELECT '眼'; SELECT 2";
    let (first, consumed) = Cursor::prepare(Rc::clone(&db), sql).unwrap();
    let mut first = first.unwrap();
    assert_eq!(&sql[..consumed], "SELECT '眼';");
    assert!(first.integer(0).is_err());
    assert!(first.step().unwrap());
    assert_eq!(first.text(0, 10).unwrap().as_deref(), Some("眼"));
    let mut second = cursor(&db, &sql[consumed..]);
    assert!(second.step().unwrap());
    assert_eq!(second.integer(0).unwrap(), 2);
    let (empty, consumed) = Cursor::prepare(db, " -- only comment").unwrap();
    assert!(empty.is_none());
    assert_eq!(consumed, " -- only comment".len());
}

#[test]
fn empty_and_invalid_sql_do_not_leave_cursors_or_truncate_at_nul() {
    let db = connection();
    let (empty, consumed) = Cursor::prepare(Rc::clone(&db), "").unwrap();
    assert!(empty.is_none());
    assert_eq!(consumed, 0);
    assert!(Cursor::prepare(Rc::clone(&db), "SELECT 1\0; SELECT 2").is_err());
    assert!(Cursor::prepare(Rc::clone(&db), "SELECT FROM").is_err());
    assert_eq!(Rc::strong_count(&db), 1);
}

#[test]
fn named_binding_preserves_omissions_and_copies_owned_strings() {
    let db = connection();
    let mut c = cursor(&db, "SELECT @1, @0, @2, ?");
    assert_eq!(c.parameter_count(), 4);
    assert_eq!(c.parameter_name(1).unwrap().as_deref(), Some("@1"));
    assert_eq!(c.parameter_name(4).unwrap(), None);
    assert!(c.parameter_name(0).is_err());
    assert!(c.parameter_name(5).is_err());
    let values = vec![
        SqlValueV1::String("a\0b".into()),
        SqlValueV1::Integer(i64::MIN),
    ];
    c.bind(&values).unwrap();
    drop(values);
    assert!(c.step().unwrap());
    assert_eq!(c.integer(0).unwrap(), i64::MIN);
    assert_eq!(c.text(1, 3).unwrap().as_deref(), Some("a\0b"));
    assert_eq!(c.column_type(2).unwrap(), ffi::SQLITE_NULL);
    assert_eq!(c.column_type(3).unwrap(), ffi::SQLITE_NULL);
    assert!(c.bind(&[]).is_err());
    let mut unknown = cursor(&db, "SELECT @1");
    assert!(matches!(
        unknown.bind(&[SqlValueV1::Null]),
        Err(Error::InvalidParameterName(_))
    ));
}

#[test]
fn multiple_readers_keep_connection_alive_without_advancing_each_other() {
    let db = connection();
    let weak = Rc::downgrade(&db);
    let mut a = cursor(&db, "SELECT 1 UNION ALL SELECT 2");
    let mut b = cursor(&db, "SELECT 7 UNION ALL SELECT 8");
    drop(db);
    assert!(a.step().unwrap());
    assert!(b.step().unwrap());
    assert_eq!(a.integer(0).unwrap(), 1);
    assert_eq!(a.integer(0).unwrap(), 1);
    assert!(a.step().unwrap());
    assert_eq!(b.integer(0).unwrap(), 7);
    a.finalize().unwrap();
    assert!(b.step().unwrap());
    assert_eq!(b.integer(0).unwrap(), 8);
    assert!(!b.step().unwrap());
    assert!(!b.step().unwrap());
    assert!(b.column_type(0).is_err());
    drop(b);
    assert!(weak.upgrade().is_none());
}

#[test]
fn coercions_are_sqlite_native_and_validate_current_row_and_column() {
    let db = connection();
    let mut c = cursor(
        &db,
        "SELECT '12e3', 'x', '9999999999999999999999', 1.5, x'313200', NULL",
    );
    assert!(c.step().unwrap());
    assert_eq!(c.column_type(0).unwrap(), ffi::SQLITE_TEXT);
    assert_eq!(c.column_type(3).unwrap(), ffi::SQLITE_FLOAT);
    assert_eq!(c.column_type(4).unwrap(), ffi::SQLITE_BLOB);
    assert_eq!(c.integer(0).unwrap(), 12);
    assert_eq!(c.integer(1).unwrap(), 0);
    assert_eq!(c.integer(2).unwrap(), i64::MAX);
    assert_eq!(c.text(3, 10).unwrap().as_deref(), Some("1.5"));
    assert_eq!(c.bytes(4).unwrap(), 3);
    assert_eq!(c.text(4, 3).unwrap().as_deref(), Some("12\0"));
    assert_eq!(c.text(5, 0).unwrap(), None);
    assert!(matches!(c.integer(6), Err(Error::InvalidColumnIndex(6))));
    assert!(c.text(usize::MAX, 1).is_err());
}

#[test]
fn oo1_text_decoder_preserves_nul_replaces_invalid_utf8_and_strips_one_bom() {
    let db = connection();
    let mut c = cursor(&db, "SELECT x'61ff0062', x'efbbbfefbbbf61', ''");
    assert!(c.step().unwrap());
    assert_eq!(c.text(0, 6).unwrap().as_deref(), Some("a\u{fffd}\0b"));
    assert!(c.text(0, 5).is_err());
    assert_eq!(c.text(1, 4).unwrap().as_deref(), Some("\u{feff}a"));
    assert_eq!(c.text(2, 0).unwrap().as_deref(), Some(""));
}

#[test]
fn cell_limit_cannot_be_relaxed_by_caller() {
    let db = connection();
    let mut c = cursor(&db, "SELECT zeroblob(1048577), zeroblob(1048576)");
    assert!(c.step().unwrap());
    assert!(
        matches!(c.text(0, usize::MAX), Err(Error::SqliteFailure(e, _)) if e.extended_code == ffi::SQLITE_TOOBIG)
    );
    assert_eq!(
        c.text(1, usize::MAX).unwrap().unwrap().len(),
        MAX_CELL_BYTES
    );
}

#[test]
fn failed_step_cannot_auto_reset_and_repeat_side_effects() {
    let db = connection();
    db.execute_batch("CREATE TABLE t(x UNIQUE); INSERT INTO t VALUES(1)")
        .unwrap();
    let mut c = cursor(&db, "INSERT INTO t VALUES(1)");
    assert!(!c.readonly());
    assert!(c.step().is_err());
    assert!(c.step().is_err());
    assert!(c.column_type(0).is_err());
    assert!(c.finalize().is_err());
    assert_eq!(Rc::strong_count(&db), 1);
    assert!(cursor(&db, "SELECT * FROM t").readonly());
}
