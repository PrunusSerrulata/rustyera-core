use era_protocol::{ProtocolBytes, ProtocolVersion};
use era_runtime_protocol::{
    ExtensionArgument, ExtensionArgumentStyle, ExtensionCallableKind, ExtensionDeclaration,
    ExtensionValueType, ExternalResource, FileCategory, FileChange, FilePayload,
    ImageMetadataResponse, ProjectAnalysisRequest, ReloadProject, RuntimeLogLevel, SubmittedFile,
};
use std::fmt::Write as _;

use super::*;
use era_runtime_protocol::ConfigurationApplication;
use erabasic_data::LegacyEncoding;

#[test]
fn static_user_argument_diagnostics_use_compat_code_and_profile_context() {
    for strict in [false, true] {
        let mut manifest = index_manifest("ERB/");
        let script = manifest
            .files
            .iter_mut()
            .find(|file| file.category == FileCategory::Erb)
            .unwrap();
        script.payload = FilePayload::Utf8(
            "@SYSTEM_TITLE\nCALL TAKE, 1, 2\nRETURN\n@TAKE(ARG)\nRETURN\n".into(),
        );
        let configuration = manifest
            .files
            .iter_mut()
            .find(|file| file.category == FileCategory::Configuration)
            .unwrap();
        let FilePayload::Utf8(text) = &mut configuration.payload else {
            unreachable!()
        };
        write!(
            text,
            "[diagnostics]\nstrict_user_call_arguments = {strict}\n"
        )
        .unwrap();
        let build = build_project(&manifest, None);
        assert_eq!(
            build.report.success, !strict,
            "{:?}",
            build.report.diagnostics
        );
        let matching = build
            .report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "compat.call.excess_arguments")
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        let diagnostic = matching[0];
        assert_eq!(
            diagnostic.level,
            if strict {
                RuntimeLogLevel::Error
            } else {
                RuntimeLogLevel::Warning
            }
        );
        let context = diagnostic.context.as_ref().unwrap();
        assert_eq!(context.stage, "compat");
        assert_eq!(context.identity.as_ref(), Some(&manifest.compatibility));
        let source = diagnostic.source.as_ref().unwrap();
        assert_eq!(source.relative_path, "ERB/main.erb");
        assert!(source.byte_end > source.byte_start);
        if !strict {
            let warm = build_project(&manifest, Some(&build.incremental));
            let matching = warm
                .report
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "compat.call.excess_arguments")
                .collect::<Vec<_>>();
            assert_eq!(matching, vec![diagnostic]);
        }
    }
}

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

