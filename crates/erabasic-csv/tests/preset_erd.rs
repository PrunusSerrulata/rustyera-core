use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
use erabasic_csv::{
    CsvDiagnosticCode, CsvLoadOptions, FilePayload, FrontendFile, ProjectFiles, load_project,
    resolve_deferred_indices,
};
use erabasic_data::{NameTableKind, UserIndexRegistration};

fn file(path: &str, text: &str) -> FrontendFile {
    FrontendFile {
        relative_path: path.into(),
        source_path: None,
        payload: FilePayload::Utf8(text.into()),
    }
}

fn snake() -> CsvLoadOptions {
    CsvLoadOptions {
        compatibility: CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake),
        ..CsvLoadOptions::default()
    }
}

#[test]
fn preset_erd_fills_empty_csv_before_lookup_and_character_loading() {
    let files = ProjectFiles {
        csv: vec![
            file("ABL.CSV", "0,csv\n1,\n"),
            file("ABL.als", "3,alias\n"),
            file("CHARA0.CSV", "NO,0\nABL,filled,7\n"),
        ],
        erb: vec![file(
            "deep/ABL.erd",
            "0,rejected\n1,filled\n2,new\n3,alias\n",
        )],
    };
    let report = load_project(&files, &snake());
    let data = report.data.unwrap().static_data;
    let table = &data.name_tables[&NameTableKind::Abl];
    assert_eq!(table.names[0].as_deref(), Some("csv"));
    assert_eq!(table.lookup["filled"], 1);
    assert_eq!(table.lookup["new"], 2);
    assert_eq!(table.lookup["alias"], 3);
    assert_eq!(data.characters[0].abl[&1], 7);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].code,
        CsvDiagnosticCode::DuplicateIndex
    );
    assert_eq!(
        report.diagnostics[0].source.as_ref().unwrap().relative_path,
        "deep/ABL.erd"
    );
}

#[test]
fn preset_erd_shared_stems_use_case_insensitive_path_order_and_empty_names_remain_fillable() {
    let files = ProjectFiles {
        csv: vec![],
        erb: vec![
            file("B/UP.ERD", "0,later\n1,filled\n"),
            file("a/palam.erd", "0,first\n1,\n"),
        ],
    };
    for reverse in [false, true] {
        let mut files = files.clone();
        if reverse {
            files.erb.reverse();
        }
        let report = load_project(&files, &snake());
        let table = &report.data.as_ref().unwrap().static_data.name_tables[&NameTableKind::Palam];
        assert_eq!(table.lookup["first"], 0);
        assert_eq!(table.lookup["filled"], 1);
        assert!(!table.lookup.contains_key("later"));
        assert_eq!(report.diagnostics.len(), 1);
    }
}

#[test]
fn preset_erd_cross_table_duplicates_skip_whole_item_row_and_str_has_no_lookup() {
    let files = ProjectFiles {
        csv: vec![file("ABL.CSV", "0,taken\n"), file("STR.CSV", "0,text\n")],
        erb: vec![
            file(
                "ITEM.erd",
                "0,taken,80\n1,text,90\n2,unique,100\n3,unique,110\n",
            ),
            file("STR.erd", "1,extra\n"),
        ],
    };
    let report = load_project(&files, &snake());
    let data = report.data.unwrap().static_data;
    assert_eq!(&data.item_prices[..4], &[0, 0, 100, 0]);
    assert_eq!(data.name_tables[&NameTableKind::Item].names[0], None);
    assert!(data.name_tables[&NameTableKind::Str].lookup.is_empty());
    assert_eq!(
        data.name_tables[&NameTableKind::Str].names[1].as_deref(),
        Some("extra")
    );
    assert_eq!(report.diagnostics.len(), 3);
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.code == CsvDiagnosticCode::DuplicateUserIndex)
    );
}

#[test]
fn preset_erd_item_prices_preserve_explicit_zero_and_only_first_erd_price() {
    let files = ProjectFiles {
        csv: vec![file(
            "ITEM.CSV",
            "0,zero,0\n1,missing\n2,bad,no\n3,paid,20\n",
        )],
        erb: vec![
            file(
                "a/ITEMPRICE.erd",
                "0,zero,4\n1,missing,5\n2,bad,6\n3,paid,20\n4,fresh,bad\n",
            ),
            file("b/ITEMSALES.erd", "1,missing,99\n4,fresh,8\n"),
        ],
    };
    let report = load_project(&files, &snake());
    assert_eq!(
        &report.data.unwrap().static_data.item_prices[..5],
        &[0, 5, 6, 20, 8]
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        vec![
            CsvDiagnosticCode::InvalidInteger,
            CsvDiagnosticCode::DuplicateIndex,
            CsvDiagnosticCode::InvalidInteger
        ]
    );
}

