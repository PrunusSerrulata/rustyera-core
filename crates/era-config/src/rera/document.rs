use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{Array, Document, DocumentMut, Item, Table, Value, value};

use crate::{ConfigStore, ConfigValue};

use super::{
    ByteSpan, RERACONFIG_SCHEMA_VERSION, ReraConfigError, ReraConfigErrorKind,
    catalog::{config_to_toml, parse_toml_value, rera_catalog, validate_config_value},
    error_at, normalize_line_endings,
    retired::{RETIRED_CONFIG_SPECS, retired_by_path},
};

mod source;

use source::{available_span, collect_source_spans, offset_span, shift_error};

#[derive(Clone, Debug)]
pub struct ReraConfigDocument {
    document: DocumentMut,
    source_offset: usize,
    source_spans: BTreeMap<String, ByteSpan>,
    upgraded_from_previous_schema: bool,
    retired_codes: Vec<&'static str>,
}

impl ReraConfigDocument {
    #[must_use]
    pub fn empty() -> Self {
        let mut document = DocumentMut::new();
        document["meta"] = Item::Table(Table::new());
        document["meta"]["schema_version"] = value(RERACONFIG_SCHEMA_VERSION);
        document["meta"]["locked_settings"] = Item::Value(Value::Array(Array::new()));
        Self {
            document,
            source_offset: 0,
            source_spans: BTreeMap::new(),
            upgraded_from_previous_schema: false,
            retired_codes: Vec::new(),
        }
    }

    /// Parse and strictly validate one UTF-8 `reraconfig.toml` document.
    ///
    /// Both LF and CRLF are accepted. Error spans always refer to UTF-8 byte offsets in the
    /// caller's original input, including an optional UTF-8 BOM.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed TOML, unknown fields, invalid metadata, types, enum values,
    /// ranges, or unsupported inline/dotted table structures. Missing settings retain defaults.
    pub fn parse(input: &str) -> Result<Self, ReraConfigError> {
        let (source, source_offset) = input
            .strip_prefix('\u{feff}')
            .map_or((input, 0), |source| (source, '\u{feff}'.len_utf8()));
        let document = source.parse::<Document<String>>().map_err(|error| {
            error_at(
                ReraConfigErrorKind::TomlSyntax,
                None,
                error
                    .span()
                    .map(|span| offset_span(span.into(), source_offset)),
                format!("TOML 语法错误：{error}"),
            )
        })?;
        let source_spans = collect_source_spans(&document, source_offset);
        let source_version = source_schema_version(&document, source_offset, &source_spans)?;
        let mut document = document.into_mut();
        let retired_codes = if source_version == 1 {
            validate_v1_meta(&document, source_offset, &source_spans)?;
            upgrade_v1_document(&mut document)
        } else {
            Vec::new()
        };
        let result = Self {
            document,
            source_offset,
            source_spans,
            upgraded_from_previous_schema: source_version == 1,
            retired_codes,
        };
        result.values()?;
        Ok(result)
    }

