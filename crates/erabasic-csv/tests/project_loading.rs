use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
use erabasic_csv::{
    CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions, FilePayload, FrontendFile,
    FrontendIoError, FrontendIoErrorKind, ProjectFiles, load_project, load_project_owned,
    resolve_deferred_indices,
};
use erabasic_data::{CharacterSelection, NameTableKind, UserIndexRegistration};

fn file(path: &str, content: &str) -> FrontendFile {
    FrontendFile {
        source_path: None,
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

fn profile_options(profile: CompatibilityProfileId) -> CsvLoadOptions {
    CsvLoadOptions {
        compatibility: CompatibilityIdentity::for_profile(profile),
        ..CsvLoadOptions::default()
    }
}

#[test]
fn owned_loader_preserves_borrowed_results_and_diagnostics() {
    let files = ProjectFiles {
        csv: vec![
            file("GAMEBASE.CSV", "コード,42\nタイトル,所有権\n"),
            file("../invalid.csv", "0,ignored\n"),
            file("nested/ITEM.csv", "1,potion,50\n"),
        ],
        erb: vec![file("nested/lookup.erd", "1,name\n")],
    };
    let options = full_project_options();
    assert_eq!(
        load_project_owned(files.clone(), &options),
        load_project(&files, &options),
    );
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
                source_path: None,
                relative_path: "missing.csv".into(),
                payload: FilePayload::IoError(FrontendIoError {
                    kind: FrontendIoErrorKind::NotFound,
                    message: "missing".into(),
                }),
            },
            FrontendFile {
                source_path: None,
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
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let options = profile_options(profile);
        let mut project = load_project(&files, &options).data.unwrap();
        let diagnostics = resolve_deferred_indices(
            &mut project,
            &[UserIndexRegistration {
                variable_name: "V".into(),
                source_stem: "KEY".into(),
                dimension: None,
                length: 2,
            }],
            &options,
        );
        assert!(
            !project
                .static_data
                .deferred_indices
                .resolved
                .contains_key("V")
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == CsvDiagnosticCode::DuplicateUserIndex
                && diagnostic.severity == CsvDiagnosticSeverity::Fatal
        }));
    }
}

#[test]
fn user_aliases_preserve_signed_indices_and_primary_names_only_for_snake() {
    let files = ProjectFiles {
        csv: vec![
            file("BUFF.csv", "0,primary\n10,z_primary\n"),
            file(
                "BUFF.als",
                "10, a_alias \n11,eleven\n300,far\n-1,negative\n-2147483648,min\n2147483647,max\n11,primary\n11,eleven\n300,eleven\n1,   \n",
            ),
        ],
        erb: vec![],
    };
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let options = profile_options(profile);
        let mut project = load_project(&files, &options).data.unwrap();
        let diagnostics = resolve_deferred_indices(
            &mut project,
            &[UserIndexRegistration {
                variable_name: "BUFF".into(),
                source_stem: "BUFF".into(),
                dimension: None,
                length: 50,
            }],
            &options,
        );
        assert!(diagnostics.is_empty());
        let resolved = &project.static_data.deferred_indices.resolved["BUFF"];
        assert_eq!(resolved.entries["primary"], 0);
        assert_eq!(resolved.canonical_names[&10], "z_primary");
        if profile == CompatibilityProfileId::EmueraSkiaSnake {
            assert_eq!(resolved.entries["a_alias"], 10);
            assert_eq!(resolved.entries["eleven"], 11);
            assert_eq!(resolved.entries["far"], 300);
            assert_eq!(resolved.entries["negative"], -1);
            assert_eq!(resolved.entries["min"], i64::from(i32::MIN));
            assert_eq!(resolved.entries["max"], i64::from(i32::MAX));
            assert_eq!(resolved.canonical_names[&-1], "negative");
            assert!(!resolved.entries.contains_key(""));
            assert!(!resolved.entries.contains_key(" a_alias "));
        } else {
            assert_eq!(resolved.entries.len(), 2);
        }
        let encoded = serde_json::to_string(&project).unwrap();
        let decoded: erabasic_data::ProjectData = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, project);
    }
}

