use std::collections::{BTreeMap, BTreeSet};

use erabasic_data::{
    DeferredIndexCatalog, DeferredIndexFile, ProjectData, ResolvedUserIndex, UserIndexRegistration,
};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::{FileIndex, FileRoot, ascii_fold, basename, is_top_level},
    reader::enabled_lines,
    tables::at_line,
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
    candidates.sort_by_key(|file| file.input_order);
    for file in candidates {
        let filename = basename(&file.path);
        let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
        catalog
            .groups
            .entry(ascii_fold(stem))
            .or_default()
            .push(DeferredIndexFile {
                relative_path: file.path.clone(),
                content: file.content.clone(),
            });
    }
    catalog
}

/// Resolve ERD/user-index CSVs after the parser has supplied the dimensions declared by
/// `#DIM`. Duplicate keys are fatal for that registration, matching `UserDefineLoadData`.
#[allow(clippy::too_many_lines)]
pub fn resolve_deferred_indices(
    project: &mut ProjectData,
    registrations: &[UserIndexRegistration],
    options: &CsvLoadOptions,
) -> Vec<CsvDiagnostic> {
    let mut diagnostics = Vec::new();
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
        let mut combined: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let mut fatal = false;
        for file in files {
            let mut names = vec![None; registration.length];
            let mut defined = BTreeSet::new();
            for line in enabled_lines(
                &file.relative_path,
                &file.content,
                options,
                &mut diagnostics,
            ) {
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
                let Ok(index) = usize::try_from(index) else {
                    diagnostics.push(out_of_range(&line));
                    continue;
                };
                if index >= names.len() {
                    diagnostics.push(out_of_range(&line));
                    continue;
                }
                if !defined.insert(index) {
                    diagnostics.push(at_line(
                        CsvDiagnosticCode::DuplicateIndex,
                        CsvDiagnosticSeverity::Warning,
                        1,
                        &line,
                        format!("user index {index} is defined more than once"),
                    ));
                }
                names[index] = Some(tokens[1].to_owned());
            }
            for (index, name) in names.into_iter().enumerate() {
                let Some(name) = name.filter(|name| !name.is_empty()) else {
                    continue;
                };
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
                    fatal = true;
                    break;
                }
                combined.insert(name, (index, file.relative_path.clone()));
            }
            if fatal {
                break;
            }
        }
        if !fatal {
            project.static_data.deferred_indices.resolved.insert(
                result_name.clone(),
                ResolvedUserIndex {
                    variable_name: result_name,
                    entries: combined
                        .into_iter()
                        .map(|(name, (index, _))| (name, index))
                        .collect(),
                },
            );
        }
    }
    diagnostics
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
