//! Runtime-owned Data/Resource sequencing for the snake text namespace.

use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;

#[allow(clippy::wildcard_imports)]
use super::super::*;

const MAXIMUM_TEXT_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_PATH_BYTES: usize = 4096;
const MAXIMUM_LIST_ENTRIES: usize = 100_000;
const MAXIMUM_LIST_PATH_BYTES: usize = 8 * 1024 * 1024;

impl RuntimeSession {
    pub(in crate::session) fn resource_storage_path(path: &str, directory: bool) -> Option<String> {
        if path.len() > MAXIMUM_PATH_BYTES || path.contains('\0') {
            return None;
        }
        if directory && (path.is_empty() || path == ".") {
            return Some(String::new());
        }
        let normalized: String = era_runtime_protocol::validate_relative_path(path)
            .ok()?
            .nfc()
            .collect();
        (normalized.len() <= MAXIMUM_PATH_BYTES).then_some(normalized)
    }

    pub(in crate::session) fn resource_storage_pattern_valid(pattern: Option<&str>) -> bool {
        era_runtime_protocol::storage_pattern::validate_snake_storage_pattern(pattern).is_ok()
    }

    pub(in crate::session) fn complete_resource_storage(
        &mut self,
        mut pending: PendingStorage,
        result: StorageResult,
    ) -> Result<(), RuntimeError> {
        let (request, path, resource, text) = match &mut pending {
            PendingStorage::HostResourceText {
                request,
                path,
                resource,
            } => (*request, path, resource, true),
            PendingStorage::HostResourceStat {
                request,
                path,
                resource,
            } => (*request, path, resource, false),
            PendingStorage::HostResourceList { .. } => {
                return self.complete_resource_list(pending, result);
            }
            _ => {
                return Err(RuntimeError::Internal(
                    "not a resource storage continuation".into(),
                ));
            }
        };
        if !*resource
            && matches!(&result, StorageResult::Error { error } if error.kind == FrontendIoErrorKind::NotFound)
        {
            *resource = true;
            let path = path.clone();
            // Keep the VM's original pending host request. Only the public storage request ID
            // advances, so late/duplicate Data replies cannot complete the Resource request.
            return self.issue_storage(
                pending,
                StorageNamespace::Resource,
                if text {
                    StorageOperation::Read
                } else {
                    StorageOperation::Stat
                },
                path,
            );
        }
        let value = match (text, result) {
            (true, StorageResult::Read { data, .. }) => {
                let text = if data.as_slice().len() <= MAXIMUM_TEXT_BYTES {
                    decode_load_text(data.as_slice()).unwrap_or_default()
                } else {
                    String::new()
                };
                VmValue::String(text)
            }
            (false, StorageResult::Metadata(_)) => VmValue::Integer(1),
            (true, StorageResult::Error { .. }) => VmValue::String(String::new()),
            (false, StorageResult::Error { .. }) => VmValue::Integer(0),
            _ => {
                return self.fault(
                    FaultCode::ServiceFailure,
                    "storage response kind differs from its resource request",
                    None,
                );
            }
        };
        self.resume_storage_host_value(request, value, Vec::new())
    }

    fn complete_resource_list(
        &mut self,
        pending: PendingStorage,
        result: StorageResult,
    ) -> Result<(), RuntimeError> {
        let PendingStorage::HostResourceList {
            request,
            target,
            directory,
            pattern,
            recursive,
            data_paths,
        } = pending
        else {
            return Err(RuntimeError::Internal(
                "not a resource listing continuation".into(),
            ));
        };
        let entries = match result {
            StorageResult::Listed { entries } => entries,
            StorageResult::Error { error } if error.kind == FrontendIoErrorKind::NotFound => {
                Vec::new()
            }
            StorageResult::Error { .. } => {
                return self.resume_storage_host_value(request, VmValue::Integer(-1), Vec::new());
            }
            _ => {
                return self.fault(
                    FaultCode::ServiceFailure,
                    "storage response kind differs from its resource listing",
                    None,
                );
            }
        };
        let Some(paths) = validated_paths(&entries, &directory, recursive) else {
            return self.resume_storage_host_value(request, VmValue::Integer(-1), Vec::new());
        };
        let Some(data_paths) = data_paths else {
            return self.issue_storage(
                PendingStorage::HostResourceList {
                    request,
                    target,
                    directory: directory.clone(),
                    pattern: pattern.clone(),
                    recursive,
                    data_paths: Some(paths),
                },
                StorageNamespace::Resource,
                StorageOperation::List { pattern, recursive },
                directory,
            );
        };
        let Some(values) = merged_paths(data_paths, paths) else {
            return self.resume_storage_host_value(request, VmValue::Integer(-1), Vec::new());
        };
        let writes = self.file_list_writes(target, &values)?;
        self.resume_storage_host_value(
            request,
            VmValue::Integer(i64::try_from(values.len()).expect("bounded file count fits i64")),
            writes,
        )
    }
}