    /// Return the effective typed configuration with defaults and locks applied.
    ///
    /// # Errors
    ///
    /// Returns an error when the document contains a field not accepted by the current schema.
    pub fn values(&self) -> Result<ConfigStore, ReraConfigError> {
        validate_meta(&self.document, self.source_offset, &self.source_spans)?;
        let specs = rera_catalog();
        let by_path = specs
            .iter()
            .map(|spec| (spec.path, spec))
            .collect::<BTreeMap<_, _>>();
        let mut store = ConfigStore::default();
        for (section, item) in self.document.iter() {
            if section == "meta" {
                continue;
            }
            let table = item.as_table().ok_or_else(|| {
                self.error_for_item(
                    ReraConfigErrorKind::UnsupportedStructure,
                    Some(section),
                    item,
                    "顶层设置必须使用普通 TOML 表",
                )
            })?;
            if table.is_dotted() {
                return Err(self.error_for_item(
                    ReraConfigErrorKind::UnsupportedStructure,
                    Some(section),
                    item,
                    "不支持 dotted table，请使用 [section] 表头",
                ));
            }
            for (key, field) in table {
                let path = format!("{section}.{key}");
                let spec = by_path.get(path.as_str()).ok_or_else(|| {
                    self.error_for_item(
                        ReraConfigErrorKind::UnknownField,
                        Some(&path),
                        field,
                        "未知设置项",
                    )
                })?;
                if field.as_table_like().is_some() {
                    return Err(self.error_for_item(
                        ReraConfigErrorKind::UnsupportedStructure,
                        Some(&path),
                        field,
                        "不允许嵌套表或 inline table",
                    ));
                }
                let parsed = parse_toml_value(spec, field).map_err(|error| {
                    shift_error(
                        error,
                        self.source_offset,
                        self.source_spans.get(spec.path).copied(),
                    )
                })?;
                store.values.insert(spec.code.to_ascii_uppercase(), parsed);
            }
        }
        for locked in locked_paths(&self.document, self.source_offset, &self.source_spans)? {
            let spec = by_path.get(locked.path.as_str()).ok_or_else(|| {
                error_at(
                    ReraConfigErrorKind::UnknownField,
                    Some("meta.locked_settings"),
                    locked.span,
                    format!("包含未知设置路径：{}", locked.path),
                )
            })?;
            store.fixed.insert(spec.code.to_ascii_uppercase(), true);
        }
        Ok(store)
    }

    /// Whether parsing upgraded a schema version 1 document to the current schema.
    #[must_use]
    pub fn was_upgraded(&self) -> bool {
        self.upgraded_from_previous_schema
    }

    /// Legacy setting codes removed while upgrading this document.
    #[must_use]
    pub fn retired_codes(&self) -> &[&'static str] {
        &self.retired_codes
    }

    /// Set one canonical setting value while preserving unrelated formatting and comments.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown code, a locked setting, or an invalid value.
    pub fn set_code(&mut self, code: &str, setting: &ConfigValue) -> Result<(), ReraConfigError> {
        self.set_code_impl(code, setting, true)
    }

    /// Replace the locked-setting path list with a deterministic canonical list.
    ///
    /// # Errors
    ///
    /// Returns an error if any code is unknown.
    pub fn set_locked_codes(
        &mut self,
        codes: impl IntoIterator<Item = String>,
    ) -> Result<(), ReraConfigError> {
        let by_code = rera_catalog()
            .into_iter()
            .map(|spec| (spec.code.to_ascii_uppercase(), spec.path))
            .collect::<BTreeMap<_, _>>();
        let mut paths = BTreeSet::new();
        for code in codes {
            let path = by_code.get(&code.to_ascii_uppercase()).ok_or_else(|| {
                error_at(
                    ReraConfigErrorKind::UnknownField,
                    Some("meta.locked_settings"),
                    None,
                    format!("未知设置 code：{code}"),
                )
            })?;
            paths.insert(*path);
        }
        self.set_locked_paths(paths);
        Ok(())
    }

    #[must_use]
    pub fn to_lf_string(&self) -> String {
        let mut output = normalize_line_endings(&self.document.to_string());
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output
    }

    pub(super) fn set_code_unchecked(
        &mut self,
        code: &str,
        setting: &ConfigValue,
    ) -> Result<(), ReraConfigError> {
        self.set_code_impl(code, setting, false)
    }

    pub(super) fn set_locked_codes_unchecked(
        &mut self,
        codes: impl IntoIterator<Item = String>,
    ) -> Result<(), ReraConfigError> {
        self.set_locked_codes(codes)
    }

