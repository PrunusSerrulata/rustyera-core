use era_protocol::{ProtocolBytes, ProtocolVersion};
use era_runtime_protocol::{
    ExtensionArgument, ExtensionArgumentStyle, ExtensionCallableKind, ExtensionDeclaration,
    ExtensionValueType, ExternalResource, FileCategory, FileChange, FilePayload,
    ImageMetadataResponse, ProjectAnalysisRequest, ReloadProject, RuntimeLogLevel, SubmittedFile,
};

use super::*;
use era_runtime_protocol::ConfigurationApplication;
use erabasic_data::LegacyEncoding;

#[test]
fn als_and_erd_are_data_inputs_with_profile_consistent_resolution() {
    for root in ["ERB/", ""] {
        let manifest = index_manifest(root);
        let build = build_project(&manifest, None);
        assert!(build.report.success, "{:?}", build.report.diagnostics);
        let artifact = build.artifact.unwrap();
        let indices = &artifact
            .artifact()
            .project_data
            .static_data
            .deferred_indices
            .resolved;
        assert_eq!(indices["BUFF"].entries["alias"], 10);
        assert_eq!(indices["BUFF"].entries["negative"], -1);
        assert_eq!(indices["BUFF"].entries["outside"], 300);
        assert_eq!(indices["BUFF"].canonical_names[&10], "main");
        assert!(artifact.artifact().source_map.sources.iter().all(|source| {
            !source.relative_path.to_ascii_lowercase().ends_with(".erd")
                && !source.relative_path.to_ascii_lowercase().ends_with(".als")
        }));
    }
}

fn index_manifest(root: &str) -> ProjectManifest {
    let file = |path: String, category, text: &str| SubmittedFile {
        relative_path: path,
        category,
        payload: FilePayload::Utf8(text.into()),
        content_hash: None,
    };
    ProjectManifest {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        project_revision: 1,
        files: vec![
            file(
                "reraconfig.toml".into(),
                FileCategory::Configuration,
                "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n",
            ),
            file(
                format!("{root}definitions.erh"),
                FileCategory::Erh,
                "#DIM BUFF,32\n",
            ),
            file(
                format!("{root}main.erb"),
                FileCategory::Erb,
                "@SYSTEM_TITLE\nRETURN\n",
            ),
            file(
                format!("{root}BUFF.erd"),
                FileCategory::Erd,
                "10,main\n11,other\n",
            ),
            file(
                format!("{root}BUFF.als"),
                FileCategory::Als,
                "10, alias \n-1,negative\n300,outside\n",
            ),
        ],
    }
}

#[test]
fn index_file_read_failures_and_non_utf8_payloads_are_not_ignored() {
    for category in [FileCategory::Als, FileCategory::Erd] {
        for payload in [
            FilePayload::Bytes(ProtocolBytes::new(vec![0xff])),
            FilePayload::ExternalResource(ExternalResource {
                byte_length: 10,
                image_metadata: None,
            }),
            FilePayload::IoError(era_runtime_protocol::FrontendIoError {
                kind: era_runtime_protocol::FrontendIoErrorKind::NotFound,
                message: "file disappeared during scan".into(),
                platform_code: None,
            }),
        ] {
            let mut manifest = index_manifest("ERB/");
            let file = manifest
                .files
                .iter_mut()
                .find(|file| file.category == category)
                .unwrap();
            // Required-input validation follows the category, not an attacker-controlled suffix.
            file.relative_path = "ERB/BUFF.nonstandard".into();
            file.payload = payload;
            let build = build_project(&manifest, None);
            assert!(!build.report.success, "{category:?} unexpectedly ignored");
            assert!(
                build.report.diagnostics.iter().any(|diagnostic| {
                    diagnostic.level == RuntimeLogLevel::Error
                        && diagnostic
                            .source
                            .as_ref()
                            .is_some_and(|source| source.relative_path.contains("BUFF"))
                }),
                "{:?}",
                build.report.diagnostics
            );
        }
    }
}

