//! Static compatibility audit: complete inventory, symbol/reference evidence, and explicit execution gaps.

mod captures;
mod evidence;
mod graph;
mod output;
mod pipeline;
mod report;
mod scan;
mod symbols;
mod targets;

use std::{
    error::Error,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use era_runtime_protocol::FileCategory;
use erabasic_analyzer::AnalyzerOptions;
use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
use erabasic_csv::{CsvLoadOptions, FilePayload, FrontendFile, ProjectFiles};
use serde_json::{Value, json};

use super::{
    baseline,
    project_inputs::{DataRoot, ProjectInputs},
};

#[derive(Default)]
struct Arguments {
    project: Option<PathBuf>,
    output: Option<PathBuf>,
    markdown: Option<PathBuf>,
    profile: Option<CompatibilityProfileId>,
    analyzer_options: Option<PathBuf>,
    csv_options: Option<PathBuf>,
    captures: Option<PathBuf>,
    all_games: bool,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut result = Self::default();
        let mut arguments = arguments.peekable();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--all-games" => result.all_games = true,
                "--profile" => result.profile = Some(arguments.next().ok_or("--profile needs a value")?.parse()?),
                "--markdown" => result.markdown = Some(arguments.next().ok_or("--markdown needs a path")?.into()),
                "--analyzer-options" => result.analyzer_options = Some(arguments.next().ok_or("--analyzer-options needs a path")?.into()),
                "--csv-options" => result.csv_options = Some(arguments.next().ok_or("--csv-options needs a path")?.into()),
                "--captures" => result.captures = Some(arguments.next().ok_or("--captures needs a path")?.into()),
                flag if flag.starts_with('-') => return Err(format!("unknown coverage option: {flag}").into()),
                path if result.project.is_none() => result.project = Some(path.into()),
                path if result.output.is_none() => result.output = Some(path.into()),
                _ => return Err("usage: coverage PROJECT [OUTPUT.json[.gz]] [--all-games] [--profile PROFILE] [--markdown OUTPUT.md] [--analyzer-options OPTIONS.json] [--csv-options OPTIONS.json] [--captures CAPTURES.json]".into()),
            }
        }
        Ok(result)
    }
}

fn decode(bytes: &[u8]) -> Option<(String, &'static str)> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(bytes)
        .ok()
        .map(|text| (text.into(), "utf-8"))
}