    fn set_code_impl(
        &mut self,
        code: &str,
        setting: &ConfigValue,
        enforce_lock: bool,
    ) -> Result<(), ReraConfigError> {
        let spec = rera_catalog()
            .into_iter()
            .find(|spec| spec.code.eq_ignore_ascii_case(code))
            .ok_or_else(|| {
                error_at(
                    ReraConfigErrorKind::UnknownField,
                    Some(code),
                    None,
                    "未知设置 code",
                )
            })?;
        validate_config_value(&spec, setting, None)?;
        if enforce_lock
            && locked_paths(&self.document, self.source_offset, &self.source_spans)?
                .iter()
                .any(|locked| locked.path == spec.path)
        {
            return Err(error_at(
                ReraConfigErrorKind::LockedSetting,
                Some(spec.path),
                None,
                "设置已被 meta.locked_settings 锁定",
            ));
        }
        let (section, key) = spec
            .path
            .split_once('.')
            .expect("reraconfig paths contain one table separator");
        if !self.document.as_table().contains_key(section) {
            self.document[section] = Item::Table(Table::new());
        }
        let table = self
            .document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .ok_or_else(|| {
                error_at(
                    ReraConfigErrorKind::UnsupportedStructure,
                    Some(section),
                    None,
                    "设置分区不是普通 TOML 表",
                )
            })?;
        let mut replacement = config_to_toml(&spec, setting);
        if let Some(existing) = table.get_mut(key) {
            if let Some(value) = existing.as_value() {
                replacement.decor_mut().clone_from(value.decor());
            }
            *existing = Item::Value(replacement);
        } else {
            table.insert(key, Item::Value(replacement));
        }
        Ok(())
    }

    fn set_locked_paths(&mut self, paths: BTreeSet<&'static str>) {
        let mut replacement = Value::Array(Array::new());
        if let Value::Array(array) = &mut replacement {
            for path in paths {
                array.push(path);
            }
        }
        if !self.document.as_table().contains_key("meta") {
            self.document["meta"] = Item::Table(Table::new());
        }
        let meta = self
            .document
            .get_mut("meta")
            .and_then(Item::as_table_mut)
            .expect("validated or newly created reraconfig meta is a table");
        if let Some(existing) = meta.get_mut("locked_settings") {
            if let Some(value) = existing.as_value() {
                replacement.decor_mut().clone_from(value.decor());
            }
            *existing = Item::Value(replacement);
        } else {
            meta.insert("locked_settings", Item::Value(replacement));
        }
    }

    fn error_for_item(
        &self,
        kind: ReraConfigErrorKind,
        path: Option<&str>,
        item: &Item,
        message: impl Into<String>,
    ) -> ReraConfigError {
        error_at(
            kind,
            path,
            item.span()
                .map(ByteSpan::from)
                .map(|span| offset_span(span, self.source_offset))
                .or_else(|| path.and_then(|path| self.source_spans.get(path).copied())),
            message,
        )
    }
}

struct LockedPath {
    path: String,
    span: Option<ByteSpan>,
}

fn validate_meta(
    document: &DocumentMut,
    source_offset: usize,
    source_spans: &BTreeMap<String, ByteSpan>,
) -> Result<(), ReraConfigError> {
    let Some(meta) = document.get("meta") else {
        return Ok(());
    };
    let table = meta.as_table().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::UnsupportedStructure,
            Some("meta"),
            available_span(meta, "meta", source_offset, source_spans),
            "meta 必须是普通 TOML 表",
        )
    })?;
    if table.is_dotted() {
        return Err(error_at(
            ReraConfigErrorKind::UnsupportedStructure,
            Some("meta"),
            available_span(meta, "meta", source_offset, source_spans),
            "meta 不支持 dotted table",
        ));
    }
    for (key, item) in table {
        match key {
            "schema_version" => {
                if item.as_integer() != Some(RERACONFIG_SCHEMA_VERSION) {
                    return Err(error_at(
                        ReraConfigErrorKind::InvalidMetadata,
                        Some("meta.schema_version"),
                        available_span(item, "meta.schema_version", source_offset, source_spans),
                        format!("仅支持整数 schema 版本 {RERACONFIG_SCHEMA_VERSION}"),
                    ));
                }
            }
            "locked_settings" => {
                let _ = item.as_array().ok_or_else(|| {
                    error_at(
                        ReraConfigErrorKind::InvalidMetadata,
                        Some("meta.locked_settings"),
                        available_span(item, "meta.locked_settings", source_offset, source_spans),
                        "必须是字符串数组",
                    )
                })?;
            }
            _ => {
                return Err(error_at(
                    ReraConfigErrorKind::UnknownField,
                    Some(&format!("meta.{key}")),
                    available_span(item, &format!("meta.{key}"), source_offset, source_spans),
                    "未知元数据字段",
                ));
            }
        }
    }
    Ok(())
}

