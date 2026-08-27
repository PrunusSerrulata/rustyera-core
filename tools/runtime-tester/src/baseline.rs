//! Read-only, content-addressed inventories independent of current frontend ingestion.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

pub(super) const GAMES: &[&str] = &[
    "eratw-sub-modding",
    "eraAkumaMaid",
    "eraMaouEx",
    "eraTW",
    "erafl",
    "erarorona",
    "eratohoK",
    "era魔界牧場1.050_tc8",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct FileIdentity {
    pub path: String,
    pub bytes: u64,
    pub blake3: String,
    pub group: String,
    pub database_role: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct GitIdentity {
    pub sha: String,
    pub dirty_entries: Vec<String>,
    pub tracked_patch_blake3: String,
    pub untracked_blake3: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Inventory {
    pub project: String,
    pub git: Option<GitIdentity>,
    pub files: Vec<FileIdentity>,
    pub excluded: BTreeMap<String, String>,
    pub group_hashes: BTreeMap<String, String>,
    pub content_hash: String,
}

pub(super) fn git_identity(root: &Path) -> io::Result<Option<GitIdentity>> {
    // A non-Git game must not inherit the outer workspace repository identity.
    if !root.join(".git").exists() {
        return Ok(None);
    }
    let git = |args: &[&str]| -> io::Result<String> {
        super::watchdog::publish(
            serde_json::json!({"phase": "git_identity", "case": root, "pending": {"command": "git", "arguments": args}, "lastFullResponse": null}),
        )?;
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other("cannot read repository identity"));
        }
        String::from_utf8(output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    };
    let sha = git(&["rev-parse", "HEAD"])?.trim().to_owned();
    let dirty_entries = git(&["status", "--porcelain=v1", "-z"])?
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect();
    let tracked_patch_blake3 = blake3::hash(git(&["diff", "--binary", "HEAD", "--"])?.as_bytes())
        .to_hex()
        .to_string();
    let mut untracked_blake3 = BTreeMap::new();
    for relative in git(&["ls-files", "--others", "--exclude-standard", "-z"])?
        .split('\0')
        .filter(|path| !path.is_empty())
    {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)?;
        let hash = if metadata.is_symlink() {
            blake3::hash(fs::read_link(path)?.to_string_lossy().as_bytes())
                .to_hex()
                .to_string()
        } else if metadata.is_file() {
            hash_file(&path)?.1
        } else {
            continue;
        };
        untracked_blake3.insert(relative.into(), hash);
    }
    Ok(Some(GitIdentity {
        sha,
        dirty_entries,
        tracked_patch_blake3,
        untracked_blake3,
    }))
}

fn exclusion(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    let parts = lower.split('/').collect::<Vec<_>>();
    if parts.iter().any(|part| {
        matches!(
            *part,
            ".git" | ".rustyera" | ".cache" | "target" | "node_modules" | ".venv"
        )
    }) {
        return Some("development_or_compiled_cache");
    }
    if parts
        .first()
        .is_some_and(|part| matches!(*part, "sav" | "save" | "logs" | "log"))
    {
        return Some("runtime_save_or_log_directory");
    }
    let name = parts.last().copied().unwrap_or_default();
    if matches!(
        name,
        ".ds_store" | "lazyloading.bin" | "lazyloadingfiles.bin"
    ) || [
        ".sav",
        ".rerasav",
        ".reraproj",
        ".log",
        ".db-wal",
        ".db-shm",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
    {
        return Some("runtime_or_cache_artifact");
    }
    if lower.starts_with("data/sql/") {
        return Some("runtime_database_overlay");
    }
    None
}

fn group(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let extension = lower.rsplit('.').next().unwrap_or_default();
    if matches!(extension, "erb" | "erh" | "csv" | "als" | "erd") {
        "source"
    } else if matches!(extension, "config" | "cfg")
        || matches!(
            lower.as_str(),
            "reraconfig.toml" | "setting.json" | "macro.txt"
        )
    {
        "configuration"
    } else {
        "resource"
    }
}

fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    super::watchdog::publish(
        serde_json::json!({"phase": "hash_file", "case": path, "pending": "open", "bytes_read": 0, "lastFullResponse": null}),
    )?;
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 1024 * 1024];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += u64::try_from(count).unwrap_or(u64::MAX);
        hasher.update(&buffer[..count]);
        super::watchdog::publish(
            serde_json::json!({"phase": "hash_file", "case": path, "pending": "read", "bytes_read": bytes, "lastFullResponse": null}),
        )?;
    }
    Ok((bytes, hasher.finalize().to_hex().to_string()))
}

fn collect(root: &Path, current: &Path, inventory: &mut Inventory) -> io::Result<()> {
    super::watchdog::publish(
        serde_json::json!({"phase": "inventory_directory", "case": inventory.project, "pending": current, "files_completed": inventory.files.len(), "excluded": inventory.excluded, "lastFullResponse": null}),
    )?;
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        let relative = relative
            .to_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 inventory path"))?
            .replace('\\', "/");
        if let Some(reason) = exclusion(&relative) {
            inventory.excluded.insert(relative, reason.into());
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            inventory
                .excluded
                .insert(relative, "unresolved_symlink_not_followed".into());
        } else if kind.is_dir() {
            collect(root, &path, inventory)?;
        } else if kind.is_file() {
            let (bytes, blake3) = hash_file(&path)?;
            let database_role = [".db", ".sqlite", ".sqlite3"]
                .iter()
                .any(|suffix| relative.to_ascii_lowercase().ends_with(suffix))
                .then(|| "source_database_seed_or_input_not_runtime_overlay".into());
            inventory.files.push(FileIdentity {
                group: group(&relative).into(),
                path: relative,
                bytes,
                blake3,
                database_role,
            });
        } else {
            inventory
                .excluded
                .insert(relative, "unsupported_file_kind".into());
        }
    }
    Ok(())
}

