//! Bounded data-only input loading for independent preset-ERD observation roots.
use super::{AuditResult, FileCategory, SubmittedFile, submitted};
use crate::project_inputs::ProjectInputs;
use std::{fs, io::Read, path::Path};

const MAX_ENTRIES: usize = 256;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn load_inputs(root: &Path, files: &mut Vec<SubmittedFile>) -> AuditResult<()> {
    let paths = data_paths(root)?;
    let inputs = ProjectInputs::new(root, &paths);
    let mut total_bytes = 0_u64;
    for relative in paths {
        let Some(category) = inputs.classify(&relative) else {
            continue;
        };
        if !matches!(
            category,
            FileCategory::Csv | FileCategory::Als | FileCategory::Erd | FileCategory::Erh
        ) || files.iter().any(|file| file.relative_path == relative)
        {
            continue;
        }
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_FILE_BYTES
        {
            return Err("preset ERD source must be a bounded regular file".into());
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or("preset ERD input size overflow")?;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err("preset ERD source inventory exceeds its byte limit".into());
        }
        let mut source = String::new();
        fs::File::open(path)?
            .take(MAX_FILE_BYTES + 1)
            .read_to_string(&mut source)?;
        if source.len() as u64 != metadata.len() {
            return Err("preset ERD source changed during loading".into());
        }
        files.push(submitted(&relative, category, source));
    }
    Ok(())
}

fn data_paths(root: &Path) -> AuditResult<Vec<String>> {
    let mut pending = vec![root.join("csv"), root.join("erb")];
    let mut paths = Vec::new();
    let mut entries = 0_usize;
    while let Some(directory) = pending.pop() {
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("preset ERD input directory must not be a symbolic link".into());
        }
        for entry in fs::read_dir(directory)? {
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err("preset ERD source inventory exceeds its entry limit".into());
            }
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err("preset ERD inputs must not be symbolic links".into());
            }
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .ok_or("preset ERD path is not UTF-8")?
                .replace('\\', "/");
            if relative.len() > 4096 || relative.split('/').count() > 16 {
                return Err("preset ERD source path exceeds its limit".into());
            }
            if metadata.is_dir() {
                pending.push(path);
            } else if metadata.is_file() {
                paths.push(relative);
            } else {
                return Err("preset ERD source is not a regular file".into());
            }
        }
    }
    // This is inventory order only. Production CSV loading chooses ERD merge order.
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_erd_group_submits_names_aliases_prices_and_character_sources() {
        let root = crate::tool_root().join("fixture-snake-upstream-c/main");
        let files = super::super::load_fixture_files(&root, "PRESET_ERD").unwrap();
        for (path, category) in [
            ("csv/ABL.CSV", FileCategory::Csv),
            ("csv/ABL.als", FileCategory::Als),
            ("csv/CHARA0.CSV", FileCategory::Csv),
            ("erb/preset_erd.erh", FileCategory::Erh),
            ("erb/deep/ABL.erd", FileCategory::Erd),
            ("erb/deep/ABL_EXTRA.erd", FileCategory::Erd),
            ("erb/deep/ITEMPRICE.erd", FileCategory::Erd),
        ] {
            let actual = files
                .iter()
                .find(|file| file.relative_path == path)
                .expect(path);
            assert_eq!(actual.category, category);
            assert_eq!(
                actual.payload,
                era_runtime_protocol::FilePayload::Utf8(
                    fs::read_to_string(root.join(path)).unwrap()
                )
            );
        }
        assert_eq!(
            files
                .iter()
                .filter(|file| file.relative_path == "csv/GAMEBASE.CSV")
                .count(),
            1
        );
        assert!(
            !files
                .iter()
                .any(|file| file.relative_path == "erb/base.erb")
        );
    }
}