#[test]
fn optional_csv_not_found_remains_optional_but_index_errors_keep_both_roots() {
    let mut manifest = index_manifest("ERB/");
    let missing = FilePayload::IoError(era_runtime_protocol::FrontendIoError {
        kind: era_runtime_protocol::FrontendIoErrorKind::NotFound,
        message: "disappeared after scan".into(),
        platform_code: None,
    });
    manifest.files.push(SubmittedFile {
        relative_path: "CSV/optional.csv".into(),
        category: FileCategory::Csv,
        payload: missing.clone(),
        content_hash: None,
    });
    assert!(build_project(&manifest, None).report.success);
    manifest.files.last_mut().unwrap().relative_path = "CSV/BUFF.als".into();
    manifest.files.last_mut().unwrap().category = FileCategory::Als;
    manifest
        .files
        .iter_mut()
        .find(|file| file.relative_path == "ERB/BUFF.als")
        .unwrap()
        .payload = missing;
    let build = build_project(&manifest, None);
    assert!(!build.report.success);
    let paths = build
        .report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "runtime.frontend_io_error")
        .map(|diagnostic| diagnostic.source.as_ref().unwrap().relative_path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        paths,
        ["CSV/BUFF.als", "ERB/BUFF.als"].into_iter().collect()
    );
}

#[test]
fn initial_and_deferred_index_diagnostics_preserve_provenance_and_utf8_spans() {
    let mut manifest = index_manifest("ERB/");
    let contents = "10,别名\n坏,错误\n";
    manifest.files.last_mut().unwrap().payload = FilePayload::Utf8(contents.into());
    for (path, category, text) in [
        ("CSV/BUFF.csv", FileCategory::Csv, "12,csvprimary\n"),
        ("CSV/BUFF.als", FileCategory::Als, contents),
        ("CSV/FLAG.csv", FileCategory::Csv, "10,flagprimary\n"),
        ("CSV/FLAG.als", FileCategory::Als, contents),
    ] {
        manifest.files.push(SubmittedFile {
            relative_path: path.into(),
            category,
            payload: FilePayload::Utf8(text.into()),
            content_hash: None,
        });
    }
    let build = build_project(&manifest, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    for path in ["CSV/BUFF.als", "ERB/BUFF.als", "CSV/FLAG.als"] {
        let source = build
            .report
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.source.as_ref())
            .find(|source| source.relative_path == path && source.byte_start > 0)
            .unwrap_or_else(|| {
                panic!(
                    "missing full-path diagnostic for {path}: {:?}",
                    build.report.diagnostics
                )
            });
        assert_eq!(source.byte_start, "10,别名\n".len() as u64);
        assert_eq!(
            &contents[usize::try_from(source.byte_start).unwrap()
                ..usize::try_from(source.byte_end).unwrap()],
            "坏,错误"
        );
    }
}

#[test]
fn index_only_reload_matches_cold_loading_after_upsert_remove_and_readd() {
    let mut manifest = index_manifest("ERB/");
    let mut previous = build_project(&manifest, None);
    assert!(previous.report.success, "{:?}", previous.report.diagnostics);
    let alias_file = manifest.files.last().unwrap().clone();
    let erd_file = manifest.files[3].clone();
    let mut updated = alias_file.clone();
    updated.payload = FilePayload::Utf8("11,alias\n".into());
    for change in [
        FileChange::Upsert { file: updated },
        FileChange::Remove {
            category: FileCategory::Als,
            relative_path: alias_file.relative_path.clone(),
        },
        FileChange::Upsert { file: alias_file },
        FileChange::Remove {
            category: FileCategory::Erd,
            relative_path: erd_file.relative_path.clone(),
        },
        FileChange::Upsert { file: erd_file },
    ] {
        let next = apply_project_delta(
            &manifest,
            &ReloadProject {
                base_revision: manifest.project_revision,
                target_revision: manifest.project_revision + 1,
                changes: vec![change],
            },
        )
        .unwrap();
        let warm = build_project(&next, Some(&previous.incremental));
        let cold = build_project(&next, None);
        assert!(warm.report.success, "{:?}", warm.report.diagnostics);
        assert!(cold.report.success, "{:?}", cold.report.diagnostics);
        assert_eq!(
            warm.artifact.as_ref().unwrap().artifact().project_data,
            cold.artifact.as_ref().unwrap().artifact().project_data
        );
        assert_ne!(
            previous.snapshot.as_ref().unwrap().project_identity,
            warm.snapshot.as_ref().unwrap().project_identity
        );
        manifest = next;
        previous = warm;
    }
}

fn configuration(text: &str) -> SubmittedFile {
    SubmittedFile {
        relative_path: "emuera.config".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(text.into()),
        content_hash: None,
    }
}

