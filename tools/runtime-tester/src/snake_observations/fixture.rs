use super::{AuditResult, COMPLETE};
use era_config::{LegacyConfigSource, migrate_legacy_configuration};
use era_runtime_protocol::{
    FileCategory, FilePayload, ProjectManifest, ResolveProjectCompatibility, SubmittedFile,
};
use erabasic_compat::CompatibilityIdentity;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
pub(super) struct Fixture {
    pub(super) version: u32,
    pub(super) seed: u64,
    pub(super) cases: Vec<Case>,
}

#[derive(Deserialize)]
pub(super) struct Case {
    pub(super) id: String,
    pub(super) group: String,
    pub(super) requests: Vec<CaseRequest>,
}

#[derive(Deserialize)]
pub(super) struct CaseRequest {
    pub(super) request: Value,
}

pub(super) fn git_output(arguments: &[&str]) -> AuditResult<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(crate::repository_root())
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "git metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn fixture_identity(root: &Path) -> AuditResult<Value> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<String, Value>,
    ) -> AuditResult<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = fs::read(&path)?;
                files.insert(
                    relative.clone(),
                    json!({
                        "path": relative, "bytes": bytes.len(), "sha256": hash(&bytes)
                    }),
                );
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    let files: Vec<_> = entries.into_values().collect();
    // serde_json's default sorted maps produce Python sort_keys=True compact encoding here.
    let digest = hash(&serde_json::to_vec(&files)?);
    Ok(json!({"files": files, "sha256": digest}))
}

fn submitted(path: &str, category: FileCategory, source: String) -> SubmittedFile {
    SubmittedFile {
        relative_path: path.into(),
        category,
        payload: FilePayload::Utf8(source),
        content_hash: None,
    }
}

pub(super) fn wrapper(request: &Value) -> AuditResult<String> {
    let action = match request["op"].as_str() {
        Some("eval") => format!(
            "RESULT = {}",
            request["source"].as_str().ok_or("eval source missing")?
        ),
        Some("run") => {
            let entry = request["entry"]
                .as_str()
                .ok_or("initial run entry missing")?;
            let arguments = request["arguments"].as_str().unwrap_or("");
            if arguments.is_empty() {
                format!("CALL {entry}")
            } else {
                format!("CALL {entry}, {arguments}")
            }
        }
        _ => return Err("unsupported request operation".into()),
    };
    Ok(format!(
        "@SYSTEM_TITLE\n{action}\nPRINTL {COMPLETE}\nINPUT\nRETURN\n"
    ))
}