#[test]
fn deferred_aliases_follow_primary_group_order_and_same_directory_root() {
    let files = ProjectFiles {
        csv: vec![
            file("buff.csv", "0,main_from_csv\n"),
            file("buff.als", "300,shared\n11,csv_only\n"),
        ],
        erb: vec![
            file("z/buff.erd", "0,erd_z\n"),
            file("z/buff.als", "300,shared\n"),
            file("a/buff.erd", "0,erd_a\n"),
            file(
                "a/buff.als",
                "11,shared\n1,main_from_csv\n300,z_alias\n300,a_alias\n",
            ),
            file("buff.erd", "0,erd_root\n"),
            file("buff.als", "10,shared\n10,erb_only\n"),
            file("orphan/buff.als", "1,orphan\n"),
        ],
    };
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let options = profile_options(profile);
        let mut project = load_project(&files, &options).data.unwrap();
        let diagnostics = resolve_deferred_indices(
            &mut project,
            &[UserIndexRegistration {
                variable_name: "BUFF".into(),
                source_stem: "BUFF".into(),
                dimension: None,
                length: 50,
            }],
            &options,
        );
        assert!(diagnostics.is_empty());
        let resolved = &project.static_data.deferred_indices.resolved["BUFF"];
        assert_eq!(resolved.entries["main_from_csv"], 0);
        if profile == CompatibilityProfileId::EmueraSkiaSnake {
            assert_eq!(resolved.canonical_names[&0], "erd_a");
            assert_eq!(resolved.canonical_names[&300], "z_alias");
            assert_eq!(resolved.entries["shared"], 11);
            assert_eq!(resolved.entries["csv_only"], 11);
            assert_eq!(resolved.entries["erb_only"], 10);
            assert!(!resolved.entries.contains_key("orphan"));
        } else {
            assert_eq!(resolved.canonical_names[&0], "main_from_csv");
            assert_eq!(resolved.entries.len(), 4);
        }
    }
}

#[test]
fn multidimensional_user_aliases_require_registered_primary_tables_and_use_erd() {
    let files = ProjectFiles {
        csv: vec![file("ORPHAN.als", "1,ignored\n")],
        erb: vec![
            file("columns/COLUMNDIV@2.ERD", "10,column\n"),
            file("columns/COLUMNDIV@2.als", "11,column_alias\n"),
            file("matrix/SEMEN_MATRIX@2.ERD", "11,matrix\n"),
            file("matrix/SEMEN_MATRIX@2.als", "300,matrix_alias\n"),
        ],
    };
    let registrations = [
        UserIndexRegistration {
            variable_name: "COLUMNDIV".into(),
            source_stem: "COLUMNDIV@2".into(),
            dimension: Some(2),
            length: 12,
        },
        UserIndexRegistration {
            variable_name: "SEMEN_MATRIX".into(),
            source_stem: "SEMEN_MATRIX@2".into(),
            dimension: Some(2),
            length: 12,
        },
        UserIndexRegistration {
            variable_name: "ORPHAN".into(),
            source_stem: "ORPHAN".into(),
            dimension: None,
            length: 12,
        },
    ];
    let options = profile_options(CompatibilityProfileId::EmueraSkiaSnake);
    let mut project = load_project(&files, &options).data.unwrap();
    assert!(resolve_deferred_indices(&mut project, &registrations, &options).is_empty());
    let resolved = &project.static_data.deferred_indices.resolved;
    assert_eq!(resolved["COLUMNDIV@2"].entries["column_alias"], 11);
    assert_eq!(resolved["SEMEN_MATRIX@2"].entries["matrix_alias"], 300);
    assert!(!resolved.contains_key("ORPHAN"));

    let disabled = CsvLoadOptions {
        use_erd: false,
        ..options
    };
    let mut project = load_project(&files, &disabled).data.unwrap();
    assert!(project.static_data.deferred_indices.groups.is_empty());
    assert!(resolve_deferred_indices(&mut project, &registrations, &disabled).is_empty());
    assert!(project.static_data.deferred_indices.resolved.is_empty());
}