fn source_schema_version(
    document: &Document<String>,
    source_offset: usize,
    source_spans: &BTreeMap<String, ByteSpan>,
) -> Result<i64, ReraConfigError> {
    let Some(meta) = document.get("meta") else {
        return Ok(1);
    };
    let table = meta.as_table().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::UnsupportedStructure,
            Some("meta"),
            available_span(meta, "meta", source_offset, source_spans),
            "meta 必须是普通 TOML 表",
        )
    })?;
    let Some(version) = table.get("schema_version") else {
        return Ok(1);
    };
    let value = version.as_integer().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidMetadata,
            Some("meta.schema_version"),
            available_span(version, "meta.schema_version", source_offset, source_spans),
            "schema_version 必须是整数",
        )
    })?;
    if matches!(value, 1 | RERACONFIG_SCHEMA_VERSION) {
        Ok(value)
    } else {
        Err(error_at(
            ReraConfigErrorKind::InvalidMetadata,
            Some("meta.schema_version"),
            available_span(version, "meta.schema_version", source_offset, source_spans),
            format!("仅支持 schema 版本 1 和 {RERACONFIG_SCHEMA_VERSION}"),
        ))
    }
}

fn validate_v1_meta(
    document: &DocumentMut,
    source_offset: usize,
    source_spans: &BTreeMap<String, ByteSpan>,
) -> Result<(), ReraConfigError> {
    let Some(meta) = document.get("meta") else {
        return Ok(());
    };
    let table = meta.as_table().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::UnsupportedStructure,
            Some("meta"),
            available_span(meta, "meta", source_offset, source_spans),
            "meta 必须是普通 TOML 表",
        )
    })?;
    if table.is_dotted() {
        return Err(error_at(
            ReraConfigErrorKind::UnsupportedStructure,
            Some("meta"),
            available_span(meta, "meta", source_offset, source_spans),
            "meta 不支持 dotted table",
        ));
    }
    for (key, item) in table {
        match key {
            "schema_version" => {}
            "locked_settings" => {
                validate_v1_locked_settings(item, source_offset, source_spans)?;
            }
            _ => {
                return Err(error_at(
                    ReraConfigErrorKind::UnknownField,
                    Some(&format!("meta.{key}")),
                    available_span(item, &format!("meta.{key}"), source_offset, source_spans),
                    "未知元数据字段",
                ));
            }
        }
    }
    Ok(())
}

fn validate_v1_locked_settings(
    item: &Item,
    source_offset: usize,
    source_spans: &BTreeMap<String, ByteSpan>,
) -> Result<(), ReraConfigError> {
    let array = item.as_array().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidMetadata,
            Some("meta.locked_settings"),
            available_span(item, "meta.locked_settings", source_offset, source_spans),
            "必须是字符串数组",
        )
    })?;
    let mut paths = BTreeSet::new();
    for value in array {
        let span = value
            .span()
            .map(ByteSpan::from)
            .map(|span| offset_span(span, source_offset))
            .or_else(|| source_spans.get("meta.locked_settings").copied());
        let path = value.as_str().ok_or_else(|| {
            error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "数组项必须是字符串",
            )
        })?;
        if !paths.insert(path) {
            return Err(error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "不允许重复路径",
            ));
        }
    }
    Ok(())
}

