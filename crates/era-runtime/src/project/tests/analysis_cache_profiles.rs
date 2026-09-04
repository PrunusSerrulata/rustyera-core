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
                // Several analyzer substages intentionally share the public Analyzing stage;
                // each substage starts a fresh real-work counter at zero.
                .all(|pair| pair[1].completed == 0 || pair[0].completed <= pair[1].completed),
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
