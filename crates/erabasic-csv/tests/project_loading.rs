use erabasic_csv::{
    CsvDiagnosticCode, CsvLoadOptions, FilePayload, FrontendFile, FrontendIoError,
    FrontendIoErrorKind, ProjectFiles, load_project, resolve_deferred_indices,
};
use erabasic_data::{CharacterSelection, NameTableKind, UserIndexRegistration};

fn file(path: &str, content: &str) -> FrontendFile {
    FrontendFile {
        relative_path: path.into(),
        payload: FilePayload::Utf8(content.into()),
    }
}

fn full_project_options() -> CsvLoadOptions {
    CsvLoadOptions {
        use_rename_file: true,
        compatible_call_name: true,
        compatible_sp_character: true,
        ..CsvLoadOptions::default()
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn loads_schema_static_data_and_phase_seeds() {
    let files = ProjectFiles {
        csv: vec![
            file(
                "_Replace.csv",
                "MONEYLABEL,円\nPBANDDEF,9\nRELATIONDEF,7\nPALAMLVDEF,0/10/20\n",
            ),
            file("_Rename.csv", "REAL,display\nESCAPED,with\\,comma\n"),
            file(
                "GAMEBASE.CSV",
                "コード,42\nバージョン,1234beta\n最初からいるキャラ,1\nアイテムなし,8\nタイトル,Demo\n",
            ),
            file("VariableSize.csv", "ABL,120\nABLNAME,110\nCDFLAG,2,3\n"),
            file("ABL.csv", "0,zero\n2,two\n2,later\n"),
            file("ABL.als", "1,alias\n3,zero\n"),
            file("ITEM.csv", "5,potion,120\n"),
            file("STR.csv", "0,initial text\n"),
            file("CSTR.csv", "0,greeting\n"),
            file(
                "CHARA0.csv",
                "NO,10\nNAME,Alice\nABL,later,5\nBASE,0,100\nCFLAG,0,1\nCSTR,greeting,hello\n",
            ),
            file(
                "nested/VarExt-extra.csv",
                "GLOBAL_MAPS,shared\nSAVE_XMLS,slot\nSTATIC_DTS,table\n",
            ),
            file("FOO.csv", "0,csv-key\n"),
        ],
        erb: vec![file("nested/FOO.erd", "1,erd-key\n")],
    };

    let report = load_project(&files, &full_project_options());
    let project = report.data.unwrap();

    assert_eq!(project.schema.variable("ABL").unwrap().dimensions, [120]);
    assert_eq!(project.schema.index_spaces[&NameTableKind::Abl].length, 120);
    assert_eq!(
        project.schema.variable("CDFLAG").unwrap().dimensions,
        [2, 3]
    );
    let abl = &project.static_data.name_tables[&NameTableKind::Abl];
    assert_eq!(abl.names[1], None);
    assert_eq!(abl.names[2].as_deref(), Some("later"));
    assert_eq!(abl.lookup["zero"], 0);
    assert_eq!(abl.lookup["alias"], 1);
    assert_eq!(project.static_data.item_prices[5], 120);
    assert_eq!(project.static_data.rename["[[display]]"], "REAL");
    assert_eq!(project.static_data.replace.money_label, "円");
    assert_eq!(project.static_data.game_base.version, 1234);
    assert_eq!(
        project.static_data.game_base.window_title.as_deref(),
        Some("Demo 1.234")
    );

    let character = &project.static_data.characters[0];
    assert_eq!(character.csv_no, 0);
    assert_eq!(character.call_name, "Alice");
    assert!(character.is_sp_character);
    assert_eq!(character.abl[&2], 5);
    assert_eq!(character.cstr[&0], "hello");
    assert_eq!(project.static_data.relation_lookup["Alice"], 10);
    assert!(
        project
            .static_data
            .extensions
            .global_maps
            .contains("shared")
    );
    assert!(project.static_data.extensions.save_xmls.contains("slot"));
    assert!(
        project
            .static_data
            .extensions
            .static_data_tables
            .contains("table")
    );
    assert_eq!(project.static_data.deferred_indices.groups["FOO"].len(), 2);

    let new_game = project.new_game_seed();
    assert_eq!(
        new_game.initial_characters,
        [
            CharacterSelection::CsvNumber(0),
            CharacterSelection::CsvNumber(1)
        ]
    );
    assert_eq!(new_game.defaults.pband_0, 9);
    assert_eq!(new_game.defaults.no_item_0, 0);
    assert_eq!(
        new_game.defaults.str_values[0].as_deref(),
        Some("initial text")
    );
    let save = project.save_load_context();
    assert!(save.clear_characters_before_overlay);
    assert!(save.copy_and_truncate_arrays);
    assert!(save.compatibility.accepts(0, 1234));

    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == CsvDiagnosticCode::ReconciledVariableSize })
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CsvDiagnosticCode::DuplicateIndex)
    );
}