#[test]
fn preset_erd_rejects_bad_rows_without_losing_following_rows() {
    let files = ProjectFiles {
        csv: vec![],
        erb: vec![file(
            "ABL.erd",
            "bad\nno,name\n-1,negative\n2147483647,huge\n0,valid\n",
        )],
    };
    let report = load_project(&files, &snake());
    assert_eq!(
        report.data.unwrap().static_data.name_tables[&NameTableKind::Abl].lookup["valid"],
        0
    );
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>(),
        vec![
            CsvDiagnosticCode::MissingComma,
            CsvDiagnosticCode::InvalidInteger,
            CsvDiagnosticCode::IndexOutOfRange,
            CsvDiagnosticCode::IndexOutOfRange
        ]
    );
}

#[test]
fn preset_erd_is_disabled_for_original_and_use_erd_false() {
    let files = ProjectFiles {
        csv: vec![],
        erb: vec![file("ABL.erd", "0,added\n")],
    };
    for options in [
        CsvLoadOptions::default(),
        CsvLoadOptions {
            use_erd: false,
            ..snake()
        },
    ] {
        let report = load_project(&files, &options);
        assert!(report.diagnostics.is_empty());
        assert_eq!(
            report.data.unwrap().static_data.name_tables[&NameTableKind::Abl].names[0],
            None
        );
    }
}

#[test]
fn preset_erd_does_not_consume_custom_erd_and_alias_inputs() {
    let files = ProjectFiles {
        csv: vec![],
        erb: vec![
            file("KEY.erd", "0,custom\n"),
            file("KEY.als", "0,alias\n"),
            file("ABL.erd", "0,preset\n"),
        ],
    };
    let options = snake();
    let mut data = load_project(&files, &options).data.unwrap();
    let diagnostics = resolve_deferred_indices(
        &mut data,
        &[UserIndexRegistration {
            variable_name: "MYVAR".into(),
            source_stem: "KEY".into(),
            dimension: None,
            length: 2,
        }],
        &options,
    );
    assert!(diagnostics.is_empty());
    let resolved = &data.static_data.deferred_indices.resolved["MYVAR"];
    assert_eq!(resolved.entries["custom"], 0);
    assert_eq!(resolved.entries["alias"], 0);
    assert_eq!(
        data.static_data.name_tables[&NameTableKind::Abl].lookup["preset"],
        0
    );
}

#[test]
fn preset_erd_path_sort_uses_unicode_simple_casing_and_supplementary_order() {
    for paths in [
        ["ä/ABL.erd", "×/ABL.erd"],
        ["\u{ffff}/ABL.erd", "\u{10000}/ABL.erd"],
        ["\u{10428}/ABL.erd", "\u{10401}/ABL.erd"],
    ] {
        let files = ProjectFiles {
            csv: vec![],
            erb: vec![file(paths[1], "0,second\n"), file(paths[0], "0,first\n")],
        };
        let report = load_project(&files, &snake());
        assert_eq!(
            report.data.unwrap().static_data.name_tables[&NameTableKind::Abl].names[0].as_deref(),
            Some("first")
        );
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0].code,
            CsvDiagnosticCode::DuplicateIndex
        );
    }
}

#[test]
fn preset_erd_windows_separator_order_is_independent_of_submission_order() {
    for reverse in [false, true] {
        let mut files = ProjectFiles {
            csv: vec![],
            erb: vec![
                file("a/ABL.erd", "12,slash_later\n"),
                file("a0/ABL.erd", "12,digit_first\n"),
            ],
        };
        if reverse {
            files.erb.reverse();
        }
        let report = load_project(&files, &snake());
        let table = &report.data.as_ref().unwrap().static_data.name_tables[&NameTableKind::Abl];
        assert_eq!(table.names[12].as_deref(), Some("digit_first"));
        assert_eq!(table.lookup["digit_first"], 12);
        assert!(!table.lookup.contains_key("slash_later"));
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0].code,
            CsvDiagnosticCode::DuplicateIndex
        );
        assert_eq!(
            report.diagnostics[0].source.as_ref().unwrap().relative_path,
            "a/ABL.erd"
        );
    }
}
