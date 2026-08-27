use std::collections::BTreeMap;

use erabasic_data::{
    DeferredIndexAliases, DeferredIndexCatalog, DeferredIndexFile, ProjectData, ResolvedUserIndex,
    UserIndexRegistration,
};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::{FileIndex, FileRoot, ascii_fold, basename, is_top_level},
    reader::enabled_lines,
    tables::{at_line, parse_alias_row},
};

#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub(crate) fn collect_deferred_indices(
    files: &FileIndex,
    options: &CsvLoadOptions,
) -> DeferredIndexCatalog {
    let mut catalog = DeferredIndexCatalog::default();
    if !options.use_erd {
        return catalog;
    }
    let mut candidates: Vec<_> = files
        .all()
        .filter(|file| {
            let name = ascii_fold(basename(&file.path));
            (file.root == FileRoot::Erb && name.ends_with(".ERD"))
                || (file.root == FileRoot::Csv
                    && is_top_level(&file.path)
                    && name.ends_with(".CSV"))
        })
        .collect();
    if options.compatibility.uses_snake_alias_rules() {
        // The reference enumerates ERD before CSV. Stable path order within each group
        // makes the insertion order portable rather than dependent on directory enumeration.
        candidates.sort_by(|left, right| {
            (left.root == FileRoot::Csv, &left.path)
                .cmp(&(right.root == FileRoot::Csv, &right.path))
        });
    } else {
        candidates.sort_by_key(|file| file.input_order);
    }
    for file in candidates {
        let filename = basename(&file.path);
        let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
        let aliases = if options.compatibility.uses_snake_alias_rules() {
            let path_stem = file
                .path
                .rsplit_once('.')
                .map_or(file.path.as_str(), |(stem, _)| stem);
            files
                .file(file.root, &format!("{path_stem}.als"))
                .map(|alias| DeferredIndexAliases {
                    relative_path: alias.source_path.clone(),
                    content: alias.content.clone(),
                })
        } else {
            None
        };
        catalog
            .groups
            .entry(ascii_fold(stem))
            .or_default()
            .push(DeferredIndexFile {
                relative_path: file.source_path.clone(),
                content: file.content.clone(),
                aliases,
            });
    }
    catalog
}

/// Resolve ERD/user-index CSVs after the parser has supplied the dimensions declared by
/// `#DIM`. Duplicate keys are fatal for that registration, matching `UserDefineLoadData`.
pub fn resolve_deferred_indices(
    project: &mut ProjectData,
    registrations: &[UserIndexRegistration],
    options: &CsvLoadOptions,
) -> Vec<CsvDiagnostic> {
    let mut diagnostics = Vec::new();
    if !options.use_erd {
        return diagnostics;
    }
    for registration in registrations {
        let result_name = registration.dimension.map_or_else(
            || registration.variable_name.clone(),
            |dimension| format!("{}@{dimension}", registration.variable_name),
        );
        if project
            .static_data
            .deferred_indices
            .resolved
            .contains_key(&result_name)
        {
            diagnostics.push(CsvDiagnostic::new(
                CsvDiagnosticCode::DuplicateUserIndexVariable,
                CsvDiagnosticSeverity::Fatal,
                3,
                "",
                None,
                format!("user index variable {result_name} is already resolved"),
            ));
            continue;
        }
        let stem = ascii_fold(&registration.source_stem);
        let Some(files) = project.static_data.deferred_indices.groups.get(&stem) else {
            continue;
        };
        if let Some(resolved) = resolve_registration(
            files,
            &result_name,
            registration.length,
            options,
            &mut diagnostics,
        ) {
            project
                .static_data
                .deferred_indices
                .resolved
                .insert(result_name, resolved);
        }
    }
    diagnostics
}

fn resolve_registration(
    files: &[DeferredIndexFile],
    result_name: &str,
    length: usize,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> Option<ResolvedUserIndex> {
    let mut combined = BTreeMap::new();
    let mut canonical_names = BTreeMap::new();
    for file in files {
        for (index, name) in load_primary_names(file, length, options, diagnostics) {
            if name.is_empty() {
                continue;
            }
            if let Some((_, previous_path)) = combined.get(&name) {
                diagnostics.push(CsvDiagnostic::new(
                    CsvDiagnosticCode::DuplicateUserIndex,
                    CsvDiagnosticSeverity::Fatal,
                    3,
                    &file.relative_path,
                    None,
                    format!(
                        "{result_name} key {name:?} is duplicated in {previous_path} and {}",
                        file.relative_path
                    ),
                ));
                return None;
            }
            canonical_names.entry(index).or_insert_with(|| name.clone());
            combined.insert(name, (index, &file.relative_path));
        }
    }
    let mut resolved = ResolvedUserIndex {
        variable_name: result_name.to_owned(),
        entries: combined
            .into_iter()
            .map(|(name, (index, _))| (name, index))
            .collect(),
        canonical_names,
    };
    if options.compatibility.uses_snake_alias_rules() {
        // All primary names must exist before any alias is admitted, even when its
        // primary definition occurs in a later file or in the other file root.
        for aliases in files.iter().filter_map(|file| file.aliases.as_ref()) {
            load_user_aliases(aliases, &mut resolved, options, diagnostics);
        }
    }
    Some(resolved)
}

fn load_primary_names(
    file: &DeferredIndexFile,
    length: usize,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> BTreeMap<i64, String> {
    let mut names = BTreeMap::new();
    for line in enabled_lines(&file.relative_path, &file.content, options, diagnostics) {
        let tokens: Vec<_> = line.text.split(',').collect();
        if tokens.len() < 2 {
            diagnostics.push(at_line(
                CsvDiagnosticCode::MissingComma,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "user index row requires an index and name",
            ));
            continue;
        }
        let Ok(index) = tokens[0].trim().parse::<i32>() else {
            diagnostics.push(at_line(
                CsvDiagnosticCode::InvalidInteger,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "user index is not an integer",
            ));
            continue;
        };
        if !usize::try_from(index).is_ok_and(|index| index < length) {
            diagnostics.push(out_of_range(&line));
            continue;
        }
        if names
            .insert(i64::from(index), tokens[1].to_owned())
            .is_some()
        {
            diagnostics.push(at_line(
                CsvDiagnosticCode::DuplicateIndex,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("user index {index} is defined more than once"),
            ));
        }
    }
    names
}

fn load_user_aliases(
    file: &DeferredIndexAliases,
    resolved: &mut ResolvedUserIndex,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    for line in enabled_lines(&file.relative_path, &file.content, options, diagnostics) {
        let Some((index, name)) = parse_alias_row(&line, diagnostics) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || resolved.entries.contains_key(name) {
            continue;
        }
        let index = i64::from(index);
        resolved.entries.insert(name.to_owned(), index);
        resolved
            .canonical_names
            .entry(index)
            .or_insert_with(|| name.to_owned());
    }
}

fn out_of_range(line: &crate::reader::EnabledLine) -> CsvDiagnostic {
    at_line(
        CsvDiagnosticCode::IndexOutOfRange,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        "user index is outside the declared dimension",
    )
}