#[test]
fn external_resource_metadata_avoids_startup_service_request() {
    let bytes = b"not transferred at startup";
    let digest = blake3::hash(bytes);
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "resources/image.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::ExternalResource(ExternalResource {
                    byte_length: bytes.len() as u64,
                    image_metadata: Some(ImageMetadataResponse {
                        width: 64,
                        height: 32,
                        format: "png".into(),
                        animated: false,
                    }),
                }),
                content_hash: Some(ProtocolBytes::new(digest.as_bytes().to_vec())),
            }],
        },
        None,
    );
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let graph = build.snapshot.unwrap().resource_graph;
    assert!(graph.metadata_requests().is_empty());
    assert_eq!(graph.embedded_project_bytes(), 0);
}

#[test]
fn invalid_external_resource_metadata_falls_back_to_lazy_service_detection() {
    let bytes = b"not transferred at startup";
    let digest = blake3::hash(bytes);
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "resources/image.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::ExternalResource(ExternalResource {
                    byte_length: bytes.len() as u64,
                    image_metadata: Some(ImageMetadataResponse {
                        width: 0,
                        height: 32,
                        format: "invalid".into(),
                        animated: false,
                    }),
                }),
                content_hash: Some(ProtocolBytes::new(digest.as_bytes().to_vec())),
            }],
        },
        None,
    );

    assert!(build.report.success, "{:?}", build.report.diagnostics);
    assert!(build.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "runtime.invalid_image_metadata"
            && diagnostic.level == RuntimeLogLevel::Warning
            && diagnostic.notification == DiagnosticNotification::LogOnly
    }));
    assert_eq!(
        build.snapshot.unwrap().resource_graph.metadata_requests(),
        vec![("resources/image.png".into(), *digest.as_bytes())]
    );
}

#[test]
fn semantic_configuration_is_applied_and_retired_settings_are_reported_once() {
    let mut diagnostics = Vec::new();
    let config = parse_configuration(
        &[
            configuration(
                "\u{feff}Sort filenames:YES\nIgnore case:NO\nMake autosaves:NO\nEnable undo with ctrl-z:YES\nAllow long input by mouse for ONEINPUT:YES\nUse the binary format for saving data:YES\nCompress save data:YES\nSave data count per page:30\nFont size:20\nLine height:22\nAllow CALL on event functions:YES\nAllow arguments omission for user functions:YES\nAuto TOSTR conversion for user function arguments:YES\nDo not process triple symbols inside FORM:YES\nImitate ERD to VARSIZE dimension specification:YES\nText color:1,2,3\nDefault ANSI encoding:KOREAN\nフォント名:Test\n",
            ),
            SubmittedFile {
                relative_path: "setting.json".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(r#"{"UseNewRandom":true}"#.into()),
                content_hash: None,
            },
        ],
        &mut diagnostics,
    )
    .semantic;
    assert!(config.csv.sort_with_filename);
    assert!(!config.csv.ignore_case);
    assert!(!config.analyzer.ignore_case);
    assert!(!config.auto_save);
    assert!(config.ctrl_z_enabled);
    assert!(config.allow_long_input_by_activation);
    assert!(config.save_in_binary);
    assert!(config.compress_save);
    assert_eq!(config.save_slot_count, 30);
    assert_eq!(config.money_label, "$");
    assert!(config.money_first);
    assert_eq!(config.maximum_shop_items, 100);
    assert_eq!(config.font_size, 20);
    assert_eq!(config.line_height, 22);
    assert!(config.analyzer.compatible_call_event);
    assert!(config.analyzer.compatible_function_argument_optional);
    assert!(config.analyzer.compatible_function_argument_auto_convert);
    assert!(config.analyzer.ignore_triple_symbols);
    assert!(config.analyzer.varsize_dimension_is_one_based);
    assert_eq!(config.analyzer.default_foreground_color, 0x0001_0203);
    assert_eq!(config.legacy_encoding, LegacyEncoding::Korean);
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "runtime.legacy_configuration_migration"
                    && diagnostic.message.contains("UseNewRandom")
            })
            .count(),
        1
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("UseNewRandom") && diagnostic.level == RuntimeLogLevel::Info
    }));
}