fn upgrade_v1_document(document: &mut DocumentMut) -> Vec<&'static str> {
    const DRAWLINE_PATH: &str = "compatibility.drawline_starts_new_line";
    const REPLACEMENT_PATH: &str = "compatibility.legacy_nonbutton_wrapping";

    let drawline_enabled = document
        .get("compatibility")
        .and_then(Item::as_table)
        .and_then(|section| section.get("drawline_starts_new_line"))
        .and_then(Item::as_bool)
        .unwrap_or(false);
    let mut locked = document
        .get("meta")
        .and_then(|meta| meta.get("locked_settings"))
        .and_then(Item::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let drawline_locked = locked.remove(DRAWLINE_PATH);
    let mut retired_codes = Vec::new();
    for spec in RETIRED_CONFIG_SPECS {
        let (section, key) = spec.path.split_once('.').expect("retired path is valid");
        let removed = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .and_then(|table| table.remove(key))
            .is_some();
        if locked.remove(spec.path) || removed {
            retired_codes.push(spec.code);
        }
    }
    for section in RETIRED_CONFIG_SPECS
        .iter()
        .filter_map(|spec| spec.path.split_once('.').map(|(section, _)| section))
        .collect::<BTreeSet<_>>()
    {
        if document
            .get(section)
            .and_then(Item::as_table)
            .is_some_and(Table::is_empty)
        {
            document.remove(section);
        }
    }
    if drawline_enabled {
        let compatibility = ensure_table(document, "compatibility");
        if let Some(existing) = compatibility.get_mut("legacy_nonbutton_wrapping") {
            if existing.as_bool().is_some() {
                let mut replacement = Value::from(true);
                if let Some(existing) = existing.as_value() {
                    replacement.decor_mut().clone_from(existing.decor());
                }
                *existing = Item::Value(replacement);
            }
        } else {
            compatibility.insert("legacy_nonbutton_wrapping", value(true));
        }
    }
    if drawline_locked {
        locked.insert(REPLACEMENT_PATH.to_owned());
    }
    let meta = ensure_table(document, "meta");
    meta.insert("schema_version", value(RERACONFIG_SCHEMA_VERSION));
    let mut locked_settings = Array::new();
    for path in locked {
        if retired_by_path(&path).is_none() {
            locked_settings.push(path);
        }
    }
    meta.insert(
        "locked_settings",
        Item::Value(Value::Array(locked_settings)),
    );
    retired_codes
}

fn ensure_table<'a>(document: &'a mut DocumentMut, section: &str) -> &'a mut Table {
    if !document.as_table().contains_key(section) {
        document[section] = Item::Table(Table::new());
    }
    document
        .get_mut(section)
        .and_then(Item::as_table_mut)
        .expect("created section is a table")
}

fn locked_paths(
    document: &DocumentMut,
    source_offset: usize,
    source_spans: &BTreeMap<String, ByteSpan>,
) -> Result<Vec<LockedPath>, ReraConfigError> {
    let Some(item) = document
        .get("meta")
        .and_then(|meta| meta.get("locked_settings"))
    else {
        return Ok(Vec::new());
    };
    let array = item.as_array().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidMetadata,
            Some("meta.locked_settings"),
            available_span(item, "meta.locked_settings", source_offset, source_spans),
            "必须是字符串数组",
        )
    })?;
    let mut paths = BTreeSet::new();
    let mut locked = Vec::new();
    for value in array {
        let span = value
            .span()
            .map(ByteSpan::from)
            .map(|span| offset_span(span, source_offset))
            .or_else(|| source_spans.get("meta.locked_settings").copied());
        let path = value.as_str().ok_or_else(|| {
            error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "数组项必须是字符串",
            )
        })?;
        if !paths.insert(path.to_owned()) {
            return Err(error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "不允许重复路径",
            ));
        }
        locked.push(LockedPath {
            path: path.to_owned(),
            span,
        });
    }
    Ok(locked)
}
