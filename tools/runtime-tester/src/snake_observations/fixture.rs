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

pub(super) fn build_manifest(
    root: &Path,
    identity: &CompatibilityIdentity,
    group: &str,
    request: &Value,
) -> AuditResult<(ProjectManifest, Value)> {
    let group_file = match group {
        "PRINTC" => "printc",
        "arithmetic" => "arithmetic",
        "RNG" => "rng",
        "REF" => "ref",
        "extra_args" => "extra-args",
        "TOINT" => "toint",
        "GETKEY" => "getkey",
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
    let effective_sources: Vec<_> = files.iter().map(|file| {
        let FilePayload::Utf8(source) = &file.payload else { unreachable!() };
        json!({"path": file.relative_path, "bytes": source.len(), "sha256": hash(source.as_bytes())})
    }).collect();
    let harness = json!({
        "effectiveSources": effective_sources, "migrationDiagnostics": migration_diagnostics,
        "wrapper": wrapper(request)?, "baseTitleReplaced": "erb/base.erb",
    });
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
    fn wrapper_preserves_argument_expressions_and_omissions() {
        for arguments in ["7,", "7, COMPAT_SIDE_EFFECT()"] {
            let source =
                wrapper(&json!({"op": "run", "entry": "COMPAT_ONE", "arguments": arguments}))
                    .unwrap();
            assert!(source.contains(&format!("CALL COMPAT_ONE, {arguments}\n")));
        }
    }
}
