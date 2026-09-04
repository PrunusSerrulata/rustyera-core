use std::collections::BTreeMap;

use erabasic_data::ReplaceSettings;

use crate::{
    CsvDiagnostic, CsvDiagnosticCode, CsvDiagnosticSeverity, CsvLoadOptions, input::IndexedFile,
};

pub(crate) fn load_replace(
    file: Option<&IndexedFile>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> ReplaceSettings {
    let mut settings = ReplaceSettings::default();
    if !options.use_replace_file {
        return settings;
    }
    let Some(file) = file else {
        return settings;
    };
    for (line_no, raw) in file.content.lines().enumerate() {
        let decoded = if line_no == 0 {
            raw.strip_prefix('\u{feff}').unwrap_or(raw)
        } else {
            raw
        };
        let line = decoded.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some(delimiter) = line.find([',', ':']) else {
            continue;
        };
        let key = line[..delimiter].trim().to_ascii_uppercase();
        let value = line[delimiter + 1..].trim();
        if value.is_empty() {
            continue;
        }
        let result = apply_replace(&mut settings, &key, value);
        if let Err((code, message)) = result {
            diagnostics.push(CsvDiagnostic::new(
                code,
                CsvDiagnosticSeverity::Warning,
                1,
                &file.source_path,
                Some(crate::input::source(
                    &file.source_path,
                    u32::try_from(line_no).unwrap_or(u32::MAX),
                    0,
                    raw.len(),
                )),
                message,
            ));
        }
    }
    if settings.draw_line_string.is_empty() {
        settings.draw_line_string = "-".into();
    }
    settings
}

#[allow(clippy::too_many_lines)]
fn apply_replace(
    settings: &mut ReplaceSettings,
    key: &str,
    value: &str,
) -> Result<(), (CsvDiagnosticCode, String)> {
    macro_rules! parse {
        ($type:ty) => {
            value.parse::<$type>().map_err(|_| {
                (
                    CsvDiagnosticCode::InvalidInteger,
                    format!("invalid value for {key}"),
                )
            })?
        };
    }
    match key {
        "MONEYLABEL" | "お金の単位" => settings.money_label = value.into(),
        "MONEYFIRST" | "単位の位置" => {
            settings.money_first = parse_bool(value).ok_or_else(|| {
                (
                    CsvDiagnosticCode::InvalidBoolean,
                    format!("invalid boolean value for {key}"),
                )
            })?;
        }
        "LOADLABEL" | "起動時簡略表示" => settings.load_label = value.into(),
        "MAXSHOPITEM" | "販売アイテム数" => settings.max_shop_item = parse!(i32),
        "DRAWLINESTRING" | "DRAWLINE文字" => settings.draw_line_string = value.into(),
        "BARCHAR1" | "BAR文字1" => settings.bar_char_1 = parse_char(value, key)?,
        "BARCHAR2" | "BAR文字2" => settings.bar_char_2 = parse_char(value, key)?,
        "TITLEMENUSTRING0" | "システムメニュー0" => {
            settings.title_menu_string_0 = value.into();
        }
        "TITLEMENUSTRING1" | "システムメニュー1" => {
            settings.title_menu_string_1 = value.into();
        }
        "COMABLEDEFAULT" | "COM_ABLE初期値" => settings.com_able_default = parse!(i32),
        "STAINDEFAULT" | "汚れの初期値" => settings.stain_default = parse_list(value, key)?,
        "TIMEUPLABEL" | "時間切れ表示" => settings.timeup_label = value.into(),
        "EXPLVDEF" | "EXPLVの初期値" => settings.exp_lv_default = parse_list(value, key)?,
        "PALAMLVDEF" | "PALAMLVの初期値" => {
            settings.palam_lv_default = parse_list(value, key)?;
        }
        "PBANDDEF" | "PBANDの初期値" => settings.pband_default = parse!(i64),
        "RELATIONDEF" | "RELATIONの初期値" => settings.relation_default = parse!(i64),
        _ => {}
    }
    Ok(())
}

fn parse_bool(value: &str) -> Option<bool> {
    if let Ok(number) = value.parse::<i32>() {
        return Some(number != 0);
    }
    if value.eq_ignore_ascii_case("NO") || value.eq_ignore_ascii_case("FALSE") || value == "後" {
        Some(false)
    } else if value.eq_ignore_ascii_case("YES")
        || value.eq_ignore_ascii_case("TRUE")
        || value == "前"
    {
        Some(true)
    } else {
        None
    }
}

fn parse_char(value: &str, key: &str) -> Result<char, (CsvDiagnosticCode, String)> {
    let mut characters = value.chars();
    let Some(character) = characters.next() else {
        return Err((CsvDiagnosticCode::InvalidCharacter, format!("empty {key}")));
    };
    if characters.next().is_some() || value.encode_utf16().count() != 1 {
        return Err((
            CsvDiagnosticCode::InvalidCharacter,
            format!("{key} must contain exactly one Unicode scalar value"),
        ));
    }
    Ok(character)
}

fn parse_list(value: &str, key: &str) -> Result<Vec<i64>, (CsvDiagnosticCode, String)> {
    value
        .split('/')
        .map(|part| {
            part.trim().parse().map_err(|_| {
                (
                    CsvDiagnosticCode::InvalidList,
                    format!("{key} contains a non-integer item"),
                )
            })
        })
        .collect()
}

pub(crate) fn load_rename(
    file: Option<&IndexedFile>,
    options: &CsvLoadOptions,
    diagnostics: &mut Vec<CsvDiagnostic>,
) -> BTreeMap<String, String> {
    let mut rename = BTreeMap::new();
    if !options.use_rename_file {
        return rename;
    }
    let Some(file) = file else {
        diagnostics.push(CsvDiagnostic::new(
            CsvDiagnosticCode::MissingRenameFile,
            CsvDiagnosticSeverity::Error,
            1,
            "_Rename.csv",
            None,
            "_Rename.csv is enabled but was not submitted",
        ));
        return rename;
    };
    for (line_no, raw) in file.content.lines().enumerate() {
        let line = if line_no == 0 {
            raw.strip_prefix('\u{feff}').unwrap_or(raw)
        } else {
            raw
        };
        if line.starts_with(';') {
            continue;
        }
        if let Some((replacement, name)) = split_once_unescaped_comma(line) {
            rename.insert(
                format!("[[{}]]", name.trim()),
                replacement.trim().to_owned(),
            );
        }
    }
    rename
}

fn split_once_unescaped_comma(value: &str) -> Option<(&str, &str)> {
    let mut delimiter = None;
    let mut previous_backslash = false;
    for (offset, character) in value.char_indices() {
        if character == ',' && !previous_backslash {
            if delimiter.is_some() {
                return None;
            }
            delimiter = Some(offset);
        }
        previous_backslash = character == '\\';
    }
    delimiter.map(|offset| (&value[..offset], &value[offset + 1..]))
}