fn validated_paths(
    entries: &[era_runtime_protocol::StorageEntry],
    directory: &str,
    recursive: bool,
) -> Option<Vec<String>> {
    if entries.len() > MAXIMUM_LIST_ENTRIES {
        return None;
    }
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{}/", directory.to_lowercase())
    };
    let mut seen = BTreeSet::new();
    let mut retained_bytes = 0_usize;
    let mut paths = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = RuntimeSession::resource_storage_path(&entry.relative_path, false)?;
        let key = path.to_lowercase();
        let tail = key.strip_prefix(&prefix)?;
        if tail.is_empty() || (!recursive && tail.contains('/')) || !seen.insert(key) {
            return None;
        }
        retained_bytes = retained_bytes.checked_add(path.len())?;
        if retained_bytes > MAXIMUM_LIST_PATH_BYTES {
            return None;
        }
        paths.push(path);
    }
    Some(paths)
}

fn merged_paths(data: Vec<String>, resources: Vec<String>) -> Option<Vec<String>> {
    let mut merged = BTreeMap::new();
    let mut retained_bytes = 0_usize;
    // Insert Data last: it owns the contents and spelling of every normalized shared path.
    for path in resources.into_iter().chain(data) {
        let key = path.to_lowercase();
        if let Some(previous) = merged.insert(key, path.clone()) {
            retained_bytes = retained_bytes.checked_sub(previous.len())?;
        }
        retained_bytes = retained_bytes.checked_add(path.len())?;
        if merged.len() > MAXIMUM_LIST_ENTRIES || retained_bytes > MAXIMUM_LIST_PATH_BYTES {
            return None;
        }
    }
    let mut paths = merged.into_values().collect::<Vec<_>>();
    paths.sort();
    Some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: String) -> era_runtime_protocol::StorageEntry {
        era_runtime_protocol::StorageEntry {
            relative_path: path,
            byte_length: 0,
            revision: None,
            change_token: None,
        }
    }

    #[test]
    fn listing_paths_enforce_scope_canonical_collisions_and_budgets() {
        for path in [
            "../secret",
            "/secret",
            "elsewhere/a.xml",
            "plugins",
            "plugins/deep/a.xml",
            "plugins/a\0.xml",
        ] {
            assert!(
                validated_paths(&[entry(path.into())], "plugins", false).is_none(),
                "{path}"
            );
        }
        assert_eq!(
            validated_paths(&[entry("plugins\\a.xml".into())], "plugins", false).unwrap(),
            ["plugins/a.xml"]
        );
        assert!(
            validated_paths(
                &[
                    entry("plugins/é.xml".into()),
                    entry("PLUGINS/e\u{301}.xml".into())
                ],
                "plugins",
                false
            )
            .is_none()
        );
        assert!(validated_paths(&[entry("x".repeat(MAXIMUM_PATH_BYTES + 1))], "", true).is_none());
        let too_many = (0..=MAXIMUM_LIST_ENTRIES)
            .map(|index| entry(format!("{index}.xml")))
            .collect::<Vec<_>>();
        assert!(validated_paths(&too_many, "", false).is_none());
        let too_long = (0..2200)
            .map(|index| entry(format!("{index:04}{}", "x".repeat(4000))))
            .collect::<Vec<_>>();
        assert!(validated_paths(&too_long, "", false).is_none());
    }

    #[test]
    fn merging_prefers_data_spelling_and_sorts_final_paths() {
        assert_eq!(
            merged_paths(
                vec!["plugins/É.xml".into(), "plugins/a.xml".into()],
                vec!["plugins/é.xml".into(), "plugins/z.xml".into()]
            )
            .unwrap(),
            ["plugins/a.xml", "plugins/z.xml", "plugins/É.xml"]
        );
    }
}
