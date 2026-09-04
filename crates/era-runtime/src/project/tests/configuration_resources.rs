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
                "\u{feff}Sort filenames:YES\nIgnore case:NO\nMake autosaves:NO\nEnable undo with ctrl-z:YES\nAllow long input by mouse for ONEINPUT:YES\nUse the binary format for saving data:YES\nCompress save data:YES\nSave data count per page:30\nFont size:20\nLine height:22\nAllow CALL on event functions:YES\nAllow arguments omission for user functions:YES\nTreat snake excess user arguments as errors:YES\nAuto TOSTR conversion for user function arguments:YES\nDo not process triple symbols inside FORM:YES\nImitate behavior for RAND:YES\nDo not auto-complete arguments for character variables:YES\nImitate ERD to VARSIZE dimension specification:YES\nDisable BEFORE_ERROR/THROW events:YES\nText color:1,2,3\nDefault ANSI encoding:KOREAN\nフォント名:Test\n",
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
    assert!(config.analyzer.strict_user_call_arguments);
    assert!(config.analyzer.compatible_function_argument_auto_convert);
    assert!(config.analyzer.ignore_triple_symbols);
    assert!(config.analyzer.compatible_rand);
    assert!(config.analyzer.system_no_target);
    assert!(config.analyzer.varsize_dimension_is_one_based);
    assert!(config.analyzer.disable_before_error_throw);
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
            "[meta]\r\nschema_version = 5\r\n[text]\r\nfont_size = 24\r\n".into(),
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
    assert!(generated.contains("schema_version = 5"));
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