fn load_fixture_files(root: &Path, group: &str) -> AuditResult<Vec<SubmittedFile>> {
    let group_file = match group {
        "PRINTC" => "printc",
        "arithmetic" => "arithmetic",
        "RNG" => "rng",
        "REF" => "ref",
        "extra_args" => "extra-args",
        "TOINT" => "toint",
        "GETKEY" => "getkey",
        "INDEX" => "index",
        "METHODS" => "methods",
        "COLUMNS" => "columns",
        "FAULT_HOOKS" => "fault_hooks",
        "DISPLAY_STATE" => "display_state",
        "INPUT" => "input",
        _ => return Err(format!("unknown fixture group {group}").into()),
    };
    let mut files = Vec::new();
    for (path, category) in [
        (format!("erb/{group_file}.erb"), FileCategory::Erb),
        ("csv/GAMEBASE.CSV".into(), FileCategory::Csv),
        ("emuera.config".into(), FileCategory::Configuration),
        ("setting.json".into(), FileCategory::Configuration),
    ] {
        files.push(submitted(
            &path,
            category,
            fs::read_to_string(root.join(&path))?,
        ));
    }
    if matches!(group, "METHODS" | "COLUMNS") {
        let header = format!("erb/{group_file}.erh");
        files.push(submitted(
            &header,
            FileCategory::Erh,
            fs::read_to_string(root.join(&header))?,
        ));
    } else if group == "INDEX" {
        // Index cases keep their own complete data inputs. Do not add these files
        // to older observation groups: their historical fixture identity is frozen.
        for (path, category) in [
            ("erb/index.erh", FileCategory::Erh),
            ("csv/BUFF.csv", FileCategory::Csv),
            ("csv/BUFF.als", FileCategory::Als),
            ("csv/FLAG.csv", FileCategory::Csv),
            ("csv/FLAG.als", FileCategory::Als),
            ("erb/columns/COLUMNDIV@2.ERD", FileCategory::Erd),
            ("erb/columns/COLUMNDIV@2.als", FileCategory::Als),
            ("erb/matrix/SEMEN_MATRIX@2.ERD", FileCategory::Erd),
            ("erb/matrix/SEMEN_MATRIX@2.als", FileCategory::Als),
        ] {
            files.push(submitted(
                path,
                category,
                fs::read_to_string(root.join(path))?,
            ));
        }
    }
    if group == "COLUMNS" {
        files.push(submitted(
            "csv/VarExt.csv",
            FileCategory::Csv,
            fs::read_to_string(root.join("csv/VarExt.csv"))?,
        ));
        let csv_root = root.join("csv");
        let mut character_files = fs::read_dir(&csv_root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()?;
        character_files.sort();
        for path in character_files {
            let metadata = fs::symlink_metadata(&path)?;
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err("observation CSV path is not UTF-8".into());
            };
            let upper = name.to_ascii_uppercase();
            if !upper.starts_with("CHARA") || !upper.ends_with(".CSV") {
                continue;
            }
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("observation character CSV must be a regular file".into());
            }
            let relative = format!("csv/{name}");
            files.push(submitted(
                &relative,
                FileCategory::Csv,
                fs::read_to_string(path)?,
            ));
        }
        load_fixture_resources(root, &root.join("plugins"), &mut files)?;
        let patterns = root.join("patterns");
        match fs::symlink_metadata(&patterns) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                load_fixture_resources(root, &patterns, &mut files)?;
            }
            Ok(_) => return Err("observation patterns must be a regular directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(files)
}

fn load_fixture_resources(
    root: &Path,
    directory: &Path,
    files: &mut Vec<SubmittedFile>,
) -> AuditResult<()> {
    let mut pending = vec![directory.to_owned()];
    let mut entry_count = 0_usize;
    let mut total_bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory)? {
            entry_count += 1;
            if entry_count > 100_000 {
                return Err("observation resource inventory exceeds its limit".into());
            }
            paths.push(entry?.path());
        }
        paths.sort();
        for path in paths {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err("observation resources must not be symbolic links".into());
            }
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("resource path is not UTF-8")?
                .replace('\\', "/");
            if relative.len() > 4096 || relative.split('/').count() > 64 {
                return Err("observation resource path exceeds its limit".into());
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                total_bytes = total_bytes
                    .checked_add(metadata.len())
                    .ok_or("observation resource size overflow")?;
                if total_bytes > 64 * 1024 * 1024 {
                    return Err("observation resources are oversized".into());
                }
                let mut contents = Vec::new();
                fs::File::open(&path)?
                    .take(metadata.len() + 1)
                    .read_to_end(&mut contents)?;
                if contents.len() as u64 != metadata.len() {
                    return Err("observation resource changed during reading".into());
                }
                files.push(SubmittedFile {
                    relative_path: relative,
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(era_protocol::ProtocolBytes::new(contents)),
                    content_hash: None,
                });
            } else {
                return Err("observation resource is not a regular file".into());
            }
        }
    }
    Ok(())
}