fn config_identity(root: &Path, inventory: &baseline::Inventory) -> Result<Value, Box<dyn Error>> {
    use era_runtime_protocol::{FilePayload, ResolveProjectCompatibility, SubmittedFile};
    let configurations = inventory
        .files
        .iter()
        .filter(|file| file.path.eq_ignore_ascii_case("reraconfig.toml"))
        .collect::<Vec<_>>();
    if configurations.len() > 1 {
        return Ok(json!({"identity": null, "status": "duplicate_root_configuration"}));
    }
    let configuration = configurations
        .first()
        .map(|file| -> Result<_, Box<dyn Error>> {
            Ok(SubmittedFile {
                relative_path: file.path.clone(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(fs::read_to_string(root.join(&file.path))?),
                content_hash: None,
            })
        })
        .transpose()?;
    Ok(serde_json::to_value(
        era_runtime::resolve_project_compatibility(&ResolveProjectCompatibility {
            request_id: 0,
            configuration,
        }),
    )?)
}

fn audit_project(
    root: &Path,
    arguments: &Arguments,
    vm: &evidence::SourceIndex,
    runtime: &evidence::SourceIndex,
    captures: &captures::Captures,
    output: &mut dyn Write,
) -> Result<String, Box<dyn Error>> {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("project needs a UTF-8 name")?;
    let inventory = baseline::inventory(root, name)?;
    let resolution = config_identity(root, &inventory).unwrap_or_else(|error| json!({"identity": null, "status": "configuration_read_error", "error": error.to_string()}));
    let configured_identity: Option<CompatibilityIdentity> = resolution
        .get("identity")
        .cloned()
        .filter(|identity| !identity.is_null())
        .map(serde_json::from_value)
        .transpose()?;
    let identity = arguments
        .profile
        .map(CompatibilityIdentity::for_profile)
        .or(configured_identity.clone())
        .unwrap_or_else(CompatibilityIdentity::reference);
    let configuration_valid = configured_identity.is_some();
    let mut analyzer_options = arguments
        .analyzer_options
        .as_ref()
        .map(|path| -> Result<AnalyzerOptions, Box<dyn Error>> {
            Ok(serde_json::from_slice(&fs::read(path)?)?)
        })
        .transpose()?
        .unwrap_or_else(AnalyzerOptions::analysis_mode);
    // Coverage always includes uncalled bodies; the remaining effective audit options are serialized below.
    analyzer_options.analysis_mode = true;
    analyzer_options.ignore_uncalled_functions = false;
    analyzer_options.compatibility = identity.clone();
    let mut csv_options = arguments
        .csv_options
        .as_ref()
        .map(|path| -> Result<CsvLoadOptions, Box<dyn Error>> {
            Ok(serde_json::from_slice(&fs::read(path)?)?)
        })
        .transpose()?
        .unwrap_or_else(|| CsvLoadOptions {
            use_rename_file: true,
            search_subdirectories: true,
            sort_with_filename: true,
            ..Default::default()
        });
    csv_options.compatibility.clone_from(&identity);
    let paths = inventory
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let project_inputs = ProjectInputs::new(root, &paths);
    let has_erb = project_inputs.has_erb;
    let has_csv = project_inputs.has_csv;
    let mut sources = Vec::new();
    let mut files = ProjectFiles::default();
    let mut inputs = Vec::new();
    let mut errors = Vec::new();
    let mut excluded_inputs = Vec::new();
    for file in &inventory.files {
        let Some(category) = project_inputs.classify(&file.path) else {
            excluded_inputs.push(json!({"path": file.path, "raw_blake3": file.blake3, "reason": "not_a_selected_project_input_category"}));
            continue;
        };
        let script = matches!(category, FileCategory::Erb | FileCategory::Erh);
        let configuration = file.path.eq_ignore_ascii_case("reraconfig.toml");
        let data_root = project_inputs.data_root(&file.path, category);
        let resource = matches!(
            category,
            FileCategory::Resource | FileCategory::ResourceManifest
        );
        if !script && data_root.is_none() && !resource && !configuration {
            excluded_inputs.push(json!({"path": file.path, "category": category, "raw_blake3": file.blake3, "reason": "configuration_resolved_separately_or_outside_selected_data_root"}));
            continue;
        }
        crate::watchdog::publish(
            json!({"phase": "coverage_read_input", "case": name, "pending": file.path, "inputs_completed": inputs.len(), "input_errors": errors, "lastFullResponse": null}),
        )?;
        if resource {
            match output::hash_path(&root.join(&file.path), file.bytes) {
                Ok(hash) if hash.blake3 == file.blake3 && hash.bytes == file.bytes => inputs.push(json!({"path": file.path, "category": if category == FileCategory::Resource {"resource"} else {"resource_manifest"}, "data_root": null, "encoding": "raw_bytes_not_analyzed", "raw_byte_length": hash.bytes, "raw_blake3": hash.blake3})),
                Ok(_) => errors.push(json!({"path": file.path, "status": "input_changed_since_inventory"})),
                Err(error) => errors.push(json!({"path": file.path, "status": "read_failed", "error_kind": format!("{:?}", error.kind()), "message": error.to_string()})),
            }
            continue;
        }
        let bytes = match fs::read(root.join(&file.path)) {
            Ok(bytes) => bytes,
            Err(error) => {
                errors.push(json!({"path": file.path, "status": "read_failed", "error_kind": format!("{:?}", error.kind()), "message": error.to_string()}));
                continue;
            }
        };
        if blake3::hash(&bytes).to_hex().to_string() != file.blake3 {
            errors.push(json!({"path": file.path, "status": "input_changed_since_inventory"}));
            continue;
        }
        let Some((text, encoding)) = decode(&bytes) else {
            errors.push(json!({"path": file.path, "status": "unsupported_encoding"}));
            continue;
        };
        inputs.push(json!({"path": file.path, "category": category, "data_root": data_root.map(DataRoot::name), "encoding": encoding, "raw_byte_length": bytes.len(), "raw_blake3": file.blake3, "decoded_utf8_byte_length": text.len(), "decoded_utf8_blake3": blake3::hash(text.as_bytes()).to_hex().to_string()}));
        if script {
            sources.push((file.path.clone(), text));
        } else if let Some(data_root) = data_root {
            let item = FrontendFile {
                relative_path: data_root.relative_path(&file.path),
                source_path: Some(file.path.clone()),
                payload: FilePayload::Utf8(text),
            };
            match data_root {
                DataRoot::Csv => files.csv.push(item),
                DataRoot::Erb => files.erb.push(item),
            }
        }
    }
    sources.sort_by(|left, right| {
        (!left.0.to_ascii_lowercase().ends_with(".erh"), &left.0)
            .cmp(&(!right.0.to_ascii_lowercase().ends_with(".erh"), &right.0))
    });
    if sources.is_empty() {
        errors.push(json!({"status": "no_readable_script_inputs"}));
    }
    let scan = scan::scan(&sources, &analyzer_options);
    let unresolved_links = inventory
        .excluded
        .values()
        .any(|reason| reason == "unresolved_symlink_not_followed");
    let pipeline = pipeline::analyze(
        &sources,
        &files,
        analyzer_options,
        csv_options,
        errors.is_empty() && configuration_valid && !unresolved_links,
    );
    let execution_captures = captures.bind(name, &identity, &inputs);
    let metadata = json!({"project": name, "inventory": inventory, "configuration_resolution": resolution,
        "analysis_identity": identity, "profile_override": arguments.profile, "configuration_valid": configuration_valid,
        "input_policy": {"script_root": if has_erb { "ERB_case_insensitive" } else { "recursive_project_fallback" }, "csv_root": if has_csv { "CSV_case_insensitive" } else { "recursive_project_fallback" }, "spans": "decoded_utf8_bytes", "encoding_fallback": "none_strict_UTF8_only_optional_BOM", "audit_options": "explicit_audit_defaults_or_JSON_overrides_not_inferred_game_semantics", "legacy_configuration_semantics": "not_applied_by_coverage_use_explicit_options_for_comparison", "appearance_parser_context": "DefaultParserContext_with_profile_lexer_switches_debug_symbol_and_ERH_macros; not full analyzer symbol resolution; continuation separator is parser default", "uncalled_functions": "included", "unresolved_links": unresolved_links},
        "inputs": inputs, "excluded_inputs": excluded_inputs, "input_errors": errors, "execution_captures": execution_captures, "parser_diagnostics": scan.diagnostics, "pipeline": pipeline});
    report::write_project(output, metadata, &scan, &pipeline, vm, runtime)
}

pub(super) fn run_cli() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse(std::env::args().skip(2))?;
    let core = super::repository_root();
    let workspace = core.parent().ok_or("missing workspace parent")?;
    if arguments.project.is_none() && !arguments.all_games {
        return Err(
            "coverage requires PROJECT, or --all-games to use the fixed games directory".into(),
        );
    }
    let root = arguments
        .project
        .clone()
        .unwrap_or_else(|| workspace.join("games"));
    let vm = evidence::SourceIndex::collect(&core, "crates/erabasic-vm/src")?;
    let runtime = evidence::SourceIndex::collect(&core, "crates/era-runtime/src")?;
    let tool_identity = serde_json::to_value(baseline::git_identity(&core)?)?;
    let captures = captures::Captures::load(arguments.captures.as_deref(), tool_identity.clone())?;
    let metadata = json!({"version": 3, "kind": "snake_compatibility_static_coverage", "hash_algorithm": "blake3", "report_stream_hash_algorithms": ["blake3", "sha256"], "tool_revision": tool_identity, "frontend_revisions": {"tui": baseline::git_identity(&workspace.join("rustyera-tui"))?, "web": baseline::git_identity(&workspace.join("rustyera-web"))?}, "source_evidence": {"vm": vm, "runtime": runtime}, "status_vocabulary": ["unknown", "compiler_trap", "unsupported_capability", "blocked", "unverified"], "unsupported_capability_policy": "only an actual capability handshake/rejection can establish this state; absent capture stays unverified", "row_evidence_policy": "api_evidence_ref indexes project.api_evidence; diagnostic_ids index project.pipeline.diagnostics; owning_function indexes parser_functions; target_resolution_ref indexes target_resolutions then candidate_set_ref; all appearances retained, Markdown is an API summary"});
    let destination: Box<dyn Write> = match &arguments.output {
        Some(path) => Box::new(
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?,
        ),
        None => Box::new(std::io::stdout().lock()),
    };
    let gzip = arguments
        .output
        .as_ref()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"));
    let mut output = output::ReportOutput::new(BufWriter::new(destination), gzip);
    // Write metadata and rows incrementally: a failed run leaves incomplete JSON,
    // never a complete-looking report that silently omits the remaining projects.
    report::object_prefix(&mut output, &metadata)?;
    output.write_all(b",\"projects\":[")?;
    let roots = if arguments.all_games {
        baseline::GAMES
            .iter()
            .map(|name| root.join(name))
            .collect::<Vec<_>>()
    } else {
        vec![root]
    };
    let mut markdown = String::from(
        "# Snake compatibility coverage\n\nStatic evidence only, not a compatibility pass. Every appearance and diagnostic is retained in the JSON report. API-level source references and registration are shared evidence, not proof of execution. Runtime/frontend execution remains unverified.\n\n",
    );
    for (index, root) in roots.iter().enumerate() {
        if index != 0 {
            output.write_all(b",")?;
        }
        markdown.push_str(&audit_project(
            root,
            &arguments,
            &vm,
            &runtime,
            &captures,
            &mut output,
        )?);
    }
    output.write_all(b"]}\n")?;
    output.flush()?;
    let (_, digest_manifest) = output.finish()?;
    if let Some(path) = &arguments.markdown {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(markdown.as_bytes())?;
    }
    if let Some(path) = &arguments.output {
        let mut manifest_path = path.as_os_str().to_os_string();
        manifest_path.push(".manifest.json");
        let mut manifest = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(PathBuf::from(manifest_path))?;
        serde_json::to_writer(&mut manifest, &digest_manifest)?;
        manifest.write_all(b"\n")?;
        manifest.flush()?;
    } else {
        let mut stderr = std::io::stderr().lock();
        serde_json::to_writer(
            &mut stderr,
            &json!({"kind": "coverage_completion_manifest", "manifest": digest_manifest}),
        )?;
        stderr.write_all(b"\n")?;
        stderr.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_options_reject_unknown_profile_and_preserve_profile_override() {
        assert!(Arguments::parse(["--profile", "snake"].into_iter().map(str::to_owned)).is_err());
        let parsed = Arguments::parse(
            [
                "game",
                "report.json",
                "--profile",
                "emuera.skia.snake",
                "--all-games",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();
        assert!(parsed.all_games);
        assert_eq!(parsed.profile.unwrap().to_string(), "emuera.skia.snake");
    }

    #[test]
    fn decoded_offsets_and_raw_hashes_are_distinct_inputs() {
        let (text, encoding) = decode(b"\xef\xbb\xbfPRINTL text\r\n").unwrap();
        assert_eq!(text, "PRINTL text\r\n");
        assert_eq!(encoding, "utf-8");
        assert!(
            decode(&[0x82, 0xa0]).is_none(),
            "legacy encodings are not silently converted"
        );
    }

    #[test]
    fn capture_binding_checks_source_and_artifact_hashes_without_claiming_execution_success() {
        use sha2::{Digest as _, Sha256};
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "rustyera-coverage-capture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fs::create_dir(&fixture.0).unwrap();
        let capture = b"{\"kind\":\"fixture_runtime_observation\"}\n";
        fs::write(fixture.0.join("capture.ndjson"), capture).unwrap();
        let identity = CompatibilityIdentity::reference();
        let core = json!({"sha": "fixture-only"});
        let raw_hash = blake3::hash(b"\xef\xbb\xbfPRINTL test")
            .to_hex()
            .to_string();
        let payload_hash = blake3::hash(b"PRINTL test").to_hex().to_string();
        let manifest = json!({"version": 1, "entries": [{"project": "fixture", "api": "PRINTL", "frontend": "core",
            "compatibility": identity, "core_identity": core, "source_hashes": {"a.erb": {"raw_blake3": raw_hash, "decoded_utf8_blake3": payload_hash}},
            "capture": "capture.ndjson", "capture_sha256": format!("{:x}", Sha256::digest(capture)), "description": "unit fixture provenance, not product evidence"}]});
        let path = fixture.0.join("manifest.json");
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let loaded = captures::Captures::load(Some(&path), core.clone()).unwrap();
        let inputs = vec![
            json!({"path": "a.erb", "raw_blake3": raw_hash, "decoded_utf8_blake3": payload_hash}),
        ];
        let bound = loaded.bind("fixture", &identity, &inputs);
        assert_eq!(captures::api_refs(&bound, "PRINTL"), [json!(0)]);
        assert_eq!(
            bound["entries"][0]["execution_status"],
            "unverified_capture_requires_behavior_review"
        );
        let wrong = vec![
            json!({"path": "a.erb", "raw_blake3": payload_hash, "decoded_utf8_blake3": payload_hash}),
        ];
        assert!(
            captures::api_refs(&loaded.bind("fixture", &identity, &wrong), "PRINTL").is_empty()
        );
        let snake = CompatibilityIdentity::for_profile(CompatibilityProfileId::EmueraSkiaSnake);
        assert!(captures::api_refs(&loaded.bind("fixture", &snake, &inputs), "PRINTL").is_empty());
        let wrong_core =
            captures::Captures::load(Some(&path), json!({"sha": "different-fixture"})).unwrap();
        assert!(
            captures::api_refs(&wrong_core.bind("fixture", &identity, &inputs), "PRINTL")
                .is_empty()
        );
        fs::write(fixture.0.join("capture.ndjson"), b"changed").unwrap();
        let changed = captures::Captures::load(Some(&path), core).unwrap();
        assert!(
            captures::api_refs(&changed.bind("fixture", &identity, &inputs), "PRINTL").is_empty()
        );
        assert_eq!(
            captures::Captures::default().bind("fixture", &identity, &inputs)["status"],
            "unverified_no_capture"
        );
    }
}
