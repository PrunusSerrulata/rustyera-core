use std::collections::BTreeSet;

use erabasic_data::{NameTableKind, ProjectSchema, StorageScope};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions, input::IndexedFile,
    reader::enabled_lines, tables::at_line,
};

#[allow(
    clippy::comparison_chain,
    clippy::needless_range_loop,
    clippy::single_match_else,
    clippy::too_many_lines
)]
pub(crate) fn load_variable_sizes(
    file: Option<&IndexedFile>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> (ProjectSchema, bool) {
    let mut schema = ProjectSchema::builtin_defaults();
    let Some(file) = file else {
        return (schema, false);
    };
    let mut changed = BTreeSet::new();
    for line in enabled_lines(&file.path, &file.content, options, diagnostics) {
        let tokens: Vec<_> = line.text.split(',').collect();
        if tokens.len() < 2 {
            diagnostics.push(at_line(
                CsvDiagnosticCode::MissingComma,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "VariableSize row requires a variable and at least one length",
            ));
            continue;
        }
        let name = tokens[0].trim();
        let Some(variable) = schema.variables.get(name) else {
            diagnostics.push(at_line(
                CsvDiagnosticCode::UnknownVariable,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("{name:?} is not a fixed Emuera variable"),
            ));
            continue;
        };
        if variable.dimensions.is_empty()
            || variable.storage == StorageScope::Calculated
            || name == "RANDDATA"
        {
            diagnostics.push(at_line(
                CsvDiagnosticCode::VariableNotResizable,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("{name} cannot be resized"),
            ));
            continue;
        }
        let rank = variable.dimensions.len();
        let can_forbid = variable.can_forbid;
        let is_local = variable.storage == StorageScope::Local;
        let Ok(first) = tokens[1].trim().parse::<i64>() else {
            diagnostics.push(invalid_length(&line, 1));
            continue;
        };
        let dimensions = if first < 0 {
            if !can_forbid {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::InvalidArraySize,
                    CsvDiagnosticSeverity::Error,
                    2,
                    &line,
                    format!("{name} cannot be disabled"),
                ));
                continue;
            }
            vec![0; rank]
        } else if first == 0 {
            diagnostics.push(at_line(
                CsvDiagnosticCode::InvalidArraySize,
                CsvDiagnosticSeverity::Error,
                2,
                &line,
                "an array length of zero is invalid; use a negative value to disable it",
            ));
            continue;
        } else {
            let mut values = vec![first];
            if tokens.len() < rank + 1 {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::InvalidArraySize,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    format!("a rank-{rank} array requires {rank} lengths"),
                ));
                continue;
            }
            let mut valid = true;
            for dimension in 2..=rank {
                match tokens[dimension].trim().parse::<i64>() {
                    Ok(value) => values.push(value),
                    Err(_) => {
                        diagnostics.push(invalid_length(&line, dimension));
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            if !validate_dimensions(&values, rank, is_local, &line, diagnostics) {
                continue;
            }
            values
                .into_iter()
                .map(|value| usize::try_from(value).expect("validated positive size"))
                .collect()
        };

        apply_size(&mut schema, name, &dimensions);
        if !changed.insert(name.to_owned()) {
            diagnostics.push(at_line(
                CsvDiagnosticCode::DuplicateVariableSize,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("{name} is resized more than once; the last value wins"),
            ));
        }
    }

    let fatal = !reconcile(&mut schema, &changed, &file.path, diagnostics);
    (schema, fatal)
}