#[test]
fn cflag_zero_only_marks_special_characters_when_compatibility_is_enabled() {
    let files = ProjectFiles {
        csv: vec![file("CHARA0.csv", "NO,0\nNAME,Player\nCFLAG,0,1900\n")],
        erb: vec![],
    };

    let normal = load_project(&files, &CsvLoadOptions::default())
        .data
        .unwrap();
    assert!(!normal.static_data.characters[0].is_sp_character);
    assert_eq!(normal.static_data.characters[0].cflag[&0], 1900);

    let compatible = load_project(
        &files,
        &CsvLoadOptions {
            compatible_sp_character: true,
            ..CsvLoadOptions::default()
        },
    )
    .data
    .unwrap();
    assert!(compatible.static_data.characters[0].is_sp_character);
}

#[test]
fn handles_bom_comments_crlf_and_continuations() {
    let files = ProjectFiles {
        csv: vec![file(
            "ABL.csv",
            "\u{feff}; comment\r\n{\r\n0,first\r\n}\r\n　; full width comment\r\n1,second\r\n",
        )],
        erb: vec![],
    };

    let report = load_project(&files, &CsvLoadOptions::default());
    let table = &report.data.unwrap().static_data.name_tables[&NameTableKind::Abl];

    assert_eq!(table.names[0].as_deref(), Some("first "));
    assert_eq!(table.names[1].as_deref(), Some("second"));
}

#[test]
fn reports_frontend_errors_but_treats_not_found_as_absent() {
    let files = ProjectFiles {
        csv: vec![
            FrontendFile {
                relative_path: "missing.csv".into(),
                payload: FilePayload::IoError(FrontendIoError {
                    kind: FrontendIoErrorKind::NotFound,
                    message: "missing".into(),
                }),
            },
            FrontendFile {
                relative_path: "denied.csv".into(),
                payload: FilePayload::IoError(FrontendIoError {
                    kind: FrontendIoErrorKind::PermissionDenied,
                    message: "denied".into(),
                }),
            },
        ],
        erb: vec![],
    };

    let report = load_project(&files, &CsvLoadOptions::default());

    assert!(report.data.is_some());
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == CsvDiagnosticCode::IoError)
            .count(),
        1
    );
}

#[test]
fn rejects_fatal_cdflag_mismatch_and_newer_engine_requirement() {
    let mismatch = ProjectFiles {
        csv: vec![file(
            "VariableSize.csv",
            "CDFLAG,2,3\nCDFLAGNAME1,100\nCDFLAGNAME2,100\n",
        )],
        erb: vec![],
    };
    let report = load_project(&mismatch, &CsvLoadOptions::default());
    assert!(report.data.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CsvDiagnosticCode::CdflagShapeMismatch)
    );

    let newer = ProjectFiles {
        csv: vec![file(
            "GAMEBASE.csv",
            "動作に必要なEmueraのバージョン,9.0.0.0\n",
        )],
        erb: vec![],
    };
    let report = load_project(&newer, &CsvLoadOptions::default());
    assert!(report.data.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == CsvDiagnosticCode::RequiresNewerEmuera })
    );
}