#[test]
fn setting_json_is_only_read_to_report_retired_reference_settings() {
    let mut diagnostics = Vec::new();
    let config = parse_configuration(
        &[SubmittedFile {
            relative_path: "setting.json".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(
                r#"{"UseNewRandom":true,"UseMouse":false,"AllowLongInputByMouse":true,"WindowWidth":1200,"FontSize":21,"LineHeight":19,"CompatiCallEvent":true,"CompatiFuncArgOptional":true,"CompatiFuncArgAutoConvert":true}"#.into(),
            ),
            content_hash: None,
        }],
        &mut diagnostics,
    )
    .semantic;
    assert!(!config.allow_long_input_by_activation);
    assert_eq!(config.font_size, 18);
    assert_eq!(config.line_height, 19);
    assert!(!config.analyzer.compatible_call_event);
    assert!(!config.analyzer.compatible_function_argument_optional);
    assert!(!config.analyzer.compatible_function_argument_auto_convert);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "runtime.legacy_configuration_migration"
            && diagnostic.message.contains("UseNewRandom")
    }));
}

#[test]
fn reraconfig_takes_priority_and_legacy_sources_generate_it_only_when_absent() {
    let legacy = configuration("Font size:21\n");
    let rera = SubmittedFile {
        relative_path: "reraconfig.toml".into(),
        category: FileCategory::Configuration,
        payload: FilePayload::Utf8(
            "[meta]\r\nschema_version = 4\r\n[text]\r\nfont_size = 24\r\n".into(),
        ),
        content_hash: None,
    };
    let mut diagnostics = Vec::new();
    let parsed = parse_configuration(&[legacy.clone(), rera], &mut diagnostics);
    assert_eq!(
        parsed.semantic.values.get_code("FontSize"),
        Some(&era_config::ConfigValue::Integer(24))
    );
    assert!(parsed.generated_source.is_none());

    let migrated = parse_configuration(&[legacy], &mut diagnostics);
    let generated = migrated
        .generated_source
        .expect("legacy source generates TOML");
    assert!(generated.contains("font_size = 21"));
    assert!(!generated.contains('\r'));
}

#[test]
fn schema_v1_reraconfig_is_returned_for_atomic_client_persistence() {
    let mut diagnostics = Vec::new();
    let parsed = parse_configuration(
        &[SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8(
                "[meta]\nschema_version = 1\n[text]\ndrawing_method = \"winapi\"\nfont_size = 20\n"
                    .into(),
            ),
            content_hash: None,
        }],
        &mut diagnostics,
    );
    let generated = parsed
        .generated_source
        .expect("schema version 1 must be persisted as version 4");
    assert!(generated.contains("schema_version = 4"));
    assert!(generated.contains("font_size = 20"));
    assert!(!generated.contains("drawing_method"));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "runtime.reraconfig_upgraded"
            && diagnostic.level == RuntimeLogLevel::Info
            && diagnostic.message.contains("TextDrawingMode")
    }));
}

#[test]
fn new_frontend_settings_have_only_the_requested_hot_applicability() {
    assert_eq!(
        profile_application("AudioVolume", ConfigurationClientProfile::Browser),
        ConfigurationApplication::Hot
    );
    assert_eq!(
        profile_application("AudioVolume", ConfigurationClientProfile::Tauri),
        ConfigurationApplication::Hot
    );
    assert!(!era_config::tui_configurable("AudioVolume"));
    for code in ["ReplaceFullWidthSpaces", "CharacterWidthMode"] {
        assert_eq!(
            profile_application(code, ConfigurationClientProfile::Tui),
            ConfigurationApplication::Hot
        );
        assert_eq!(
            profile_application(code, ConfigurationClientProfile::Browser),
            ConfigurationApplication::Hot
        );
    }
    let audio = era_config::catalog()
        .into_iter()
        .find(|spec| spec.code == "AudioVolume")
        .unwrap();
    assert!(!audio.clients.contains(&era_config::ConfigClient::Tui));
    let width_mode = era_config::catalog()
        .into_iter()
        .find(|spec| spec.code == "CharacterWidthMode")
        .unwrap();
    assert_eq!(
        width_mode.effect,
        era_config::ConfigEffect::PortableSemantic
    );
    assert!(
        width_mode
            .clients
            .contains(&era_config::ConfigClient::Runtime)
    );
}

#[test]
fn invalid_reraconfig_volume_is_rejected_by_the_project_boundary() {
    let mut diagnostics = Vec::new();
    let parsed = parse_configuration(
        &[SubmittedFile {
            relative_path: "reraconfig.toml".into(),
            category: FileCategory::Configuration,
            payload: FilePayload::Utf8("[audio]\nvolume = 101\n".into()),
            content_hash: None,
        }],
        &mut diagnostics,
    );
    assert_eq!(
        parsed.semantic.values.get_code("AudioVolume"),
        Some(&era_config::ConfigValue::Integer(100))
    );
    assert!(diagnostics.iter().any(|item| {
        item.code == "runtime.invalid_reraconfig" && item.level == RuntimeLogLevel::Error
    }));
}

