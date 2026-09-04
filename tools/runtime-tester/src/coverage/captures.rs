//! Optional local capture provenance. Hash binding is deliberately not a test-result interpreter.

use erabasic_compat::CompatibilityIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST: u64 = 4 * 1024 * 1024;
const MAX_CAPTURE: u64 = 1024 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    project: String,
    api: String,
    frontend: String,
    compatibility: CompatibilityIdentity,
    core_identity: Value,
    source_hashes: BTreeMap<String, SourceHashes>,
    capture: PathBuf,
    capture_sha256: String,
    description: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceHashes {
    raw_blake3: Option<String>,
    decoded_utf8_blake3: Option<String>,
}

#[derive(Default)]
pub(super) struct Captures {
    entries: Vec<(Entry, Value)>,
    core_identity: Value,
}

impl Captures {
    pub fn load(path: Option<&Path>, core_identity: Value) -> Result<Self, Box<dyn Error>> {
        let Some(path) = path else {
            return Ok(Self {
                core_identity,
                ..Self::default()
            });
        };
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_MANIFEST {
            return Err("capture manifest exceeds 4 MiB".into());
        }
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        if manifest.version != 1 || manifest.entries.len() > 10_000 {
            return Err("unsupported or excessive capture manifest".into());
        }
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()?;
        let mut entries = Vec::new();
        for entry in manifest.entries {
            if entry.source_hashes.is_empty()
                || entry.source_hashes.len() > 100_000
                || !matches!(
                    entry.frontend.as_str(),
                    "core" | "tui" | "browser" | "tauri" | "oracle_original" | "oracle_snake"
                )
            {
                return Err(
                    "capture needs source hashes and a recognized observation frontend".into(),
                );
            }
            let safe = !entry.capture.as_os_str().is_empty()
                && entry
                    .capture
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)));
            let checked = if safe {
                root.join(&entry.capture)
                    .canonicalize()
                    .ok()
                    .filter(|path| path.starts_with(&root))
            } else {
                None
            };
            let verification = checked.map_or_else(|| json!({"status": "unverified_unsafe_or_missing_capture_path"}), |capture| {
                match super::output::hash_path(&capture, MAX_CAPTURE) {
                    Ok(hash) if hash.sha256 == entry.capture_sha256 => json!({"status": "capture_bytes_verified", "digest": hash}),
                    Ok(hash) => json!({"status": "unverified_capture_hash_mismatch", "actual_digest": hash}),
                    Err(error) => json!({"status": "unverified_capture_read_error", "error": error.to_string()}),
                }
            });
            entries.push((entry, verification));
        }
        Ok(Self {
            entries,
            core_identity,
        })
    }

    pub fn bind(
        &self,
        project: &str,
        compatibility: &CompatibilityIdentity,
        inputs: &[Value],
    ) -> Value {
        let by_path = inputs
            .iter()
            .filter_map(|input| input["path"].as_str().map(|path| (path, input)))
            .collect::<BTreeMap<_, _>>();
        let entries = self.entries.iter().enumerate().filter(|(_, (entry, _))| entry.project == project).map(|(id, (entry, verification))| {
            let mut mismatches = Vec::new();
            if entry.compatibility != *compatibility { mismatches.push(json!("compatibility_identity_mismatch")); }
            if entry.core_identity != self.core_identity { mismatches.push(json!("core_identity_mismatch")); }
            for (path, expected) in &entry.source_hashes {
                let input = by_path.get(path.as_str());
                let matches = input.is_some_and(|input| (expected.raw_blake3.is_some() || expected.decoded_utf8_blake3.is_some())
                    && expected.raw_blake3.as_ref().is_none_or(|hash| input["raw_blake3"] == *hash)
                    && expected.decoded_utf8_blake3.as_ref().is_none_or(|hash| input["decoded_utf8_blake3"] == *hash));
                if !matches { mismatches.push(json!({"source_hash_mismatch": path})); }
            }
            let bound = mismatches.is_empty() && verification["status"] == "capture_bytes_verified";
            json!({"id": id, "api": entry.api.to_ascii_uppercase(), "frontend": entry.frontend, "capture": entry.capture,
                "description": entry.description, "verification": verification, "binding_mismatches": mismatches,
                "source_hashes": entry.source_hashes, "core_identity": entry.core_identity, "compatibility": entry.compatibility,
                "binding_status": if bound { "bound_capture" } else { "unverified" },
                "execution_status": "unverified_capture_requires_behavior_review",
                "scope": "only_declared_source_hashes_not_all_game_occurrences",
                "proof_boundary": "hashes_verify_provenance_not_binary_origin_outcomes_or_capability_rejection"})
        }).collect::<Vec<_>>();
        json!({"status": if entries.is_empty() { "unverified_no_capture" } else { "capture_provenance_reported" }, "entries": entries})
    }
}

pub(super) fn api_refs(captures: &Value, api: &str) -> Vec<Value> {
    captures["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["binding_status"] == "bound_capture" && entry["api"] == api)
        .map(|entry| entry["id"].clone())
        .collect()
}
