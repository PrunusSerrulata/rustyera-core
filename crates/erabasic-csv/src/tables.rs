use std::collections::{BTreeMap, BTreeSet};

use erabasic_data::{NameAlias, NameTable, NameTableKind, ProjectSchema};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::FileIndex,
    reader::{EnabledLine, enabled_lines},
};

pub(crate) const TABLE_FILES: [(&str, NameTableKind); 29] = [
    ("ABL.CSV", NameTableKind::Abl),
    ("EXP.CSV", NameTableKind::Exp),
    ("TALENT.CSV", NameTableKind::Talent),
    ("PALAM.CSV", NameTableKind::Palam),
    ("TRAIN.CSV", NameTableKind::Train),
    ("MARK.CSV", NameTableKind::Mark),
    ("ITEM.CSV", NameTableKind::Item),
    ("BASE.CSV", NameTableKind::Base),
    ("SOURCE.CSV", NameTableKind::Source),
    ("EX.CSV", NameTableKind::Ex),
    ("STR.CSV", NameTableKind::Str),
    ("EQUIP.CSV", NameTableKind::Equip),
    ("TEQUIP.CSV", NameTableKind::Tequip),
    ("FLAG.CSV", NameTableKind::Flag),
    ("TFLAG.CSV", NameTableKind::Tflag),
    ("CFLAG.CSV", NameTableKind::Cflag),
    ("TCVAR.CSV", NameTableKind::Tcvar),
    ("CSTR.CSV", NameTableKind::Cstr),
    ("STAIN.CSV", NameTableKind::Stain),
    ("CDFLAG1.CSV", NameTableKind::Cdflag1),
    ("CDFLAG2.CSV", NameTableKind::Cdflag2),
    ("STRNAME.CSV", NameTableKind::Strname),
    ("TSTR.CSV", NameTableKind::Tstr),
    ("SAVESTR.CSV", NameTableKind::Savestr),
    ("GLOBAL.CSV", NameTableKind::Global),
    ("GLOBALS.CSV", NameTableKind::Globals),
    ("DAY.CSV", NameTableKind::Day),
    ("TIME.CSV", NameTableKind::Time),
    ("MONEY.CSV", NameTableKind::Money),
];

