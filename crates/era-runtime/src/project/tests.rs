use era_protocol::ProtocolVersion;
use era_runtime_protocol::{
    ExtensionArgument, ExtensionArgumentStyle, ExtensionCallableKind, ExtensionDeclaration,
    ExtensionValueType, FileCategory, FileChange, FilePayload, ProjectAnalysisRequest,
    ReloadProject, RuntimeLogLevel, SubmittedFile,
};

use super::*;

#[test]
fn only_erd_sources_are_forwarded_to_the_deferred_index_loader() {
    assert!(is_deferred_index_source("ERB/index.ERD"));
    assert!(is_deferred_index_source("nested/index.erd"));
    assert!(!is_deferred_index_source("ERB/main.ERB"));
    assert!(!is_deferred_index_source("ERB/header.ERH"));
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
            "[meta]\r\nschema_version = 2\r\n[text]\r\nfont_size = 24\r\n".into(),
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
        .expect("schema version 1 must be persisted as version 2");
    assert!(generated.contains("schema_version = 2"));
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
