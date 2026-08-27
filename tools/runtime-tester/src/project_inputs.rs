//! Shared project input classification for runtime, extractor, and coverage audits.

use std::{collections::BTreeSet, fs, path::Path};

use era_protocol::ProtocolBytes;
use era_runtime_protocol::{FileCategory, FilePayload, SubmittedFile};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DataRoot {
    Csv,
    Erb,
}

impl DataRoot {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Erb => "erb",
        }
    }

    pub(super) fn relative_path(self, path: &str) -> String {
        path.split_once('/')
            .filter(|(first, _)| first.eq_ignore_ascii_case(self.name()))
            .map_or(path, |(_, rest)| rest)
            .to_owned()
    }
}

pub(super) struct ProjectInputs {
    pub(super) has_csv: bool,
    pub(super) has_erb: bool,
    erd_aliases: BTreeSet<String>,
    aliases: BTreeSet<String>,
}

impl ProjectInputs {
    pub(super) fn new(root: &Path, paths: &[String]) -> Self {
        Self::with_roots(
            super::has_direct_child_directory(root, "CSV"),
            super::has_direct_child_directory(root, "ERB"),
            paths,
        )
    }

    fn with_roots(has_csv: bool, has_erb: bool, paths: &[String]) -> Self {
        let mut result = Self {
            has_csv,
            has_erb,
            erd_aliases: BTreeSet::new(),
            aliases: BTreeSet::new(),
        };
        for path in paths {
            match result.classify(path) {
                Some(FileCategory::Erd) => {
                    result.erd_aliases.insert(alias_path(path));
                }
                Some(FileCategory::Als) => {
                    result.aliases.insert(path.to_ascii_lowercase());
                }
                _ => {}
            }
        }
        result
    }

    pub(super) fn classify(&self, path: &str) -> Option<FileCategory> {
        let lower = path.to_ascii_lowercase();
        let first = lower.split('/').next().unwrap_or_default();
        let name = lower.rsplit('/').next().unwrap_or_default();
        let extension = name.rsplit('.').next().unwrap_or_default();
        if matches!(name, "reraconfig.toml" | "setting.json") {
            return Some(FileCategory::Configuration);
        }
        if matches!(extension, "xml" | "txt" | "db" | "sqlite") {
            return (!matches!(
                first,
                ".git" | ".rustyera" | "sav" | "save" | "saves" | "data" | "log" | "logs"
            ))
            .then_some(FileCategory::Resource);
        }
        if first == "resources" {
            return match extension {
                "csv" => Some(FileCategory::ResourceManifest),
                "bmp" | "gif" | "jpeg" | "jpg" | "png" | "webp" | "aac" | "flac" | "m4a"
                | "mp3" | "ogg" | "opus" | "wav" => Some(FileCategory::Resource),
                _ => None,
            };
        }
        if first == "sound" {
            return matches!(
                extension,
                "aac" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav"
            )
            .then_some(FileCategory::Resource);
        }
        if first == "font" {
            return matches!(extension, "otf" | "ttc" | "ttf" | "woff" | "woff2")
                .then_some(FileCategory::Resource);
        }
        match extension {
            "csv" if !self.has_csv || first == "csv" => Some(FileCategory::Csv),
            "erb" if !self.has_erb || first == "erb" => Some(FileCategory::Erb),
            "erh" if !self.has_erb || first == "erb" => Some(FileCategory::Erh),
            "erd" if !self.has_erb || first == "erb" => Some(FileCategory::Erd),
            "als" if !(self.has_csv || self.has_erb) || matches!(first, "csv" | "erb") => {
                Some(FileCategory::Als)
            }
            "config" if !self.has_csv || !lower.contains('/') || first == "csv" => {
                Some(FileCategory::Configuration)
            }
            _ => None,
        }
    }

    pub(super) fn data_root(&self, path: &str, category: FileCategory) -> Option<DataRoot> {
        match category {
            FileCategory::Csv => Some(DataRoot::Csv),
            FileCategory::Erd => Some(DataRoot::Erb),
            FileCategory::Als => {
                let lower = path.to_ascii_lowercase();
                Some(
                    if lower.starts_with("erb/") || self.erd_aliases.contains(&lower) {
                        DataRoot::Erb
                    } else {
                        DataRoot::Csv
                    },
                )
            }
            _ => None,
        }
    }

