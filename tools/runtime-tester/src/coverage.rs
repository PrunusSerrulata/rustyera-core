//! Batch-zero audit: input inventory, parser appearances, real compilation, and explicit evidence gaps.

mod evidence;
mod pipeline;
mod scan;

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use erabasic_analyzer::{AnalyzerOptions, builtin_function_names, builtin_instruction_names};
use erabasic_compat::{CompatibilityIdentity, CompatibilityProfileId};
use erabasic_csv::{CsvLoadOptions, FilePayload, FrontendFile, ProjectFiles};
use serde_json::{Value, json};

use super::baseline;

#[derive(Default)]
struct Arguments {
    project: Option<PathBuf>,
    output: Option<PathBuf>,
    markdown: Option<PathBuf>,
    profile: Option<CompatibilityProfileId>,
    analyzer_options: Option<PathBuf>,
    csv_options: Option<PathBuf>,
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
                flag if flag.starts_with('-') => return Err(format!("unknown coverage option: {flag}").into()),
                path if result.project.is_none() => result.project = Some(path.into()),
                path if result.output.is_none() => result.output = Some(path.into()),
                _ => return Err("usage: coverage PROJECT [OUTPUT.json] [--all-games] [--profile PROFILE] [--markdown OUTPUT.md] [--analyzer-options OPTIONS.json] [--csv-options OPTIONS.json]".into()),
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

fn is_input(path: &str, root: &str, has_root: bool, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or_default();
    extensions.contains(&extension) && (!has_root || lower.starts_with(&format!("{root}/")))
}

fn config_identity(root: &Path, inventory: &baseline::Inventory) -> Result<Value, Box<dyn Error>> {
    use era_runtime_protocol::{
        FileCategory, FilePayload, ResolveProjectCompatibility, SubmittedFile,
    };
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
) -> Result<Value, Box<dyn Error>> {
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
    let csv_options = arguments
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
    let has_erb = inventory
        .files
        .iter()
        .any(|file| file.path.to_ascii_lowercase().starts_with("erb/"));
    let has_csv = inventory
        .files
        .iter()
        .any(|file| file.path.to_ascii_lowercase().starts_with("csv/"));
    let mut sources = Vec::new();
    let mut files = ProjectFiles::default();
    let mut inputs = Vec::new();
    let mut errors = Vec::new();
    for file in &inventory.files {
        let script = is_input(&file.path, "erb", has_erb, &["erb", "erh"]);
        let csv_input = is_input(&file.path, "csv", has_csv, &["csv", "als", "erd"]);
        let erb_data = is_input(&file.path, "erb", has_erb, &["erd"]);
        if !script && !csv_input && !erb_data {
            continue;
        }
        crate::watchdog::publish(
            json!({"phase": "coverage_read_input", "case": name, "pending": file.path, "inputs_completed": inputs.len(), "input_errors": errors, "lastFullResponse": null}),
        )?;
        let bytes = fs::read(root.join(&file.path))?;
        if blake3::hash(&bytes).to_hex().to_string() != file.blake3 {
            errors.push(json!({"path": file.path, "status": "input_changed_since_inventory"}));
            continue;
        }
        let Some((text, encoding)) = decode(&bytes) else {
            errors.push(json!({"path": file.path, "status": "unsupported_encoding"}));
            continue;
        };
        inputs.push(json!({"path": file.path, "encoding": encoding, "raw_blake3": file.blake3, "decoded_utf8_blake3": blake3::hash(text.as_bytes()).to_hex().to_string()}));
        if script {
            sources.push((file.path.clone(), text));
        } else {
            let relative_path = if (csv_input && has_csv) || (erb_data && has_erb) {
                file.path
                    .split_once('/')
                    .map_or(file.path.as_str(), |(_, rest)| rest)
                    .into()
            } else {
                file.path.clone()
            };
            let item = FrontendFile {
                relative_path,
                payload: FilePayload::Utf8(text),
            };
            if csv_input {
                files.csv.push(item);
            } else {
                files.erb.push(item);
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
    let registry = erabasic_compiler::default_host_registry();
    let functions = builtin_function_names();
    let instructions = builtin_instruction_names();
    let mut counts = BTreeMap::<String, BTreeMap<String, usize>>::new();
    let rows = scan.rows.iter().enumerate().map(|(index, appearance)| {
        if index % 128 == 0 {
            crate::watchdog::publish_or_exit(json!({"phase": "coverage_rows", "case": name, "pending": appearance, "rows_completed": index, "rows_total": scan.rows.len(), "diagnostics": pipeline.diagnostics, "lastFullResponse": null}));
        }
        *counts.entry(appearance.api.clone()).or_default().entry(appearance.activity.clone()).or_default() += 1;
        let known = match appearance.form.as_str() {
            "expression" => functions.contains(&appearance.api) || scan.user_functions.contains(&appearance.api),
            "instruction" => instructions.contains(&appearance.api),
            "declaration" | "operator" | "compound_assignment" => true,
            _ => false,
        };
        let analyzer_diagnostics = pipeline::overlapping(&pipeline, &appearance.path, appearance.span, "analyzer");
        let compiler_diagnostics = pipeline::overlapping(&pipeline, &appearance.path, appearance.span, "compiler");
        let analyzer_status = if !known { "unknown" } else if appearance.activity != "active_ast" { "unverified" } else if analyzer_diagnostics.iter().any(|item| item.error) { "rejected" } else if pipeline.analyzer == "accepted" { "accepted_under_reported_audit_options" } else { "unverified_due_to_project_errors" };
        let compiler_status = if compiler_diagnostics.iter().any(|item| item.code == "UnsupportedConstruct") { "compiler_trap" } else if compiler_diagnostics.iter().any(|item| item.error) { "rejected" } else if appearance.activity != "active_ast" { "unverified" } else if pipeline.compiler == "accepted" { "project_compiled_occurrence_not_executed" } else { "blocked" };
        json!({
            "appearance": appearance,
            "overload": {"argument_count": appearance.arity, "omitted_arguments": appearance.omitted_arguments, "resolution": "not_inferred_from_arity_consult_analyzer_diagnostics"},
            "analyzer": {"status": analyzer_status, "catalog_known": known, "diagnostics": analyzer_diagnostics},
            "compiler": {"status": compiler_status, "project_status": pipeline.compiler, "diagnostics": compiler_diagnostics},
            "registry": evidence::registration(registry.classification(&appearance.api)),
            "vm": vm.vm(&appearance.api), "runtime": runtime.references(&appearance.api),
            "required_service": evidence::required_service(&appearance.api),
            "frontends": {"tui": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}, "browser": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}, "tauri": {"status": "unverified", "reason": "no_runtime_capability_handshake_or_execution_capture"}},
            "migration": evidence::migration(&appearance.api, &appearance.raw),
            "dynamic_verification": "not_run"
        })
    }).collect::<Vec<_>>();
    Ok(
        json!({"project": name, "inventory": inventory, "configuration_resolution": resolution,
        "analysis_identity": identity, "profile_override": arguments.profile, "configuration_valid": configuration_valid,
        "input_policy": {"script_root": if has_erb { "ERB_case_insensitive" } else { "recursive_project_fallback" }, "csv_root": if has_csv { "CSV_case_insensitive" } else { "recursive_project_fallback" }, "spans": "decoded_utf8_bytes", "encoding_fallback": "none_strict_UTF8_only_optional_BOM", "audit_options": "explicit_audit_defaults_or_JSON_overrides_not_inferred_game_semantics", "legacy_configuration_semantics": "not_applied_by_coverage_use_explicit_options_for_comparison", "appearance_parser_context": "DefaultParserContext_with_profile_lexer_switches_debug_symbol_and_ERH_macros; not full analyzer symbol resolution; continuation separator is parser default", "uncalled_functions": "included", "unresolved_links": unresolved_links},
        "inputs": inputs, "input_errors": errors, "parser_diagnostics": scan.diagnostics, "pipeline": pipeline, "api_counts": counts, "rows": rows}),
    )
}

fn markdown(report: &Value) -> String {
    let mut output = String::from(
        "# Snake compatibility coverage\n\nThis is static evidence, not a compatibility pass. Native registration and source references do not prove implementation. All runtime/frontend execution remains unverified.\n\n",
    );
    for project in report["projects"].as_array().into_iter().flatten() {
        output.push_str(&format!("## {}\n\n| API | Form | Location (UTF-8 bytes) | Activity | Analyzer | Compiler | Registry | VM | Classification / batch |\n|---|---|---|---|---|---|---|---|---|\n", project["project"].as_str().unwrap_or("?")));
        for row in project["rows"].as_array().into_iter().flatten() {
            let appearance = &row["appearance"];
            let cell = |value: &Value| {
                value
                    .as_str()
                    .unwrap_or("—")
                    .replace('|', "\\|")
                    .replace('\n', " ")
            };
            output.push_str(&format!(
                "| {} | {} | {}:{}-{} | {} | {} | {} | {} | {} | {} / {} |\n",
                cell(&appearance["api"]),
                cell(&appearance["form"]),
                cell(&appearance["path"]),
                appearance["span"]["start"],
                appearance["span"]["end"],
                cell(&appearance["activity"]),
                cell(&row["analyzer"]["status"]),
                cell(&row["compiler"]["status"]),
                cell(&row["registry"]["classification"]),
                cell(&row["vm"]["status"]),
                cell(&row["migration"]["classification"]),
                row["migration"]["batch"]
            ));
        }
        output.push('\n');
    }
    output
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
    let projects = if arguments.all_games {
        baseline::GAMES
            .iter()
            .map(|name| audit_project(&root.join(name), &arguments, &vm, &runtime))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![audit_project(&root, &arguments, &vm, &runtime)?]
    };
    let report = json!({"version": 1, "kind": "snake_compatibility_static_coverage", "hash_algorithm": "blake3", "tool_revision": baseline::git_identity(&core)?, "frontend_revisions": {"tui": baseline::git_identity(&workspace.join("rustyera-tui"))?, "web": baseline::git_identity(&workspace.join("rustyera-web"))?}, "source_evidence": {"vm": vm, "runtime": runtime}, "status_vocabulary": ["unknown", "compiler_trap", "unsupported_capability", "blocked", "unverified"], "unsupported_capability_policy": "only an actual capability handshake/rejection can establish this state; absent capture stays unverified", "projects": projects});
    if let Some(path) = &arguments.markdown {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(markdown(&report).as_bytes())?;
    }
    baseline::write_json(&report, arguments.output.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_roots_do_not_mix_archived_scripts_into_active_project() {
        assert!(is_input("ERB/sub/A.ERB", "erb", true, &["erb", "erh"]));
        assert!(!is_input("backup/A.ERB", "erb", true, &["erb", "erh"]));
        assert!(is_input(
            "CSV/name.ALS",
            "csv",
            true,
            &["csv", "als", "erd"]
        ));
    }

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
}