#[test]
fn only_directory_components_with_hash_receive_priority() {
    assert!(path_has_priority_directory("ERB/#boot/first.erb"));
    assert!(path_has_priority_directory("ERB/a#early/first.erb"));
    assert!(!path_has_priority_directory("ERB/ordinary/#function.erb"));
    assert!(!path_has_priority_directory("root.erb"));
}

#[test]
fn reference_file_order_places_parent_files_before_child_directories() {
    let mut paths = [
        "ERB/events/diary/DIARY.ERH",
        "ERB/COLOREDMAPS/COLOREDOPTION.ERH",
        "ERB/DIM.ERH",
        "ERB/A.ERH",
        "ERB/events/ROOT.ERH",
    ];
    paths.sort_by(|left, right| compare_reference_file_paths(left, right));
    assert_eq!(
        paths,
        [
            "ERB/A.ERH",
            "ERB/DIM.ERH",
            "ERB/COLOREDMAPS/COLOREDOPTION.ERH",
            "ERB/events/ROOT.ERH",
            "ERB/events/diary/DIARY.ERH",
        ]
    );
}

#[test]
fn category_root_prefix_is_removed_only_at_internal_loader_boundary() {
    assert_eq!(
        category_relative_path("CSV/_Rename.csv", "CSV"),
        "_Rename.csv"
    );
    assert_eq!(
        category_relative_path("csv/sub/data.csv", "CSV"),
        "sub/data.csv"
    );
    assert_eq!(category_relative_path("ERB/main.erb", "ERB"), "main.erb");
    assert_eq!(
        category_relative_path("scripts/main.erb", "ERB"),
        "scripts/main.erb"
    );
    assert_eq!(category_relative_path("CSV.csv", "CSV"), "CSV.csv");
}

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

#[test]
fn analysis_selection_checks_unreachable_code_without_loading_a_project() {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 9,
        files: vec![
            SubmittedFile {
                relative_path: "good.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@UNUSED\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "bad.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("this is not valid at top level".into()),
                content_hash: None,
            },
        ],
    };
    let report = analyze_submitted_project_with_extensions(
        &ProjectAnalysisRequest {
            manifest,
            selected_erb_paths: vec!["good.erb".into()],
            debug_mode: true,
        },
        &[],
        ConfigurationClientProfile::Reference,
    );
    assert!(report.success, "{:?}", report.diagnostics);
    assert_eq!(report.analyzed_erb_paths, vec!["good.erb"]);
}

#[test]
fn portable_extensions_participate_in_analysis_and_deterministic_host_lowering() {
    let declaration = ExtensionDeclaration {
        id: "example.echo.v1".into(),
        era_name: "EXT_ECHO".into(),
        kind: ExtensionCallableKind::Function,
        arguments: vec![ExtensionArgument {
            value_type: ExtensionValueType::String,
            mutable: false,
            optional: false,
        }],
        variadic: false,
        return_type: ExtensionValueType::String,
        argument_style: ExtensionArgumentStyle::Normal,
        operation: "example.echo".into(),
        operation_version: ProtocolVersion::new(1, 0),
    };
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8(
                "@SYSTEM_TITLE\nRESULTS '= EXT_ECHO(\"ok\")\nRETURN\n".into(),
            ),
            content_hash: None,
        }],
    };
    let build = build_project_with_extensions(&manifest, None, None, &[declaration]);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let artifact = build.artifact.unwrap();
    assert!(
        artifact.artifact().host_imports.iter().any(|import| {
            import.import.namespace == "rustyera.extension" && import.import.name == "example.echo"
        }),
        "{:#?}",
        artifact.artifact().host_imports
    );
}

#[test]
fn query_visible_configuration_participates_in_project_identity() {
    let manifest = |font_size| ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "emuera.config".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(format!("Font size:{font_size}\n")),
                content_hash: None,
            },
        ],
    };
    let first = build_project(&manifest(18), None);
    let second = build_project(&manifest(19), None);
    assert!(first.report.success, "{:?}", first.report.diagnostics);
    assert!(second.report.success, "{:?}", second.report.diagnostics);
    assert_ne!(
        first.snapshot.unwrap().project_identity,
        second.snapshot.unwrap().project_identity
    );
}