    fn submitted_path(&self, path: &str, category: FileCategory, keep_roots: bool) -> String {
        // ALS roots disambiguate identically named CSV and ERD aliases. Preserve the
        // associated CSV path too, while retaining legacy minimal paths for old inputs.
        if keep_roots
            || matches!(category, FileCategory::Als | FileCategory::Erd)
            || (category == FileCategory::Csv && self.aliases.contains(&alias_path(path)))
            || matches!(
                category,
                FileCategory::Resource | FileCategory::ResourceManifest
            )
        {
            return path.to_owned();
        }
        path.split_once('/')
            .filter(|(first, _)| {
                first.eq_ignore_ascii_case("csv") || first.eq_ignore_ascii_case("erb")
            })
            .map_or(path, |(_, rest)| rest)
            .to_owned()
    }

    pub(super) fn submitted_files(
        &self,
        root: &Path,
        paths: &[String],
        keep_roots: bool,
    ) -> Vec<SubmittedFile> {
        paths
            .iter()
            .filter_map(|path| {
                let category = self.classify(path)?;
                let (payload, hash) = if category == FileCategory::Resource {
                    let bytes = fs::read(root.join(path)).expect("read submitted project resource");
                    let hash = blake3::hash(&bytes);
                    (FilePayload::Bytes(ProtocolBytes::new(bytes)), hash)
                } else {
                    let text = super::read_submitted_text(root.join(path), category)
                        .expect("decode submitted project source");
                    let hash = blake3::hash(text.as_bytes());
                    (FilePayload::Utf8(text), hash)
                };
                Some(SubmittedFile {
                    relative_path: self.submitted_path(path, category, keep_roots),
                    category,
                    payload,
                    content_hash: Some(ProtocolBytes::new(hash.as_bytes().to_vec())),
                })
            })
            .collect()
    }
}

fn alias_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let stem = lower
        .rsplit_once('.')
        .map_or(lower.as_str(), |(stem, _)| stem);
    format!("{stem}.als")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_roots_exclude_uninstalled_aliases_and_erd() {
        let inputs = ProjectInputs::with_roots(true, true, &[]);
        for (path, category) in [
            ("CSV/nested/BUFF.ALS", FileCategory::Als),
            ("ERB/nested/BUFF.als", FileCategory::Als),
            ("ERB/nested/BUFF.ERD", FileCategory::Erd),
            ("ERB/main.erb", FileCategory::Erb),
        ] {
            assert_eq!(inputs.classify(path), Some(category), "{path}");
        }
        for path in [
            "backup/BUFF.als",
            "backup/BUFF.erd",
            "CSV/BUFF.erd",
            "backup/main.erb",
        ] {
            assert_eq!(inputs.classify(path), None, "{path}");
        }
        let csv_only = ProjectInputs::with_roots(true, false, &[]);
        assert_eq!(csv_only.classify("backup/BUFF.als"), None);
        assert_eq!(
            csv_only.classify("nested/BUFF.erd"),
            Some(FileCategory::Erd)
        );
        let erb_only = ProjectInputs::with_roots(false, true, &[]);
        assert_eq!(erb_only.classify("backup/BUFF.als"), None);
    }

    #[test]
    fn flat_aliases_follow_same_directory_erd_and_minimal_paths_keep_root_identity() {
        let paths = ["nested/BUFF.erd", "nested/BUFF.als", "BUFF.als"].map(str::to_owned);
        let inputs = ProjectInputs::with_roots(false, false, &paths);
        assert_eq!(inputs.classify("nested/BUFF.als"), Some(FileCategory::Als));
        assert_eq!(
            inputs.data_root("nested/BUFF.als", FileCategory::Als),
            Some(DataRoot::Erb)
        );
        assert_eq!(
            inputs.data_root("BUFF.als", FileCategory::Als),
            Some(DataRoot::Csv)
        );

        let paths = [
            "CSV/BUFF.csv",
            "CSV/BUFF.als",
            "ERB/BUFF.erd",
            "ERB/BUFF.als",
        ]
        .map(str::to_owned);
        let inputs = ProjectInputs::with_roots(true, true, &paths);
        for path in &paths {
            let category = inputs.classify(path).unwrap();
            assert_eq!(inputs.submitted_path(path, category, false), *path);
        }
        assert_eq!(
            inputs.submitted_path("CSV/GAMEBASE.csv", FileCategory::Csv, false),
            "GAMEBASE.csv"
        );
        assert_eq!(
            inputs.submitted_path("ERB/main.erb", FileCategory::Erb, false),
            "main.erb"
        );
    }
}
