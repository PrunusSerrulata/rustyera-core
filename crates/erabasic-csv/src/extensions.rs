use erabasic_data::ExtensionData;

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::{FileIndex, FileRoot, ascii_fold, basename},
    reader::enabled_lines,
    tables::at_line,
};

#[allow(clippy::case_sensitive_file_extension_comparisons)]
pub(crate) fn load_extensions(
    files: &FileIndex,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> ExtensionData {
    let mut result = ExtensionData::default();
    let mut candidates: Vec<_> = files
        .all()
        .filter(|file| {
            let name = ascii_fold(basename(&file.path));
            file.root == FileRoot::Csv && name.starts_with("VAREXT") && name.ends_with(".CSV")
        })
        .collect();
    candidates.sort_by_key(|file| file.input_order);
    for file in candidates {
        for line in enabled_lines(&file.source_path, &file.content, options, diagnostics) {
            let mut tokens = line.text.split(',');
            let category = tokens.next().unwrap_or_default();
            let Some(first_name) = tokens.next() else {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::MissingComma,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    "VarExt row requires a category and at least one name",
                ));
                continue;
            };
            if category.is_empty() {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::StartedWithComma,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    "VarExt row starts with a comma",
                ));
                continue;
            }
            let category = if options.ignore_case {
                category.to_ascii_uppercase()
            } else {
                category.to_owned()
            };
            let target = match category.as_str() {
                "GLOBAL_MAPS" => Some(&mut result.global_maps),
                "SAVE_MAPS" => Some(&mut result.save_maps),
                "STATIC_MAPS" => Some(&mut result.static_maps),
                "GLOBAL_XMLS" => Some(&mut result.global_xmls),
                "SAVE_XMLS" => Some(&mut result.save_xmls),
                "STATIC_XMLS" => Some(&mut result.static_xmls),
                "GLOBAL_DTS" => Some(&mut result.global_data_tables),
                "SAVE_DTS" => Some(&mut result.save_data_tables),
                "STATIC_DTS" => Some(&mut result.static_data_tables),
                _ => None,
            };
            if let Some(target) = target {
                target.extend(
                    std::iter::once(first_name)
                        .chain(tokens)
                        .map(|value| value.trim().to_owned()),
                );
            }
        }
    }
    result
}
