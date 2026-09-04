#[test]
fn project_source_location_reports_zero_based_utf8_byte_line_and_column() {
    let text = "最初の行\n\tRESULT = 未定義\n";
    let byte_start = text.find("未定義").unwrap();
    let location = project_source_location(
        "ERB/main.erb".into(),
        byte_start,
        byte_start + "未定義".len(),
        None,
        Some(text),
    );

    assert_eq!(location.relative_path, "ERB/main.erb");
    assert_eq!(location.line, Some(1));
    assert_eq!(location.byte_column, Some(10));
    assert_eq!(location.byte_start, u64::try_from(byte_start).unwrap());
    assert_eq!(
        location.byte_end,
        u64::try_from(byte_start + "未定義".len()).unwrap()
    );
}

#[test]
fn project_build_populates_analyzer_diagnostic_line_and_byte_column() {
    let text = "@SYSTEM_TITLE\nUNKNOWN 1\nRETURN\n";
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "ERB/bad.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(text.into()),
                content_hash: None,
            }],
        },
        None,
    );

    let diagnostic = build
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "analyzer.unknowninstruction")
        .expect("unknown-instruction diagnostic");
    let source = diagnostic.source.as_ref().expect("source location");
    assert_eq!(source.relative_path, "ERB/bad.erb");
    assert_eq!(source.line, Some(1));
    assert_eq!(source.byte_column, Some(0));
    assert_eq!(
        source.byte_start,
        u64::try_from(text.find("UNKNOWN").unwrap()).unwrap()
    );
}

#[test]
fn owned_project_build_maps_compiler_errors_to_utf8_byte_columns() {
    let text = "@SYSTEM_TITLE\nPRINTL 日本語\nRESULT = GETNUMB(\"TARGET\")\nRETURN\n";
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "ERB/compiler-error.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(text.into()),
                content_hash: None,
            }],
        },
        None,
    );

    let diagnostic = build
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "compiler.unsupportedconstruct")
        .expect("unsupported compiler diagnostic");
    let source = diagnostic
        .source
        .as_ref()
        .expect("compiler source location");
    assert_eq!(source.relative_path, "ERB/compiler-error.erb");
    assert_eq!(source.line, Some(2));
    assert_eq!(source.byte_column, Some(9));
    assert_eq!(
        source.byte_start,
        u64::try_from(text.find("GETNUMB").unwrap()).unwrap()
    );
}

#[test]
fn project_load_report_projects_only_defined_gamebase_information() {
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "CSV/GameBase.csv".into(),
                category: FileCategory::Csv,
                payload: FilePayload::Utf8(
                    "タイトル,Demo\n作者,Author\nバージョン,1001\n製作年,2026\n追加情報,Notes\n"
                        .into(),
                ),
                content_hash: None,
            }],
        },
        None,
    );

    assert!(build.report.success, "{:?}", build.report.diagnostics);
    assert_eq!(
        build.report.game_information,
        Some(Box::new(ProjectGameInformation {
            title: Some("Demo".into()),
            author: Some("Author".into()),
            version: Some("1.001".into()),
            year: Some("2026".into()),
            information: Some("Notes".into()),
        }))
    );

    let missing = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 2,
            files: vec![SubmittedFile {
                relative_path: "GAMEBASE.CSV".into(),
                category: FileCategory::Csv,
                payload: FilePayload::Utf8("タイトル,Demo\n作者,   \n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    assert_eq!(
        missing.report.game_information,
        Some(Box::new(ProjectGameInformation {
            title: Some("Demo".into()),
            author: None,
            version: None,
            year: None,
            information: None,
        }))
    );
}

#[test]
fn focused_eratw_system_slices_exercise_runtime_owned_save_flows() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/eraTW/ERB");
    for (relative, required) in [
        ("TITLE.ERB", &["@SYSTEM_TITLE", "LOADGAME"][..]),
        ("SHOP関連/SHOP.ERB", &["SAVEGAME", "LOADGAME"]),
        ("SYSTEM.ERB", &["@EVENTLOAD"]),
        ("ステータス表示関連/INFO.ERB", &["@SAVEINFO", "PUTFORM"]),
    ] {
        // This is a read-only corpus audit; functional behavior is covered by the small
        // controller fixtures so the 80+ MiB real project is never a default test input.
        let source = std::fs::read_to_string(root.join(relative)).expect("UTF-8 eraTW slice");
        for needle in required {
            assert!(
                source.contains(needle),
                "{relative} no longer contains {needle}"
            );
        }
    }
}

#[test]
fn project_delta_is_monotonic_normalized_and_unique() {
    let current = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 4,
        files: vec![SubmittedFile {
            relative_path: "ERB\\main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("old".into()),
            content_hash: None,
        }],
    };
    let updated = apply_project_delta(
        &current,
        &ReloadProject {
            base_revision: 4,
            target_revision: 5,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "ERB/./main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("new".into()),
                    content_hash: None,
                },
            }],
        },
    )
    .unwrap();
    assert_eq!(updated.project_revision, 5);
    assert_eq!(updated.files.len(), 1);
    assert_eq!(updated.files[0].relative_path, "ERB/main.erb");
    assert!(matches!(updated.files[0].payload, FilePayload::Utf8(ref value) if value == "new"));

    let duplicate = ReloadProject {
        base_revision: 4,
        target_revision: 5,
        changes: vec![
            FileChange::Remove {
                category: FileCategory::Erb,
                relative_path: "ERB/main.erb".into(),
            },
            FileChange::Remove {
                category: FileCategory::Erb,
                relative_path: "erb\\MAIN.erb".into(),
            },
        ],
    };
    assert!(apply_project_delta(&current, &duplicate).is_err());
}