pub(crate) fn load_name_tables(
    files: &FileIndex,
    schema: &ProjectSchema,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> (BTreeMap<NameTableKind, NameTable>, Vec<i64>) {
    let mut tables = BTreeMap::new();
    for kind in NameTableKind::ALL {
        let length = schema
            .index_spaces
            .get(&kind)
            .map_or(0, |space| space.length);
        tables.insert(kind, NameTable::empty(length));
    }
    let item_length = schema
        .index_spaces
        .get(&NameTableKind::Item)
        .map_or(0, |space| space.length);
    let mut item_prices = vec![0; item_length];

    for (filename, kind) in TABLE_FILES {
        let Some(file) = files.csv_file(filename) else {
            continue;
        };
        let table = tables
            .get_mut(&kind)
            .expect("all name tables are allocated before loading");
        load_table(
            file.source_path.as_str(),
            &file.content,
            kind,
            table,
            &mut item_prices,
            options,
            diagnostics,
        );

        let stem = filename
            .strip_suffix(".CSV")
            .expect("table constants end in .CSV");
        let alias_name = format!("{stem}.als");
        if let Some(alias_file) = files.csv_file(&alias_name) {
            load_aliases(
                &alias_file.source_path,
                &alias_file.content,
                table,
                options,
                diagnostics,
            );
        }
    }
    // STR is deliberately excluded in the reference because its values are data rather
    // than symbolic names.
    for (kind, table) in &mut tables {
        if *kind != NameTableKind::Str {
            table.rebuild_lookup();
        }
    }
    (tables, item_prices)
}

fn load_table(
    path: &str,
    content: &str,
    kind: NameTableKind,
    table: &mut NameTable,
    item_prices: &mut [i64],
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let mut defined = BTreeSet::new();
    for line in enabled_lines(path, content, options, diagnostics) {
        let tokens: Vec<_> = line.text.split(',').collect();
        if tokens.len() < 2 {
            diagnostics.push(at_line(
                CsvDiagnosticCode::MissingComma,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "a name table row requires an index and value",
            ));
            continue;
        }
        let Ok(index) = tokens[0].trim().parse::<i32>() else {
            diagnostics.push(at_line(
                CsvDiagnosticCode::InvalidInteger,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "the first value is not an integer",
            ));
            continue;
        };
        if table.names.is_empty() {
            diagnostics.push(at_line(
                CsvDiagnosticCode::ProhibitedNameTable,
                CsvDiagnosticSeverity::Error,
                2,
                &line,
                "this name table is disabled",
            ));
            break;
        }
        let Ok(index_usize) = usize::try_from(index) else {
            diagnostics.push(out_of_range(&line, index));
            continue;
        };
        if index_usize >= table.names.len() {
            diagnostics.push(out_of_range(&line, index));
            continue;
        }
        if !defined.insert(index) {
            diagnostics.push(at_line(
                CsvDiagnosticCode::DuplicateIndex,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("index {index} is defined more than once; the last value wins"),
            ));
        }
        table.names[index_usize] = Some(tokens[1].to_owned());
        if kind == NameTableKind::Item && tokens.len() >= 3 {
            match tokens[2].trim().parse::<i64>() {
                Ok(price) => item_prices[index_usize] = price,
                Err(_) => diagnostics.push(at_line(
                    CsvDiagnosticCode::InvalidInteger,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    "item price is not an integer",
                )),
            }
        }
    }
}

fn load_aliases(
    path: &str,
    content: &str,
    table: &mut NameTable,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let mut defined_indices = BTreeSet::new();
    let mut alias_names = BTreeSet::new();
    let snake_rules = options.compatibility.uses_snake_alias_rules();
    for line in enabled_lines(path, content, options, diagnostics) {
        let Some((index, name)) = parse_alias_row(&line, diagnostics) else {
            continue;
        };
        if !snake_rules && !defined_indices.insert(index) {
            diagnostics.push(at_line(
                CsvDiagnosticCode::DuplicateIndex,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                format!("alias index {index} is defined more than once"),
            ));
        }
        let alias = if snake_rules { name.trim() } else { name }.to_owned();
        if !alias_names.insert(alias.clone()) {
            diagnostics.push(at_line(
                CsvDiagnosticCode::DuplicateAlias,
                if snake_rules {
                    CsvDiagnosticSeverity::Warning
                } else {
                    CsvDiagnosticSeverity::Error
                },
                if snake_rules { 1 } else { 3 },
                &line,
                format!("alias {alias:?} is defined more than once"),
            ));
            if snake_rules {
                continue;
            }
            // Original Dictionary.Add throws and abandons the rest of the alias file.
            break;
        }
        table.aliases.push(NameAlias { name: alias, index });
    }
}

pub(crate) fn parse_alias_row<'a>(
    line: &'a EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> Option<(i32, &'a str)> {
    let mut tokens = line.text.split(',');
    let index_text = tokens.next().unwrap_or_default();
    let Some(name) = tokens.next() else {
        diagnostics.push(at_line(
            CsvDiagnosticCode::MissingComma,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            "an alias row requires an index and alias",
        ));
        return None;
    };
    let Ok(index) = index_text.trim().parse::<i32>() else {
        diagnostics.push(at_line(
            CsvDiagnosticCode::InvalidInteger,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            "the first alias value is not an integer",
        ));
        return None;
    };
    Some((index, name))
}

pub(crate) fn at_line(
    code: CsvDiagnosticCode,
    severity: CsvDiagnosticSeverity,
    reference_level: u8,
    line: &crate::reader::EnabledLine,
    message: impl Into<String>,
) -> CsvDiagnostic {
    CsvDiagnostic::new(
        code,
        severity,
        reference_level,
        &line.source.relative_path,
        Some(line.source.clone()),
        message,
    )
}

fn out_of_range(line: &crate::reader::EnabledLine, index: i32) -> CsvDiagnostic {
    at_line(
        CsvDiagnosticCode::IndexOutOfRange,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("index {index} is outside the declared table length"),
    )
}