#[test]
fn search_subdirectories_configuration_loads_nested_character_templates() {
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "ERB/TITLE.ERB".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nADDCHARA 0\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CSV/characters/Chara0.csv".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("NO,0\nNAME,Master\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("\u{feff}サブディレクトリを検索する:YES\n".into()),
                    content_hash: None,
                },
            ],
        },
        None,
    );
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let artifact = build.artifact.expect("compiled artifact");

    assert!(
        artifact
            .artifact()
            .project_data
            .static_data
            .characters
            .iter()
            .any(|template| template.no == 0 && template.name == "Master")
    );
}

#[test]
fn runtime_project_build_retains_a_compact_serializable_incremental_cache() {
    use std::fmt::Write as _;

    let mut source = String::new();
    for index in 0..128 {
        write!(source, "@FUNCTION_{index}\nRESULT = {index}\nRETURN\n").unwrap();
    }
    let build = build_project(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source),
                content_hash: None,
            }],
        },
        None,
    );
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let encoded = serde_json::to_vec(&build.incremental).unwrap();
    let decoded: IncrementalState = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(decoded, build.incremental);
    assert_eq!(decoded.cached_function_count(), 128);
    let encoded = String::from_utf8(encoded).unwrap();
    assert!(!encoded.contains("\"opcode\""));
    assert!(!encoded.contains("\"project_data\""));
}

#[test]
fn project_build_reports_real_workload_progress() {
    use std::sync::{Arc, Mutex};

    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let reporter = ProjectProgressReporter::new(move |progress| {
        sink.lock().unwrap().push(progress);
    });
    let build = build_project_with_extensions_and_progress(
        &ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
        None,
        &[],
        ConfigurationClientProfile::Reference,
        Some(&reporter),
    );
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let observed = observed.lock().unwrap();
    for stage in [
        ProjectProgressStage::Normalizing,
        ProjectProgressStage::LoadingData,
        ProjectProgressStage::Parsing,
        ProjectProgressStage::Analyzing,
        ProjectProgressStage::Compiling,
        ProjectProgressStage::Finalizing,
        ProjectProgressStage::Validating,
        ProjectProgressStage::Preparing,
    ] {
        let values = observed
            .iter()
            .filter(|progress| progress.stage == stage)
            .collect::<Vec<_>>();
        assert!(!values.is_empty(), "missing {stage:?} progress");
        assert_eq!(values[0].completed, 0, "{stage:?} did not start at zero");
        let final_value = values.last().unwrap();
        assert_eq!(
            final_value.completed, final_value.total,
            "{stage:?} did not complete"
        );
        assert!(
            values
                .windows(2)
                .all(|pair| pair[0].completed <= pair[1].completed),
            "{stage:?} regressed"
        );
    }
}

#[test]
fn experimental_profile_is_preserved_and_conflicting_configuration_is_rejected() {
    let snake = erabasic_compat::CompatibilityIdentity::for_profile(
        erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
    );
    let manifest = ProjectManifest {
        project_revision: 1,
        compatibility: snake.clone(),
        files: vec![
            SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8("[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
        ],
    };
    let built = build_project(&manifest, None);
    assert!(built.report.success, "{:?}", built.report.diagnostics);
    assert_eq!(built.report.compatibility.as_ref(), Some(&snake));
    assert_eq!(
        built
            .artifact
            .as_ref()
            .unwrap()
            .artifact()
            .manifest
            .compatibility,
        snake
    );
    assert!(built.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "runtime.experimental_compatibility_profile"
            && diagnostic.context.as_ref().unwrap().identity.as_ref() == Some(&snake)
    }));
    let compact = build_owned_project_with_extensions_and_progress(
        manifest.clone(),
        None,
        None,
        &[],
        ConfigurationClientProfile::Reference,
        false,
        None,
    );
    let snapshot = compact.snapshot.unwrap();
    assert!(
        matches!(&snapshot.manifest.files[0].payload, FilePayload::Utf8(source) if source.contains("emuera.skia.snake"))
    );
    let mut conflicting = manifest;
    conflicting.compatibility = era_runtime_protocol::CompatibilityIdentity::default();
    let rejected = build_project(&conflicting, None);
    assert!(!rejected.report.success);
    assert!(rejected.artifact.is_none());
    assert_eq!(
        rejected.report.diagnostics[0].code,
        "runtime.compatibility_identity_mismatch"
    );
}
