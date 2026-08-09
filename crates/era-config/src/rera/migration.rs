use serde_json::Value as JsonValue;

use crate::{ConfigStore, ConfigValue, is_regular_code, is_replace_code, resolve_code};

use super::{ByteSpan, ReraConfigDocument, catalog::rera_catalog};

#[derive(Clone, Copy, Debug)]
pub struct LegacyConfigSource<'a> {
    pub relative_path: &'a str,
    pub contents: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyMigrationDiagnosticKind {
    MalformedLine,
    UnknownSetting,
    InvalidValue,
    InvalidJson,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyMigrationDiagnostic {
    pub kind: LegacyMigrationDiagnosticKind,
    pub relative_path: String,
    pub line: Option<usize>,
    pub span: Option<ByteSpan>,
    pub message: String,
}

impl std::fmt::Display for LegacyMigrationDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "{}:{line}: {}", self.relative_path, self.message),
            None => write!(formatter, "{}: {}", self.relative_path, self.message),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LegacyMigration {
    pub document: ReraConfigDocument,
    pub values: ConfigStore,
    pub diagnostics: Vec<LegacyMigrationDiagnostic>,
}

/// Convert every explicitly supported legacy setting source into `reraconfig.toml`.
///
/// # Panics
///
/// Panics only if the built-in catalog, normalization rules, and generated TOML schema disagree;
/// those are programmer invariants covered by round-trip tests.
#[must_use]
pub fn migrate_legacy_configuration(sources: &[LegacyConfigSource<'_>]) -> LegacyMigration {
    let mut values = ConfigStore::default();
    let mut diagnostics = Vec::new();

    if let Some(source) = preferred_source(sources, "_default.config", "default.config") {
        migrate_colon_config(
            source,
            LegacyColonKind::Regular,
            &mut values,
            &mut diagnostics,
        );
    }
    if let Some(source) = named_source(sources, "emuera.config") {
        migrate_colon_config(
            source,
            LegacyColonKind::Regular,
            &mut values,
            &mut diagnostics,
        );
    }
    if let Some(source) = named_source(sources, "setting.json") {
        migrate_setting_json(source, &mut values, &mut diagnostics);
    }
    if let Some(source) = preferred_source(sources, "_fixed.config", "fixed.config") {
        migrate_colon_config(
            source,
            LegacyColonKind::Fixed,
            &mut values,
            &mut diagnostics,
        );
    }
    if let Some(source) = named_source(sources, "debug.config") {
        migrate_colon_config(
            source,
            LegacyColonKind::Debug,
            &mut values,
            &mut diagnostics,
        );
    }
    if matches!(
        values.get_code("UseReplaceFile"),
        Some(ConfigValue::Boolean(true))
    ) && let Some(source) = named_source(sources, "_Replace.csv")
    {
        migrate_replace(source, &mut values, &mut diagnostics);
    }

    normalize_legacy_values(&mut values);
    let defaults = ConfigStore::default();
    let mut document = ReraConfigDocument::empty();
    for spec in rera_catalog() {
        let setting = values
            .get_code(spec.code)
            .expect("legacy store contains every catalog setting");
        if setting
            != defaults
                .get_code(spec.code)
                .expect("default catalog is complete")
            || values.is_fixed(spec.code)
        {
            document
                .set_code_unchecked(spec.code, setting)
                .expect("normalized legacy values satisfy reraconfig schema");
        }
    }
    document
        .set_locked_codes_unchecked(
            rera_catalog()
                .into_iter()
                .filter(|spec| values.is_fixed(spec.code))
                .map(|spec| spec.code.to_owned()),
        )
        .expect("migration only locks catalog settings");
    let values = document
        .values()
        .expect("generated legacy migration document is valid");
    LegacyMigration {
        document,
        values,
        diagnostics,
    }
}

#[derive(Clone, Copy)]
enum LegacyColonKind {
    Regular,
    Fixed,
    Debug,
}

fn migrate_colon_config(
    source: LegacyConfigSource<'_>,
    kind: LegacyColonKind,
    values: &mut ConfigStore,
    diagnostics: &mut Vec<LegacyMigrationDiagnostic>,
) {
    for line in legacy_lines(source.contents) {
        let content = line.text;
        if content.is_empty() || content.starts_with(';') {
            continue;
        }
        let Some(delimiter) = content.find(':') else {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::MalformedLine,
                "缺少 ':' 分隔符",
            ));
            continue;
        };
        let name = content[..delimiter].trim();
        let Some(mut code) = resolve_code(name) else {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::UnknownSetting,
                format!("未知旧设置 {name:?}"),
            ));
            continue;
        };
        if code == "COMPATIDRAWLINE" {
            code = "COMPATILINEFEEDAS1739".into();
        }
        let permitted = match kind {
            LegacyColonKind::Regular | LegacyColonKind::Fixed => is_regular_code(&code),
            LegacyColonKind::Debug => code.starts_with("DEBUG"),
        };
        if !permitted {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::UnknownSetting,
                format!("{name:?} 不属于该旧配置文件"),
            ));
            continue;
        }

        let fixed = matches!(kind, LegacyColonKind::Fixed);
        if code == "EDITORARGUMENT" {
            let remainder = &content[delimiter + 1..];
            let raw_value = remainder
                .split_once(':')
                .map_or(remainder, |(value, _)| value);
            values
                .values
                .insert(code, ConfigValue::String(raw_value.to_owned()));
            continue;
        }
        let remainder = &content[delimiter + 1..];
        let raw_value = if code == "TEXTEDITOR" {
            remainder.trim()
        } else {
            remainder
                .split_once(':')
                .map_or(remainder, |(value, _)| value)
                .trim()
        };
        if values.apply(&code, raw_value, fixed).is_err() {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::InvalidValue,
                format!("无法迁移设置 {name:?}"),
            ));
        }
    }
}