pub(super) fn build_manifest(
    root: &Path,
    identity: &CompatibilityIdentity,
    group: &str,
    request: &Value,
) -> AuditResult<(ProjectManifest, Value)> {
    let mut files = load_fixture_files(root, group)?;
    let sources: Vec<_> = files
        .iter()
        .filter_map(|file| match &file.payload {
            FilePayload::Utf8(contents) if file.category == FileCategory::Configuration => {
                Some(LegacyConfigSource {
                    relative_path: &file.relative_path,
                    contents,
                })
            }
            _ => None,
        })
        .collect();
    let migration = migrate_legacy_configuration(&sources);
    let migration_diagnostics: Vec<_> = migration
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut configuration = migration.document.to_lf_string();
    let profile_line = format!("profile = \"{}\"\n", identity.profile.as_str());
    if configuration.contains("[compatibility]\n") {
        configuration = configuration.replacen(
            "[compatibility]\n",
            &format!("[compatibility]\n{profile_line}"),
            1,
        );
    } else {
        configuration.push_str(&format!("\n[compatibility]\n{profile_line}"));
    }
    let configuration = submitted(
        "reraconfig.toml",
        FileCategory::Configuration,
        configuration,
    );
    let resolved = era_runtime::resolve_project_compatibility(&ResolveProjectCompatibility {
        request_id: 1,
        configuration: Some(configuration.clone()),
    });
    if resolved.identity.as_ref() != Some(identity) {
        return Err(format!(
            "generated configuration rejected: {:?}",
            resolved.diagnostics
        )
        .into());
    }
    files.push(configuration);
    files.push(submitted(
        "erb/__observation_title.erb",
        FileCategory::Erb,
        wrapper(request)?,
    ));
    let effective_sources: Vec<_> = files
        .iter()
        .map(|file| {
            let source = match &file.payload {
                FilePayload::Utf8(source) => source.as_bytes(),
                FilePayload::Bytes(source) => source.as_slice(),
                _ => unreachable!("observation inputs are eagerly loaded"),
            };
            json!({"path": file.relative_path, "bytes": source.len(), "sha256": hash(source)})
        })
        .collect();
    let mut harness = json!({
        "effectiveSources": effective_sources, "migrationDiagnostics": migration_diagnostics,
        "wrapper": wrapper(request)?, "baseTitleReplaced": "erb/base.erb",
    });
    if group == "COLUMNS" {
        harness["storageSimulation"] = json!({
            "backend": "case-local owned memory; no filesystem I/O",
            "originalDataList": "existing Data directory, otherwise manifest resources; never union",
            "originalPatterns": "bounded snake matcher for fixture patterns; not full original frontend matching",
            "limitations": "NFC/lowercase keys and synthetic range tokens; no OS permissions or symlinks",
        });
    }
    Ok((
        ProjectManifest {
            project_revision: 1,
            files,
            compatibility: identity.clone(),
        },
        harness,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_fixture_keeps_resource_bytes_and_loads_persistent_declarations() {
        let root = crate::tool_root().join("fixture-snake-data");
        let identity = CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
        let request = json!({"op":"run","entry":"C1_CASE_RESOURCE_READ"});
        let (manifest, harness) = build_manifest(&root, &identity, "COLUMNS", &request).unwrap();
        for path in [
            "plugins/data.txt",
            "plugins/nested/child.txt",
            "plugins/map.xml",
            "plugins/dataset-schema.xml",
            "plugins/dataset.xml",
            "patterns/SEED.TXT",
            "patterns/[ab].txt",
            "patterns/a.txt",
            "patterns/é.txt",
            "patterns/😀.txt",
        ] {
            let resource = manifest
                .files
                .iter()
                .find(|file| file.relative_path == path)
                .unwrap();
            assert_eq!(resource.category, FileCategory::Resource);
            let FilePayload::Bytes(bytes) = &resource.payload else {
                panic!("resource must retain raw bytes")
            };
            assert_eq!(bytes.as_slice(), fs::read(root.join(path)).unwrap());
            let evidence = harness["effectiveSources"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["path"] == path)
                .unwrap();
            assert_eq!(evidence["sha256"], hash(bytes.as_slice()));
        }
        assert!(manifest.files.iter().any(
            |file| file.relative_path == "csv/VarExt.csv" && file.category == FileCategory::Csv
        ));
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.relative_path == "erb/columns.erh"
                    && file.category == FileCategory::Erh)
        );
    }

    #[test]
    fn batch_2c_fixture_adds_character_csv_files_in_stable_path_order() {
        let root = crate::tool_root().join("fixture-snake-batch2-data");
        let paths = load_fixture_files(&root, "COLUMNS")
            .unwrap()
            .into_iter()
            .filter(|file| file.relative_path.starts_with("csv/CHARA"))
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "csv/CHARA1.CSV",
                "csv/CHARA20.CSV",
                "csv/CHARA30.CSV",
                "csv/CHARA90.CSV"
            ]
        );
    }

    #[test]
    fn batch_2d_fixture_routes_fault_hook_sources_and_configuration() {
        for variant in ["enabled", "disabled"] {
            let root = crate::tool_root()
                .join("fixture-snake-batch2-fault-hooks")
                .join(variant);
            let files = load_fixture_files(&root, "FAULT_HOOKS").unwrap();
            assert!(files.iter().any(|file| {
                file.relative_path == "erb/fault_hooks.erb" && file.category == FileCategory::Erb
            }));
            assert!(files.iter().any(|file| {
                file.relative_path == "emuera.config"
                    && file.category == FileCategory::Configuration
            }));
        }
    }

    #[test]
    fn data_fixture_executes_defaults_global_and_resource_overlay() {
        use erabasic_compat::CompatibilityProfileId;
        let root = crate::tool_root().join("fixture-snake-data");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        for (id, expected) in [
            (
                "column-basic-defaults",
                json!({"RESULT:10":777,"RESULT:11":11,"RESULTS:10":"默认&A<quote>"}),
            ),
            (
                "column-empty-string-and-explicit-null",
                json!({"RESULT:10":0,"RESULT:11":1,"RESULT:12":0,"RESULT:13":0}),
            ),
            (
                "column-update-no-backfill",
                json!({"RESULT:10":11,"RESULT:11":22}),
            ),
            (
                "column-global-roundtrip",
                json!({"RESULT:10":1,"RESULT:11":7,"RESULT:12":55,"RESULT:13":12,"RESULT:14":1,"RESULTS:10":"saved-map","RESULTS:11":"saved-xml"}),
            ),
            (
                "column-resource-map-xml-datatable",
                json!({"RESULT:10":1,"RESULT:11":29,"RESULT:12":17,"RESULT:13":1,"RESULTS:10":"station","RESULTS:11":"from-data","RESULTS:12":"from-schema","RESULTS:13":"29"}),
            ),
            (
                "column-resource-overlay-enumeration",
                json!({"RESULT:10":1,"RESULT:11":1,"RESULT:12":2,"RESULT:13":1,"RESULTS:10":"resource-text\n","RESULTS:11":"overlay-text"}),
            ),
        ] {
            let case = fixture.cases.iter().find(|case| case.id == id).unwrap();
            for profile in [
                CompatibilityProfileId::EmueraEm,
                CompatibilityProfileId::EmueraSkiaSnake,
            ] {
                let mut expected = expected.clone();
                if id == "column-resource-overlay-enumeration"
                    && profile == CompatibilityProfileId::EmueraEm
                {
                    // The reference expectation remains 2. Original Rust hosts select
                    // the existing Data directory; only snake runtime merges resources.
                    expected["RESULT:12"] = json!(1);
                }
                let result = super::super::observe_case(
                    &root,
                    &CompatibilityIdentity::for_profile(profile),
                    fixture.seed,
                    case,
                )
                .unwrap();
                assert_eq!(result["load"]["success"], true, "{id}: {result}");
                assert_eq!(result["steps"][0]["status"], "executed", "{id}: {result}");
                assert_eq!(result["steps"][0]["result"]["ok"], true, "{id}: {result}");
                assert_eq!(
                    result["steps"][0]["result"]["watches"], expected,
                    "{id}: {result}"
                );
            }
        }
    }

    #[test]
    fn arithmetic_fixture_preserves_observed_result_after_return() {
        let root = crate::tool_root().join("fixture-snake-compatibility");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        let case = fixture
            .cases
            .iter()
            .find(|case| case.id == "arithmetic-variable")
            .unwrap();
        let result = super::super::observe_case(
            &root,
            &CompatibilityIdentity::reference(),
            fixture.seed,
            case,
        )
        .unwrap();
        let watches = &result["steps"][0]["result"]["watches"];
        assert_eq!(watches["RESULT:0"], json!(i64::MIN));
        assert_eq!(watches["RESULT:1"], json!(i64::MAX - 1));
        assert_eq!(watches["RESULT:2"], json!(i64::MAX));
    }
    #[test]
    fn wrapper_preserves_argument_expressions_and_omissions() {
        for arguments in ["7,", "7, COMPAT_SIDE_EFFECT()"] {
            let source =
                wrapper(&json!({"op": "run", "entry": "COMPAT_ONE", "arguments": arguments}))
                    .unwrap();
            assert!(source.contains(&format!("CALL COMPAT_ONE, {arguments}\n")));
        }
    }

    #[test]
    fn index_fixture_submits_primary_alias_and_header_inputs_without_reclassification() {
        let root = crate::tool_root().join("fixture-snake-index-inputs");
        let identity = CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        );
        let request = json!({"op": "run", "entry": "INDEX_READ", "arguments": "\"alias\""});
        let (manifest, _) = build_manifest(&root, &identity, "INDEX", &request).unwrap();
        for (path, category) in [
            ("erb/index.erh", FileCategory::Erh),
            ("csv/BUFF.als", FileCategory::Als),
            ("erb/columns/COLUMNDIV@2.ERD", FileCategory::Erd),
            ("erb/matrix/SEMEN_MATRIX@2.als", FileCategory::Als),
        ] {
            let submitted = manifest
                .files
                .iter()
                .find(|file| file.relative_path == path)
                .unwrap();
            assert_eq!(submitted.category, category);
            assert_eq!(
                submitted.payload,
                FilePayload::Utf8(fs::read_to_string(root.join(path)).unwrap())
            );
        }
        assert!(
            !manifest
                .files
                .iter()
                .any(|file| file.relative_path == "erb/base.erb")
        );
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.relative_path == "erb/__observation_title.erb")
        );
    }

    #[test]
    fn index_fixture_executes_aliases_only_for_the_snake_profile() {
        use erabasic_compat::CompatibilityProfileId;

        let root = crate::tool_root().join("fixture-snake-index-inputs");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        let case = fixture
            .cases
            .iter()
            .find(|case| case.id == "index-user-alias-10-trim-first-wins")
            .unwrap();
        for profile in [
            CompatibilityProfileId::EmueraEm,
            CompatibilityProfileId::EmueraSkiaSnake,
        ] {
            let result = super::super::observe_case(
                &root,
                &CompatibilityIdentity::for_profile(profile),
                fixture.seed,
                case,
            )
            .unwrap();
            assert_eq!(result["load"]["success"], true, "{result}");
            assert_eq!(result["steps"][0]["status"], "executed", "{result}");
            let observation = &result["steps"][0]["result"];
            if profile == CompatibilityProfileId::EmueraSkiaSnake {
                assert_eq!(observation["ok"], true, "{result}");
                assert_eq!(observation["watches"]["RESULT:0"], 110);
            } else {
                assert_eq!(observation["ok"], false, "{result}");
                assert_eq!(observation["termination"], "faulted", "{result}");
            }
        }
    }

    #[test]
    fn method_fixture_executes_lazy_values_references_and_statement_results() {
        use erabasic_compat::CompatibilityProfileId;

        let root = crate::tool_root().join("fixture-snake-methods");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        for (id, expected) in [
            (
                "method-present-skips-fallback",
                json!({"RESULT:0":23,"METHOD_TRACE:0":1234,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-missing-only-fallback",
                json!({"RESULT:0":90,"METHOD_TRACE:0":19,"METHOD_BODY_COUNT:0":0}),
            ),
            (
                "method-explicit-omitted-slot",
                json!({"RESULT:0":57,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-i64-min-is-value",
                json!({"RESULT:0":i64::MIN,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-whole-array-ref-skips-index",
                json!({"RESULT:0":11,"METHOD_VALUES:0":11,"METHOD_VALUES:2":30,"METHOD_WORDS:1":"changed","METHOD_INDEX_COUNT:0":0,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-value-captured-before-next-argument",
                json!({"RESULT:0":102,"METHOD_VALUES:0":99,"METHOD_TRACE:0":4,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-integer-statement",
                json!({"RESULT:0":42,"METHOD_TRACE:0":0,"METHOD_BODY_COUNT:0":1}),
            ),
            (
                "method-string-statement",
                json!({"RESULTS:0":"zero","METHOD_TRACE:0":0,"METHOD_BODY_COUNT:0":1}),
            ),
        ] {
            let case = fixture.cases.iter().find(|case| case.id == id).unwrap();
            for profile in [
                CompatibilityProfileId::EmueraEm,
                CompatibilityProfileId::EmueraSkiaSnake,
            ] {
                let result = super::super::observe_case(
                    &root,
                    &CompatibilityIdentity::for_profile(profile),
                    fixture.seed,
                    case,
                )
                .unwrap();
                assert_eq!(result["load"]["success"], true, "{id}: {result}");
                assert_eq!(result["steps"][0]["status"], "executed", "{id}: {result}");
                let observation = &result["steps"][0]["result"];
                assert_eq!(observation["ok"], true, "{id}: {result}");
                assert_eq!(observation["watches"], expected, "{id}: {result}");
            }
        }
    }

    #[test]
    fn original_builtin_alias_warning_keeps_later_aliases_available() {
        let root = crate::tool_root().join("fixture-snake-index-inputs");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        // The pinned original oracle aborts this ALS file while formatting a warning.
        // Preserve Rust's existing continue-after-warning behavior, not that defect.
        for (case_id, expected) in [
            ("index-builtin-alias-same-index", 500),
            ("index-builtin-untrimmed-name", 210),
        ] {
            let case = fixture
                .cases
                .iter()
                .find(|case| case.id == case_id)
                .unwrap();
            let result = super::super::observe_case(
                &root,
                &CompatibilityIdentity::reference(),
                fixture.seed,
                case,
            )
            .unwrap();
            assert_eq!(result["load"]["success"], true, "{result}");
            let observation = &result["steps"][0]["result"];
            assert_eq!(observation["ok"], true, "{result}");
            assert_eq!(observation["watches"]["RESULT:0"], expected, "{result}");
        }
    }

    #[test]
    fn index_fixture_distinguishes_shared_static_names_from_original_dynamic_gap() {
        use erabasic_compat::CompatibilityProfileId;

        let root = crate::tool_root().join("fixture-snake-index-inputs");
        let fixture: Fixture =
            serde_json::from_slice(&fs::read(root.join("cases.json")).unwrap()).unwrap();
        for case_id in [
            "index-static-primary-names",
            "index-primary-name-precedes-alias",
            "index-column-primary",
            "index-matrix-primary-300",
        ] {
            let case = fixture
                .cases
                .iter()
                .find(|case| case.id == case_id)
                .unwrap();
            let is_static = case_id == "index-static-primary-names";
            for profile in [
                CompatibilityProfileId::EmueraEm,
                CompatibilityProfileId::EmueraSkiaSnake,
            ] {
                let result = super::super::observe_case(
                    &root,
                    &CompatibilityIdentity::for_profile(profile),
                    fixture.seed,
                    case,
                )
                .unwrap();
                assert_eq!(result["load"]["success"], true, "{result}");
                assert_eq!(result["steps"][0]["status"], "executed", "{result}");
                let observation = &result["steps"][0]["result"];
                if !is_static && profile == CompatibilityProfileId::EmueraEm {
                    // Preserve the original profile's existing dynamic user-index gap.
                    // The fixed original oracle succeeds; this is not its rejection.
                    assert_eq!(observation["ok"], false, "{result}");
                    assert_eq!(observation["termination"], "faulted", "{result}");
                    continue;
                }
                assert_eq!(observation["ok"], true, "{result}");
                let expected = match case_id {
                    "index-column-primary" => 311,
                    "index-matrix-primary-300" => 600,
                    _ => 110,
                };
                assert_eq!(observation["watches"]["RESULT:0"], expected, "{result}");
                if is_static {
                    assert_eq!(observation["watches"]["RESULT:1"], 311, "{result}");
                    assert_eq!(observation["watches"]["RESULT:2"], 600, "{result}");
                }
            }
        }
    }
}
