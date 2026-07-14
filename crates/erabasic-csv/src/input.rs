use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvSourceLocation};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontendIoErrorKind {
    NotFound,
    PermissionDenied,
    InvalidData,
    Interrupted,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrontendIoError {
    pub kind: FrontendIoErrorKind,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FilePayload {
    Utf8(String),
    IoError(FrontendIoError),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrontendFile {
    pub relative_path: String,
    pub payload: FilePayload,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectFiles {
    pub csv: Vec<FrontendFile>,
    pub erb: Vec<FrontendFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileRoot {
    Csv,
    Erb,
}

#[derive(Clone, Debug)]
pub(crate) struct IndexedFile {
    pub root: FileRoot,
    pub path: String,
    pub content: String,
    pub input_order: usize,
}

#[derive(Debug, Default)]
pub(crate) struct FileIndex {
    files: Vec<IndexedFile>,
    by_key: BTreeMap<(u8, String), usize>,
}

impl FileIndex {
    pub fn build(files: &ProjectFiles, diagnostics: &mut Vec<CsvDiagnostic>) -> Self {
        let mut result = Self::default();
        let mut input_order = 0;
        for (root, entries) in [(FileRoot::Csv, &files.csv), (FileRoot::Erb, &files.erb)] {
            for entry in entries {
                let Some(path) = normalize_path(&entry.relative_path) else {
                    diagnostics.push(CsvDiagnostic::new(
                        CsvDiagnosticCode::InvalidPath,
                        CsvDiagnosticSeverity::Error,
                        2,
                        &entry.relative_path,
                        None,
                        "paths must be relative and may not contain '..'",
                    ));
                    input_order += 1;
                    continue;
                };
                let FilePayload::Utf8(content) = &entry.payload else {
                    if let FilePayload::IoError(error) = &entry.payload
                        && error.kind != FrontendIoErrorKind::NotFound
                    {
                        diagnostics.push(CsvDiagnostic::new(
                            CsvDiagnosticCode::IoError,
                            CsvDiagnosticSeverity::Error,
                            2,
                            &path,
                            None,
                            format!("frontend I/O error: {}", error.message),
                        ));
                    }
                    input_order += 1;
                    continue;
                };
                let root_key = match root {
                    FileRoot::Csv => 0,
                    FileRoot::Erb => 1,
                };
                let key = (root_key, ascii_fold(&path));
                if result.by_key.contains_key(&key) {
                    diagnostics.push(CsvDiagnostic::new(
                        CsvDiagnosticCode::DuplicatePath,
                        CsvDiagnosticSeverity::Error,
                        2,
                        &path,
                        None,
                        "duplicate normalized path; the first file is used",
                    ));
                    input_order += 1;
                    continue;
                }
                let index = result.files.len();
                result.files.push(IndexedFile {
                    root,
                    path,
                    content: content.to_owned(),
                    input_order,
                });
                result.by_key.insert(key, index);
                input_order += 1;
            }
        }
        result
    }

    pub fn csv_file(&self, path: &str) -> Option<&IndexedFile> {
        self.by_key
            .get(&(0, ascii_fold(path)))
            .map(|index| &self.files[*index])
    }

    pub fn all(&self) -> impl Iterator<Item = &IndexedFile> {
        self.files.iter()
    }
}

fn normalize_path(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        return None;
    }
    let replaced = path.replace('\\', "/");
    if replaced.len() >= 2 && replaced.as_bytes()[1] == b':' {
        return None;
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub(crate) fn ascii_fold(value: &str) -> String {
    value.to_ascii_uppercase()
}

pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(crate) fn is_top_level(path: &str) -> bool {
    !path.contains('/')
}

pub(crate) fn source(
    path: &str,
    line: u32,
    byte_start: usize,
    byte_end: usize,
) -> CsvSourceLocation {
    CsvSourceLocation {
        relative_path: path.to_owned(),
        physical_line: line,
        logical_line: line,
        byte_start,
        byte_end,
    }
}