fn migrate_setting_json(
    source: LegacyConfigSource<'_>,
    values: &mut ConfigStore,
    diagnostics: &mut Vec<LegacyMigrationDiagnostic>,
) {
    match serde_json::from_str::<JsonValue>(source.contents.trim_start_matches('\u{feff}')) {
        Ok(value) => {
            if let Some(value) = value.get("UseNewRandom").and_then(JsonValue::as_bool) {
                let _ = values.apply("UseNewRandom", if value { "YES" } else { "NO" }, false);
            }
        }
        Err(error) => diagnostics.push(LegacyMigrationDiagnostic {
            kind: LegacyMigrationDiagnosticKind::InvalidJson,
            relative_path: source.relative_path.into(),
            line: Some(error.line()),
            span: None,
            message: format!("无法解析 JSON：{error}"),
        }),
    }
}

fn migrate_replace(
    source: LegacyConfigSource<'_>,
    values: &mut ConfigStore,
    diagnostics: &mut Vec<LegacyMigrationDiagnostic>,
) {
    for line in legacy_lines(source.contents) {
        let content = line.text.trim();
        if content.is_empty() || content.starts_with(';') {
            continue;
        }
        let Some(delimiter) = content.find([',', ':']) else {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::MalformedLine,
                "缺少 ',' 或 ':' 分隔符",
            ));
            continue;
        };
        let name = content[..delimiter].trim();
        let Some(code) = resolve_code(name) else {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::UnknownSetting,
                format!("未知替换设置 {name:?}"),
            ));
            continue;
        };
        if !is_replace_code(&code) {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::UnknownSetting,
                format!("{name:?} 不是 _Replace.csv 设置"),
            ));
            continue;
        }
        let setting = content[delimiter + 1..].trim();
        if setting.is_empty() || values.apply(&code, setting, false).is_err() {
            diagnostics.push(diagnostic(
                source,
                &line,
                LegacyMigrationDiagnosticKind::InvalidValue,
                format!("无法迁移替换设置 {name:?}"),
            ));
        }
    }
}

fn normalize_legacy_values(values: &mut ConfigStore) {
    for spec in rera_catalog() {
        let Some(ConfigValue::Integer(value)) = values.get_code(spec.code).cloned() else {
            continue;
        };
        let normalized = value
            .max(spec.minimum.unwrap_or(i64::MIN))
            .min(spec.maximum.unwrap_or(i64::MAX));
        values.values.insert(
            spec.code.to_ascii_uppercase(),
            ConfigValue::Integer(normalized),
        );
    }
}

fn named_source<'a>(
    sources: &'a [LegacyConfigSource<'a>],
    name: &str,
) -> Option<LegacyConfigSource<'a>> {
    let mut matches = sources
        .iter()
        .copied()
        .filter(|source| basename(source.relative_path).eq_ignore_ascii_case(name))
        .collect::<Vec<_>>();
    matches.sort_by_key(|source| source.relative_path.to_ascii_lowercase());
    matches.into_iter().next()
}

fn preferred_source<'a>(
    sources: &'a [LegacyConfigSource<'a>],
    preferred: &str,
    fallback: &str,
) -> Option<LegacyConfigSource<'a>> {
    named_source(sources, preferred).or_else(|| named_source(sources, fallback))
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

struct LegacyLine<'a> {
    number: usize,
    span: ByteSpan,
    text: &'a str,
}

fn legacy_lines(input: &str) -> Vec<LegacyLine<'_>> {
    let (input, source_offset) = input
        .strip_prefix('\u{feff}')
        .map_or((input, 0), |input| (input, '\u{feff}'.len_utf8()));
    let mut lines = Vec::new();
    let mut start = 0;
    let mut number = 1;
    let bytes = input.as_bytes();
    let mut cursor = 0;
    while cursor <= bytes.len() {
        let at_end = cursor == bytes.len();
        let at_line_end = !at_end && matches!(bytes[cursor], b'\r' | b'\n');
        if at_end || at_line_end {
            lines.push(LegacyLine {
                number,
                span: ByteSpan {
                    start: start + source_offset,
                    end: cursor + source_offset,
                },
                text: &input[start..cursor],
            });
            if at_end {
                break;
            }
            if bytes[cursor] == b'\r' && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\n' {
                cursor += 1;
            }
            cursor += 1;
            start = cursor;
            number += 1;
        } else {
            cursor += 1;
        }
    }
    lines
}

fn diagnostic(
    source: LegacyConfigSource<'_>,
    line: &LegacyLine<'_>,
    kind: LegacyMigrationDiagnosticKind,
    message: impl Into<String>,
) -> LegacyMigrationDiagnostic {
    LegacyMigrationDiagnostic {
        kind,
        relative_path: source.relative_path.into(),
        line: Some(line.number),
        span: Some(line.span),
        message: message.into(),
    }
}