#[test]
fn resolves_deferred_indices_after_user_schema_is_known() {
    let files = ProjectFiles {
        csv: vec![file("DIMKEY.csv", "0,first\n1,duplicate\n")],
        erb: vec![file("sub/DIMKEY.erd", "2,third\n")],
    };
    let mut project = load_project(&files, &CsvLoadOptions::default())
        .data
        .unwrap();
    let diagnostics = resolve_deferred_indices(
        &mut project,
        &[UserIndexRegistration {
            variable_name: "MYVAR".into(),
            source_stem: "DIMKEY".into(),
            dimension: None,
            length: 3,
        }],
        &CsvLoadOptions::default(),
    );

    assert!(diagnostics.is_empty());
    let resolved = &project.static_data.deferred_indices.resolved["MYVAR"];
    assert_eq!(resolved.entries["first"], 0);
    assert_eq!(resolved.entries["third"], 2);
}

#[test]
fn duplicate_deferred_key_is_fatal_for_that_registration() {
    let files = ProjectFiles {
        csv: vec![file("KEY.csv", "0,same\n")],
        erb: vec![file("KEY.erd", "1,same\n")],
    };
    let mut project = load_project(&files, &CsvLoadOptions::default())
        .data
        .unwrap();
    let diagnostics = resolve_deferred_indices(
        &mut project,
        &[UserIndexRegistration {
            variable_name: "V".into(),
            source_stem: "KEY".into(),
            dimension: None,
            length: 2,
        }],
        &CsvLoadOptions::default(),
    );

    assert!(
        !project
            .static_data
            .deferred_indices
            .resolved
            .contains_key("V")
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == CsvDiagnosticCode::DuplicateUserIndex)
    );
}

#[test]
fn project_json_is_deterministic() {
    let files = ProjectFiles {
        csv: vec![file("ABL.csv", "1,one\n0,zero\n")],
        erb: vec![],
    };
    let report = load_project(&files, &CsvLoadOptions::default());
    let first = serde_json::to_string(&report).unwrap();
    let second = serde_json::to_string(&report).unwrap();

    assert_eq!(first, second);
    let decoded: erabasic_csv::CsvLoadReport = serde_json::from_str(&first).unwrap();
    assert_eq!(decoded, report);
}

#[test]
#[allow(clippy::too_many_lines)]
fn reference_cli_fixture_has_the_same_rust_projection() {
    let files = ProjectFiles {
        csv: vec![
            file(
                "GAMEBASE.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/GAMEBASE.CSV"
                ),
            ),
            file(
                "VariableSize.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/VariableSize.CSV"
                ),
            ),
            file(
                "ABL.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/ABL.CSV"
                ),
            ),
            file(
                "ABL.als",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/ABL.als"
                ),
            ),
            file(
                "ITEM.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/ITEM.CSV"
                ),
            ),
            file(
                "STR.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/STR.CSV"
                ),
            ),
            file(
                "CSTR.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/CSTR.CSV"
                ),
            ),
            file(
                "CHARA0.CSV",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/CHARA0.CSV"
                ),
            ),
            file(
                "_Replace.csv",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/_Replace.csv"
                ),
            ),
            file(
                "VarExt-oracle.csv",
                include_str!(
                    "../../../reference/emuera.em/emuera-reference-cli/tests/fixture/csv/VarExt-oracle.csv"
                ),
            ),
        ],
        erb: vec![],
    };
    let project = load_project(&files, &CsvLoadOptions::default())
        .data
        .unwrap();

    assert_eq!(project.schema.variable("ABL").unwrap().dimensions, [120]);
    assert_eq!(
        project.static_data.name_tables[&NameTableKind::Abl].lookup["later"],
        2
    );
    assert_eq!(project.static_data.item_prices[5], 120);
    assert_eq!(
        project.static_data.name_tables[&NameTableKind::Str].names[0].as_deref(),
        Some("initial text")
    );
    assert_eq!(project.static_data.characters[0].abl[&2], 5);
    assert_eq!(project.static_data.game_base.unique_code, 42);
}
