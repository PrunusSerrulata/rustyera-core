use std::collections::BTreeMap;

use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::{
    ConfigApplication, ConfigClient, ConfigValue, browser_application, tauri_application,
    tui_application,
};

use super::{
    RERACONFIG_SCHEMA_VERSION, ReraConfigSpec,
    catalog::{config_to_toml, enum_to_toml, rera_catalog},
};

/// Generate the JSON Schema representation of the built-in configuration catalog.
///
/// # Panics
///
/// Panics only if a built-in catalog path is malformed or the in-memory JSON value cannot be
/// serialized. Both conditions are programmer invariants.
#[must_use]
pub fn generate_json_schema() -> String {
    let specs = rera_catalog();
    let mut sections = BTreeMap::<&str, JsonMap<String, JsonValue>>::new();
    for spec in &specs {
        let (section, key) = spec.path.split_once('.').expect("catalog path is valid");
        sections
            .entry(section)
            .or_default()
            .insert(key.into(), schema_for(spec));
    }
    let paths = specs.iter().map(|spec| spec.path).collect::<Vec<_>>();
    let mut properties = JsonMap::new();
    properties.insert(
        "meta".into(),
        json!({
            "type": "object",
            "description": "配置格式元数据。所有字段均可省略，省略时采用 schema 版本 1 且没有锁定项。",
            "additionalProperties": false,
            "properties": {
                "schema_version": {
                    "type": "integer",
                    "const": RERACONFIG_SCHEMA_VERSION,
                    "default": RERACONFIG_SCHEMA_VERSION,
                    "description": "reraconfig.toml 的格式版本。当前仅支持整数 1。"
                },
                "locked_settings": {
                    "type": "array",
                    "default": [],
                    "uniqueItems": true,
                    "items": { "type": "string", "enum": paths },
                    "description": "不可由客户端设置面板修改的规范设置路径，承接旧 _fixed.config 语义。"
                }
            }
        }),
    );
    for (section, fields) in sections {
        properties.insert(
            section.into(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": fields
            }),
        );
    }
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:rustyera:reraconfig:schema:1",
        "title": "RustyEra reraconfig.toml",
        "description": "RustyEra 跨客户端项目配置。所有设置项均可省略，省略时采用各项统一默认值。",
        "type": "object",
        "additionalProperties": false,
        "properties": properties
    });
    let mut output = serde_json::to_string_pretty(&root).expect("schema values are serializable");
    output.push('\n');
    output
}

/// Generate a complete TOML example with Chinese comments from the built-in catalog.
///
/// # Panics
///
/// Panics only if a built-in catalog path does not contain its required table separator.
#[must_use]
pub fn generate_annotated_example() -> String {
    let mut grouped = BTreeMap::<&str, Vec<ReraConfigSpec>>::new();
    for spec in rera_catalog() {
        let section = spec.path.split_once('.').expect("catalog path is valid").0;
        grouped.entry(section).or_default().push(spec);
    }
    let mut output = String::from(
        "# RustyEra 跨客户端项目配置示例。所有字段都可以省略，省略时使用注释中的默认值。\n\n[meta]\n# 类型：整数；当前仅支持 1。\nschema_version = 1\n# 类型：字符串数组；列出的设置不可由客户端设置面板修改。\nlocked_settings = []\n",
    );
    for (section, specs) in grouped {
        output.push_str("\n[");
        output.push_str(section);
        output.push_str("]\n");
        for spec in specs {
            output.push_str("# ");
            output.push_str(&description_with_type(&spec));
            output.push('\n');
            let key = spec.path.split_once('.').expect("catalog path is valid").1;
            output.push_str(key);
            output.push_str(" = ");
            output.push_str(&config_to_toml(&spec, &spec.default).to_string());
            output.push('\n');
        }
    }
    output
}

