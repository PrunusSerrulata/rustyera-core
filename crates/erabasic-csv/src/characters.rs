use std::collections::{BTreeMap, BTreeSet};

use erabasic_data::{CharacterTemplate, NameTable, NameTableKind, ProjectSchema};

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions,
    input::{FileIndex, FileRoot, ascii_fold, basename},
    reader::enabled_lines,
    tables::at_line,
};

pub(crate) fn load_characters(
    files: &FileIndex,
    schema: &ProjectSchema,
    tables: &BTreeMap<NameTableKind, NameTable>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> (Vec<CharacterTemplate>, BTreeMap<String, i64>) {
    let mut paths: Vec<_> = files
        .all()
        .filter(|file| {
            file.root == FileRoot::Csv
                && (options.search_subdirectories || !file.path.contains('/'))
                && is_character_filename(basename(&file.path))
        })
        .collect();
    if options.sort_with_filename {
        paths.sort_by_key(|file| ascii_fold(&file.path));
    } else {
        paths.sort_by_key(|file| file.input_order);
    }

    let mut characters = Vec::new();
    for file in paths {
        load_character_file(
            &file.path,
            &file.content,
            schema,
            tables,
            options,
            diagnostics,
            &mut characters,
        );
        characters.sort_by_key(|character| character.no);
    }
    if options.compatible_call_name {
        for character in &mut characters {
            if character.call_name.is_empty() {
                character.call_name.clone_from(&character.name);
            }
        }
    }
    for character in &mut characters {
        character.is_sp_character = character.cflag.get(&0).is_some_and(|value| *value != 0);
    }
    diagnose_duplicate_characters(&characters, options, diagnostics);

    let mut relation_lookup = BTreeMap::new();
    for character in &characters {
        for name in [
            &character.name,
            &character.call_name,
            &character.nick_name,
            &character.master_name,
        ] {
            if !name.is_empty() {
                relation_lookup.entry(name.clone()).or_insert(character.no);
            }
        }
    }
    (characters, relation_lookup)
}

#[allow(clippy::case_sensitive_file_extension_comparisons)]
fn is_character_filename(filename: &str) -> bool {
    let folded = ascii_fold(filename);
    folded.starts_with("CHARA") && folded.ends_with(".CSV")
}

#[allow(clippy::too_many_arguments)]
fn load_character_file(
    path: &str,
    content: &str,
    schema: &ProjectSchema,
    tables: &BTreeMap<NameTableKind, NameTable>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
    output: &mut Vec<CharacterTemplate>,
) {
    let mut template: Option<CharacterTemplate> = None;
    for line in enabled_lines(path, content, options, diagnostics) {
        let tokens: Vec<_> = line.text.split(',').collect();
        if tokens.len() < 2 {
            diagnostics.push(at_line(
                CsvDiagnosticCode::MissingComma,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "character row requires at least two values",
            ));
            continue;
        }
        if tokens[0].is_empty() {
            diagnostics.push(at_line(
                CsvDiagnosticCode::StartedWithComma,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "character row starts with a comma",
            ));
            continue;
        }
        if equal_keyword(tokens[0], "NO", options.ignore_case) || tokens[0] == "番号" {
            if template.is_some() {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::DuplicateCharacterNumberField,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    "a character file may contain only one NO field",
                ));
                continue;
            }
            let Ok(no) = tokens[1].trim_end().trim_start().parse::<i64>() else {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::InvalidInteger,
                    CsvDiagnosticSeverity::Warning,
                    1,
                    &line,
                    format!("character number {:?} is not an integer", tokens[1]),
                ));
                continue;
            };
            let value = CharacterTemplate {
                no,
                csv_no: character_csv_number(path),
                ..CharacterTemplate::default()
            };
            output.push(value.clone());
            template = Some(value);
            continue;
        }
        let Some(current) = template.as_mut() else {
            diagnostics.push(at_line(
                CsvDiagnosticCode::CharacterDataBeforeNumber,
                CsvDiagnosticSeverity::Warning,
                1,
                &line,
                "character data appears before NO",
            ));
            continue;
        };
        if !apply_character_field(current, &tokens, schema, tables, &line, diagnostics) {
            break;
        }
        if let Some(last) = output.iter_mut().rev().find(|item| item.no == current.no) {
            last.clone_from(current);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn apply_character_field(
    character: &mut CharacterTemplate,
    tokens: &[&str],
    schema: &ProjectSchema,
    tables: &BTreeMap<NameTableKind, NameTable>,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> bool {
    let field = tokens[0].to_uppercase();
    match field.as_str() {
        "NAME" | "名前" => tokens[1].clone_into(&mut character.name),
        "CALLNAME" | "呼び名" => tokens[1].clone_into(&mut character.call_name),
        "NICKNAME" | "あだ名" => tokens[1].clone_into(&mut character.nick_name),
        "MASTERNAME" | "主人の呼び方" => tokens[1].clone_into(&mut character.master_name),
        "ISASSI" | "助手" => {}
        "MARK" | "刻印" => assign_integer(
            "MARK",
            NameTableKind::Mark,
            &mut character.mark,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "EXP" | "経験" => assign_integer(
            "EXP",
            NameTableKind::Exp,
            &mut character.exp,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "ABL" | "能力" => assign_integer(
            "ABL",
            NameTableKind::Abl,
            &mut character.abl,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "BASE" | "基礎" => assign_integer(
            "MAXBASE",
            NameTableKind::Base,
            &mut character.max_base,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "TALENT" | "素質" => assign_integer(
            "TALENT",
            NameTableKind::Talent,
            &mut character.talent,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "RELATION" | "相性" => assign_integer_without_names(
            "RELATION",
            &mut character.relation,
            tokens,
            schema,
            line,
            diagnostics,
        ),
        "CFLAG" | "フラグ" => assign_integer(
            "CFLAG",
            NameTableKind::Cflag,
            &mut character.cflag,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "EQUIP" | "装着物" => assign_integer(
            "EQUIP",
            NameTableKind::Equip,
            &mut character.equip,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "JUEL" | "珠" => assign_integer(
            "JUEL",
            NameTableKind::Palam,
            &mut character.juel,
            tokens,
            schema,
            tables,
            line,
            diagnostics,
        ),
        "CSTR" => {
            if tokens.len() < 3 {
                diagnostics.push(at_line(
                    CsvDiagnosticCode::MissingCharacterValue,
                    CsvDiagnosticSeverity::Error,
                    3,
                    line,
                    "CSTR row is missing its third value; the reference abandons this file",
                ));
                return false;
            }
            assign_string(
                &mut character.cstr,
                tokens,
                schema,
                tables,
                line,
                diagnostics,
            );
        }
        _ => diagnostics.push(at_line(
            CsvDiagnosticCode::UnknownCharacterField,
            CsvDiagnosticSeverity::Warning,
            1,
            line,
            format!("unknown character field {:?}", tokens[0]),
        )),
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn assign_integer(
    variable: &str,
    kind: NameTableKind,
    target: &mut BTreeMap<usize, i64>,
    tokens: &[&str],
    schema: &ProjectSchema,
    tables: &BTreeMap<NameTableKind, NameTable>,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let table = tables.get(&kind);
    let index = resolve_character_index(variable, tokens[1], schema, table, line, diagnostics);
    let Some(index) = index else { return };
    let value = tokens
        .get(2)
        .and_then(|value| parse_era_integer(value))
        .unwrap_or(1);
    if target.insert(index, value).is_some() {
        duplicate_field(variable, index, line, diagnostics);
    }
}

fn assign_integer_without_names(
    variable: &str,
    target: &mut BTreeMap<usize, i64>,
    tokens: &[&str],
    schema: &ProjectSchema,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let index = resolve_character_index(variable, tokens[1], schema, None, line, diagnostics);
    let Some(index) = index else { return };
    let value = tokens
        .get(2)
        .and_then(|value| parse_era_integer(value))
        .unwrap_or(1);
    if target.insert(index, value).is_some() {
        duplicate_field(variable, index, line, diagnostics);
    }
}

fn assign_string(
    target: &mut BTreeMap<usize, String>,
    tokens: &[&str],
    schema: &ProjectSchema,
    tables: &BTreeMap<NameTableKind, NameTable>,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let index = resolve_character_index(
        "CSTR",
        tokens[1],
        schema,
        tables.get(&NameTableKind::Cstr),
        line,
        diagnostics,
    );
    let Some(index) = index else { return };
    if target.insert(index, tokens[2].to_owned()).is_some() {
        duplicate_field("CSTR", index, line, diagnostics);
    }
}

fn resolve_character_index(
    variable: &str,
    text: &str,
    schema: &ProjectSchema,
    table: Option<&NameTable>,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> Option<usize> {
    let length = schema
        .variable(variable)
        .and_then(|variable| variable.dimensions.first())
        .copied()
        .unwrap_or(0);
    if length == 0 {
        diagnostics.push(at_line(
            CsvDiagnosticCode::ProhibitedVariable,
            CsvDiagnosticSeverity::Error,
            2,
            line,
            format!("{variable} is disabled"),
        ));
        return None;
    }
    let numeric = parse_era_integer(text.trim_end());
    let index = if let Some(value) = numeric {
        value
    } else if let Some(table) = table {
        let Some(index) = table.lookup.get(text) else {
            diagnostics.push(at_line(
                CsvDiagnosticCode::UndefinedName,
                CsvDiagnosticSeverity::Warning,
                1,
                line,
                format!("{text:?} is not defined in the corresponding name table"),
            ));
            return None;
        };
        i64::from(*index)
    } else {
        -1
    };
    let Ok(index) = usize::try_from(index) else {
        diagnostics.push(index_out_of_range(text, line));
        return None;
    };
    if index >= length {
        diagnostics.push(index_out_of_range(text, line));
        return None;
    }
    Some(index)
}

fn parse_era_integer(value: &str) -> Option<i64> {
    let mut value = value;
    let mut sign = 1_i64;
    if let Some(rest) = value.strip_prefix('+') {
        value = rest;
    } else if let Some(rest) = value.strip_prefix('-') {
        value = rest;
        sign = -1;
    }
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        let digits: String = hex.chars().take_while(char::is_ascii_hexdigit).collect();
        return (!digits.is_empty())
            .then(|| {
                i64::from_str_radix(&digits, 16)
                    .ok()
                    .map(|number| number * sign)
            })
            .flatten();
    }
    if let Some(binary) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        let digits: String = binary
            .chars()
            .take_while(|character| matches!(character, '0' | '1'))
            .collect();
        return (!digits.is_empty())
            .then(|| {
                i64::from_str_radix(&digits, 2)
                    .ok()
                    .map(|number| number * sign)
            })
            .flatten();
    }
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty())
        .then(|| digits.parse::<i64>().ok().map(|number| number * sign))
        .flatten()
}

fn character_csv_number(path: &str) -> i64 {
    let upper = ascii_fold(path);
    let Some(start) = upper.find("CHARA") else {
        return 0;
    };
    let suffix = &path[start + "CHARA".len()..];
    let digits: String = suffix.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

fn diagnose_duplicate_characters(
    characters: &[CharacterTemplate],
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    let mut normal = BTreeSet::new();
    let mut special = BTreeSet::new();
    for character in characters {
        let target = if options.compatible_sp_character && character.is_sp_character {
            &mut special
        } else {
            &mut normal
        };
        if !target.insert(character.no) {
            diagnostics.push(CsvDiagnostic::new(
                CsvDiagnosticCode::DuplicateCharacter,
                CsvDiagnosticSeverity::Warning,
                1,
                "",
                None,
                format!(
                    "character number {} is defined more than once",
                    character.no
                ),
            ));
        }
    }
}

fn equal_keyword(left: &str, right: &str, ignore_case: bool) -> bool {
    if ignore_case {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn duplicate_field(
    variable: &str,
    index: usize,
    line: &crate::reader::EnabledLine,
    diagnostics: &mut Vec<CsvDiagnostic>,
) {
    diagnostics.push(at_line(
        CsvDiagnosticCode::DuplicateCharacterField,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("{variable}:{index} is defined more than once; the last value wins"),
    ));
}

fn index_out_of_range(text: &str, line: &crate::reader::EnabledLine) -> CsvDiagnostic {
    at_line(
        CsvDiagnosticCode::IndexOutOfRange,
        CsvDiagnosticSeverity::Warning,
        1,
        line,
        format!("character array index {text:?} is out of range"),
    )
}