#[test]
fn builtin_alias_duplicate_recovery_and_trimming_are_profile_scoped() {
    let files = ProjectFiles {
        csv: vec![
            file("CFLAG.csv", "0,primary\n"),
            file(
                "CFLAG.als",
                "10, trimmed \n11,shared\n11,another\n300,shared\n300,later\n-1,negative\n11,primary\n",
            ),
        ],
        erb: vec![],
    };
    for profile in [
        CompatibilityProfileId::EmueraEm,
        CompatibilityProfileId::EmueraSkiaSnake,
    ] {
        let report = load_project(&files, &profile_options(profile));
        let project = report.data.unwrap();
        let table = &project.static_data.name_tables[&NameTableKind::Cflag];
        assert_eq!(table.lookup["primary"], 0);
        assert_eq!(table.lookup["shared"], 11);
        let duplicate = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == CsvDiagnosticCode::DuplicateAlias)
            .unwrap();
        if profile == CompatibilityProfileId::EmueraSkiaSnake {
            assert_eq!(table.lookup["trimmed"], 10);
            assert_eq!(table.lookup["another"], 11);
            assert_eq!(table.lookup["later"], 300);
            assert_eq!(table.lookup["negative"], -1);
            assert_eq!(duplicate.severity, CsvDiagnosticSeverity::Warning);
            assert!(
                !report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| { diagnostic.code == CsvDiagnosticCode::DuplicateIndex })
            );
        } else {
            assert_eq!(table.lookup[" trimmed "], 10);
            assert!(!table.lookup.contains_key("later"));
            assert!(!table.lookup.contains_key("negative"));
            assert_eq!(duplicate.severity, CsvDiagnosticSeverity::Error);
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|diagnostic| { diagnostic.code == CsvDiagnosticCode::DuplicateIndex })
            );
        }
    }
}

#[test]
fn deferred_primary_duplicates_and_invalid_aliases_keep_distinct_error_paths() {
    let files = ProjectFiles {
        csv: vec![
            file(
                "BUFF.csv",
                "1,discarded\n1,kept\n-1,negative_main\n50,too_large_main\n",
            ),
            file(
                "BUFF.als",
                "broken\n2147483648,too_large_integer\n11,valid\n",
            ),
        ],
        erb: vec![],
    };
    let options = profile_options(CompatibilityProfileId::EmueraSkiaSnake);
    let registration = UserIndexRegistration {
        variable_name: "BUFF".into(),
        source_stem: "BUFF".into(),
        dimension: None,
        length: 50,
    };
    let mut project = load_project(&files, &options).data.unwrap();
    let diagnostics = resolve_deferred_indices(&mut project, &[registration], &options);
    let resolved = &project.static_data.deferred_indices.resolved["BUFF"];
    assert_eq!(
        resolved.entries,
        [("kept".into(), 1), ("valid".into(), 11)]
            .into_iter()
            .collect()
    );
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        [
            CsvDiagnosticCode::DuplicateIndex,
            CsvDiagnosticCode::IndexOutOfRange,
            CsvDiagnosticCode::IndexOutOfRange,
            CsvDiagnosticCode::MissingComma,
            CsvDiagnosticCode::InvalidInteger,
        ]
    );
}

#[test]
fn missing_submitted_alias_and_erd_inputs_are_reported() {
    let missing = |path: &str| FrontendFile {
        source_path: None,
        relative_path: path.into(),
        payload: FilePayload::IoError(FrontendIoError {
            kind: FrontendIoErrorKind::NotFound,
            message: "file disappeared after scanning".into(),
        }),
    };
    let files = ProjectFiles {
        csv: vec![missing("BUFF.als"), missing("optional.csv")],
        erb: vec![missing("nested/BUFF.ERD")],
    };
    let report = load_project(&files, &CsvLoadOptions::default());
    let failed_paths: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == CsvDiagnosticCode::IoError)
        .map(|diagnostic| diagnostic.source.as_ref().unwrap().relative_path.as_str())
        .collect();
    assert_eq!(failed_paths, ["BUFF.als", "nested/BUFF.ERD"]);
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
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/GAMEBASE.CSV"),
            ),
            file(
                "VariableSize.CSV",
                include_str!(
                    "../../../tools/runtime-tester/fixture-declaration/csv/VariableSize.CSV"
                ),
            ),
            file(
                "ABL.CSV",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/ABL.CSV"),
            ),
            file(
                "ABL.als",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/ABL.als"),
            ),
            file(
                "ITEM.CSV",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/ITEM.CSV"),
            ),
            file(
                "STR.CSV",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/STR.CSV"),
            ),
            file(
                "CSTR.CSV",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/CSTR.CSV"),
            ),
            file(
                "CHARA0.CSV",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/CHARA0.CSV"),
            ),
            file(
                "_Replace.csv",
                include_str!("../../../tools/runtime-tester/fixture-declaration/csv/_Replace.csv"),
            ),
            file(
                "VarExt-oracle.csv",
                include_str!(
                    "../../../tools/runtime-tester/fixture-declaration/csv/VarExt-oracle.csv"
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