fn schema_for(spec: &ReraConfigSpec) -> JsonValue {
    let mut schema = JsonMap::new();
    schema.insert(
        "description".into(),
        JsonValue::String(description_with_type(spec)),
    );
    schema.insert("default".into(), default_json(spec));
    schema.insert("x-rustyera-setting-id".into(), json!(spec.id));
    schema.insert("x-rustyera-legacy-code".into(), json!(spec.code));
    schema.insert(
        "x-rustyera-clients".into(),
        JsonValue::Array(
            spec.clients
                .iter()
                .map(|client| json!(client_name(*client)))
                .collect(),
        ),
    );
    schema.insert(
        "x-rustyera-application".into(),
        json!(application_name(spec.code)),
    );
    if spec.deprecated {
        schema.insert("deprecated".into(), JsonValue::Bool(true));
    }
    match &spec.default {
        ConfigValue::Boolean(_) => schema.insert("type".into(), json!("boolean")),
        ConfigValue::Integer(_) => {
            schema.insert("type".into(), json!("integer"));
            if let Some(minimum) = spec.minimum {
                schema.insert("minimum".into(), json!(minimum));
            }
            if let Some(maximum) = spec.maximum {
                schema.insert("maximum".into(), json!(maximum));
            }
            None
        }
        ConfigValue::String(_) => schema.insert("type".into(), json!("string")),
        ConfigValue::Enum { allowed, .. } => {
            schema.insert("type".into(), json!("string"));
            schema.insert(
                "enum".into(),
                JsonValue::Array(
                    allowed
                        .iter()
                        .map(|value| json!(enum_to_toml(spec.code, value)))
                        .collect(),
                ),
            )
        }
        ConfigValue::Color(_) => {
            schema.insert("type".into(), json!("array"));
            schema.insert("minItems".into(), json!(3));
            schema.insert("maxItems".into(), json!(3));
            schema.insert(
                "items".into(),
                json!({"type": "integer", "minimum": 0, "maximum": 255}),
            )
        }
        ConfigValue::Character(_) => {
            schema.insert("type".into(), json!("string"));
            schema.insert("minLength".into(), json!(1));
            schema.insert("maxLength".into(), json!(1))
        }
        ConfigValue::IntegerList(_) => {
            schema.insert("type".into(), json!("array"));
            schema.insert("items".into(), json!({"type": "integer"}))
        }
        ConfigValue::StringList(_) => {
            schema.insert("type".into(), json!("array"));
            schema.insert("items".into(), json!({"type": "string"}))
        }
    };
    JsonValue::Object(schema)
}

fn default_json(spec: &ReraConfigSpec) -> JsonValue {
    match &spec.default {
        ConfigValue::Boolean(value) => json!(value),
        ConfigValue::Integer(value) => json!(value),
        ConfigValue::String(value) => json!(value),
        ConfigValue::Enum { value, .. } => json!(enum_to_toml(spec.code, value)),
        ConfigValue::Color(value) => {
            json!([(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff])
        }
        ConfigValue::Character(value) => json!(value.to_string()),
        ConfigValue::IntegerList(values) => json!(values),
        ConfigValue::StringList(values) => json!(values),
    }
}

fn description_with_type(spec: &ReraConfigSpec) -> String {
    let kind = match &spec.default {
        ConfigValue::Boolean(_) => "布尔",
        ConfigValue::Integer(_) => "整数",
        ConfigValue::String(_) => "字符串",
        ConfigValue::Enum { .. } => "字符串枚举",
        ConfigValue::Color(_) => "RGB 整数数组",
        ConfigValue::Character(_) => "单个 Unicode 字符",
        ConfigValue::IntegerList(_) => "整数数组",
        ConfigValue::StringList(_) => "字符串数组",
    };
    let range = match (spec.minimum, spec.maximum) {
        (Some(minimum), Some(maximum)) => format!("；范围：{minimum}..={maximum}"),
        (Some(minimum), None) => format!("；最小值：{minimum}"),
        (None, Some(maximum)) => format!("；最大值：{maximum}"),
        (None, None) => String::new(),
    };
    let allowed = match &spec.default {
        ConfigValue::Enum { allowed, .. } => format!(
            "；可选值：{}",
            allowed
                .iter()
                .map(|value| format!("\"{}\"", enum_to_toml(spec.code, value)))
                .collect::<Vec<_>>()
                .join("、")
        ),
        ConfigValue::Color(_) => "；每个分量范围：0..=255".into(),
        _ => String::new(),
    };
    format!(
        "用途：{}；类型：{kind}{range}{allowed}",
        spec.description_zh_cn
    )
}

fn client_name(client: ConfigClient) -> &'static str {
    match client {
        ConfigClient::Runtime => "runtime",
        ConfigClient::Tui => "tui",
        ConfigClient::Browser => "browser",
        ConfigClient::Tauri => "tauri",
    }
}

fn application_name(code: &str) -> &'static str {
    let applications = [
        tui_application(code),
        browser_application(code),
        tauri_application(code),
    ];
    if applications.contains(&Some(ConfigApplication::Hot)) {
        "hot"
    } else if applications.contains(&Some(ConfigApplication::Restart)) {
        "restart"
    } else {
        "unsupported"
    }
}