fn digest<'a>(files: impl Iterator<Item = &'a FileIdentity>) -> String {
    let mut hash = blake3::Hasher::new_derive_key("rustyera.compatibility.baseline.v1");
    for file in files {
        hash.update(
            &u64::try_from(file.path.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        hash.update(file.path.as_bytes());
        hash.update(&file.bytes.to_le_bytes());
        hash.update(file.blake3.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

pub(super) fn inventory(root: &Path, project: &str) -> io::Result<Inventory> {
    let mut result = Inventory {
        project: project.into(),
        git: git_identity(root)?,
        files: Vec::new(),
        excluded: BTreeMap::new(),
        group_hashes: BTreeMap::new(),
        content_hash: String::new(),
    };
    collect(root, root, &mut result)?;
    result
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    for category in ["source", "configuration", "resource"] {
        result.group_hashes.insert(
            category.into(),
            digest(result.files.iter().filter(|file| file.group == category)),
        );
    }
    result.content_hash = digest(result.files.iter());
    Ok(result)
}

pub(super) fn write_json(
    value: &impl Serialize,
    destination: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    super::watchdog::publish(
        serde_json::json!({"phase": "serialize_report", "case": destination, "pending": "JSON", "lastFullResponse": null}),
    )?;
    if let Some(path) = destination {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        write_report(io::BufWriter::new(file), value, destination)?;
    } else {
        write_report(io::stdout().lock(), value, destination)?;
    }
    Ok(())
}

fn write_report(
    writer: impl Write,
    value: &impl Serialize,
    destination: Option<&Path>,
) -> io::Result<()> {
    struct ProgressWriter<'a, W> {
        writer: W,
        destination: Option<&'a Path>,
        bytes: u64,
        published: u64,
    }
    impl<W: Write> Write for ProgressWriter<'_, W> {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let count = self.writer.write(buffer)?;
            self.bytes += count as u64;
            if self.bytes - self.published >= 1024 * 1024 {
                super::watchdog::publish(
                    serde_json::json!({"phase": "write_report", "case": self.destination, "pending": "JSON", "bytes_written": self.bytes, "lastFullResponse": null}),
                )?;
                self.published = self.bytes;
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.writer.flush()
        }
    }
    let mut writer = ProgressWriter {
        writer,
        destination,
        bytes: 0,
        published: 0,
    };
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

pub(super) fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args_os().skip(2).collect::<Vec<_>>();
    if arguments.len() > 2 {
        return Err("usage: baseline [GAMES_ROOT] [OUTPUT.json]".into());
    }
    let workspace = super::repository_root()
        .parent()
        .ok_or("missing workspace parent")?
        .to_owned();
    let root = arguments
        .first()
        .map_or_else(|| workspace.join("games"), std::path::PathBuf::from);
    let games = GAMES
        .iter()
        .map(|name| inventory(&root.join(name), name))
        .collect::<Result<Vec<_>, _>>()?;
    let oracles = [("emuera.em", "26a35dc9334bb67590b96f7b8efbefbf199e391e", "af9886061ba420d530581e7975c4db735c391d03"), ("emuera_lazyloading_selfmodified_version", "fc4fb21416768c17256d0e82f997e5f99c9bba91", "4a46d7b52280733e8ecb8eeb630a87facdc03a23")]
        .into_iter().map(|(repository, semantic, wrapper_base)| Ok(serde_json::json!({"repository": repository, "semantic_sha": semantic, "wrapper_base_sha": wrapper_base, "wrapper_current": git_identity(&workspace.join(repository))?})))
        .collect::<io::Result<Vec<_>>>()?;
    let components = ["rustyera-core", "rustyera-tui", "rustyera-web"]
        .into_iter()
        .map(|name| Ok((name, git_identity(&workspace.join(name))?)))
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    let report = serde_json::json!({"version": 1, "hash_algorithm": "blake3", "core_batch0_base_sha": "aa5cd34e9f11346ee6a66e3ab9c4978c92137103", "components": components, "oracles": oracles, "games": games});
    write_json(&report, arguments.get(1).map(Path::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_keeps_future_inputs_and_distinguishes_runtime_artifacts() {
        for file in [
            "CSV/names.als",
            "CSV/array.erd",
            "plugins/qol_data.db",
            "resources/portrait.webp",
            "lazyloading.cfg",
            "emuera.config",
        ] {
            assert_eq!(exclusion(file), None, "{file}");
        }
        for file in [
            "sav/save00.sav",
            ".rustyera/cache/file",
            "last_log.log",
            "lazyloading.bin",
            "Data/sql/qol.db",
        ] {
            assert!(exclusion(file).is_some(), "{file}");
        }
        assert_eq!(group("CSV/names.als"), "source");
        assert_eq!(group("reraconfig.toml"), "configuration");
    }

    #[test]
    fn digest_changes_for_resource_content_and_relative_path() {
        let mut file = FileIdentity {
            path: "resources/a.webp".into(),
            bytes: 3,
            blake3: blake3::hash(b"abc").to_hex().to_string(),
            group: "resource".into(),
            database_role: None,
        };
        let original = digest(std::iter::once(&file));
        file.path = "resources/b.webp".into();
        assert_ne!(digest(std::iter::once(&file)), original);
        file.path = "resources/a.webp".into();
        file.blake3 = blake3::hash(b"def").to_hex().to_string();
        assert_ne!(digest(std::iter::once(&file)), original);
    }
}