fn validate_dimensions(
    dimensions: &[i64],
    rank: usize,
    is_local: bool,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> bool {
    if dimensions.iter().any(|length| *length < 1) {
        diagnostics.push(at_line(
            CsvDiagnosticCode::InvalidArraySize,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            "array dimensions must be positive",
        ));
        return false;
    }
    if rank == 1 && !is_local && dimensions[0] < 100 {
        diagnostics.push(at_line(
            CsvDiagnosticCode::InvalidArraySize,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            "a built-in one-dimensional array cannot be smaller than 100",
        ));
        return false;
    }
    if dimensions.iter().any(|length| *length > 1_000_000) {
        diagnostics.push(at_line(
            CsvDiagnosticCode::ArraySizeTooLarge,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            "a dimension cannot exceed 1,000,000",
        ));
        return false;
    }
    let product = dimensions
        .iter()
        .try_fold(1_i64, |product, value| product.checked_mul(*value));
    let limit = if rank == 3 { 10_000_000 } else { 1_000_000 };
    if product.is_none_or(|product| product > limit) {
        diagnostics.push(at_line(
            CsvDiagnosticCode::ArraySizeTooLarge,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            format!("rank-{rank} array exceeds the {limit} element limit"),
        ));
        return false;
    }
    true
}

fn apply_size(schema: &mut ProjectSchema, name: &str, dimensions: &[usize]) {
    match name {
        "ITEMNAME" | "ITEMPRICE" => {
            set_variable_size(schema, "ITEMPRICE", dimensions);
            set_name_size(schema, NameTableKind::Item, dimensions[0]);
        }
        "STR" => {
            set_variable_size(schema, "STR", dimensions);
            set_name_size(schema, NameTableKind::Str, dimensions[0]);
        }
        _ => {
            if let Some(kind) = kind_for_variable(name) {
                set_name_size(schema, kind, dimensions[0]);
            } else {
                set_variable_size(schema, name, dimensions);
            }
        }
    }
}

fn set_variable_size(schema: &mut ProjectSchema, name: &str, dimensions: &[usize]) {
    if let Some(variable) = schema.variables.get_mut(name) {
        variable.dimensions.clone_from(&dimensions.to_vec());
    }
}

fn set_name_size(schema: &mut ProjectSchema, kind: NameTableKind, length: usize) {
    if let Some(space) = schema.index_spaces.get_mut(&kind) {
        space.length = length;
    }
    if let Some(variable) = schema.variables.get_mut(kind.variable_name()) {
        variable.dimensions = vec![length];
    }
}

#[allow(clippy::too_many_lines)]
fn reconcile(
    schema: &mut ProjectSchema,
    changed: &BTreeSet<String>,
    path: &str,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> bool {
    const PAIRS: [(&str, NameTableKind); 21] = [
        ("ABL", NameTableKind::Abl),
        ("TALENT", NameTableKind::Talent),
        ("EXP", NameTableKind::Exp),
        ("MARK", NameTableKind::Mark),
        ("BASE", NameTableKind::Base),
        ("SOURCE", NameTableKind::Source),
        ("EX", NameTableKind::Ex),
        ("EQUIP", NameTableKind::Equip),
        ("TEQUIP", NameTableKind::Tequip),
        ("FLAG", NameTableKind::Flag),
        ("TFLAG", NameTableKind::Tflag),
        ("CFLAG", NameTableKind::Cflag),
        ("TCVAR", NameTableKind::Tcvar),
        ("CSTR", NameTableKind::Cstr),
        ("STAIN", NameTableKind::Stain),
        ("STR", NameTableKind::Strname),
        ("TSTR", NameTableKind::Tstr),
        ("SAVESTR", NameTableKind::Savestr),
        ("GLOBAL", NameTableKind::Global),
        ("GLOBALS", NameTableKind::Globals),
        ("DAY", NameTableKind::Day),
    ];
    for (main, kind) in PAIRS.into_iter().chain([
        ("TIME", NameTableKind::Time),
        ("MONEY", NameTableKind::Money),
    ]) {
        reconcile_pair(schema, changed, path, diagnostics, main, kind);
    }

    let palam_changed = changed.contains("PALAM");
    let juel_changed = changed.contains("JUEL");
    let name_changed = changed.contains("PALAMNAME");
    if palam_changed || juel_changed {
        let palam = first_dimension(schema, "PALAM");
        let juel = first_dimension(schema, "JUEL");
        let main_max = palam.max(juel);
        if name_changed {
            let name_length = name_length(schema, NameTableKind::Palam);
            if name_length != main_max {
                let merged = name_length.max(main_max);
                if palam == main_max {
                    set_variable_size(schema, "PALAM", &[merged]);
                }
                if juel == main_max {
                    set_variable_size(schema, "JUEL", &[merged]);
                }
                set_name_size(schema, NameTableKind::Palam, merged);
                reconciled_warning(
                    path,
                    diagnostics,
                    "PALAM, JUEL and PALAMNAME were reconciled",
                );
            }
        } else {
            set_name_size(schema, NameTableKind::Palam, main_max);
        }
    } else if name_changed {
        let name_length = name_length(schema, NameTableKind::Palam);
        set_variable_size(schema, "PALAM", &[name_length]);
        let juel = first_dimension(schema, "JUEL");
        if name_length < juel {
            set_name_size(schema, NameTableKind::Palam, juel);
            reconciled_warning(path, diagnostics, "PALAMNAME was enlarged to the JUEL size");
        }
    }

    let cdf_names_changed = changed.contains("CDFLAGNAME1") || changed.contains("CDFLAGNAME2");
    let cdf_changed = changed.contains("CDFLAG");
    let shape = schema
        .variable("CDFLAG")
        .map_or_else(|| vec![1, 1], |variable| variable.dimensions.clone());
    let names = [
        name_length(schema, NameTableKind::Cdflag1),
        name_length(schema, NameTableKind::Cdflag2),
    ];
    if cdf_changed && cdf_names_changed && shape.as_slice() != names {
        diagnostics.push(global_diagnostic(
            CsvDiagnosticCode::CdflagShapeMismatch,
            CsvDiagnosticSeverity::Fatal,
            3,
            path,
            "CDFLAG dimensions do not match CDFLAGNAME1/2",
        ));
        return false;
    }
    if cdf_names_changed && !cdf_changed {
        if names[0].saturating_mul(names[1]) > 1_000_000 {
            diagnostics.push(global_diagnostic(
                CsvDiagnosticCode::CdflagShapeMismatch,
                CsvDiagnosticSeverity::Fatal,
                3,
                path,
                "CDFLAG name dimensions exceed 1,000,000 elements",
            ));
            return false;
        }
        set_variable_size(schema, "CDFLAG", &names);
    } else if cdf_changed && !cdf_names_changed {
        set_name_size(schema, NameTableKind::Cdflag1, shape[0]);
        set_name_size(schema, NameTableKind::Cdflag2, shape[1]);
    }
    true
}

fn reconcile_pair(
    schema: &mut ProjectSchema,
    changed: &BTreeSet<String>,
    path: &str,
    diagnostics: &mut Vec<CsvDiagnostic>,
    main: &str,
    kind: NameTableKind,
) {
    let name = kind.variable_name();
    match (changed.contains(main), changed.contains(name)) {
        (true, true) => {
            let main_length = first_dimension(schema, main);
            let table_length = name_length(schema, kind);
            if main_length != table_length {
                let merged = main_length.max(table_length);
                set_variable_size(schema, main, &[merged]);
                set_name_size(schema, kind, merged);
                reconciled_warning(
                    path,
                    diagnostics,
                    &format!("{main} and {name} were reconciled to {merged}"),
                );
            }
        }
        (false, true) => {
            let length = name_length(schema, kind);
            set_variable_size(schema, main, &[length]);
        }
        (true, false) => {
            let length = first_dimension(schema, main);
            set_name_size(schema, kind, length);
        }
        (false, false) => {}
    }
}

fn first_dimension(schema: &ProjectSchema, name: &str) -> usize {
    schema
        .variable(name)
        .and_then(|variable| variable.dimensions.first())
        .copied()
        .unwrap_or(0)
}

fn name_length(schema: &ProjectSchema, kind: NameTableKind) -> usize {
    schema
        .index_spaces
        .get(&kind)
        .map_or(0, |space| space.length)
}

fn kind_for_variable(name: &str) -> Option<NameTableKind> {
    NameTableKind::ALL
        .into_iter()
        .find(|kind| kind.variable_name() == name)
}

fn invalid_length(line: &crate::reader::EnabledLine, field: usize) -> CsvDiagnostic {
    at_line(
        CsvDiagnosticCode::InvalidInteger,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("length field {} is not an integer", field + 1),
    )
}

fn reconciled_warning(path: &str, diagnostics: &mut Vec<CsvDiagnostic>, message: &str) {
    diagnostics.push(global_diagnostic(
        CsvDiagnosticCode::ReconciledVariableSize,
        CsvDiagnosticSeverity::Warning,
        1,
        path,
        message,
    ));
}

fn global_diagnostic(
    code: CsvDiagnosticCode,
    severity: CsvDiagnosticSeverity,
    reference_level: u8,
    path: &str,
    message: impl Into<String>,
) -> CsvDiagnostic {
    CsvDiagnostic::new(code, severity, reference_level, path, None, message)
}
