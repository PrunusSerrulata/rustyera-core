use era_runtime_protocol::{
    FileCategory, FilePayload, ProtocolDiagnostic, RuntimeLogLevel, SourceLocation,
};
use erabasic_analyzer::WarningPolicy;
use erabasic_config::{ConfigStore, ConfigValue};
use erabasic_data::LegacyEncoding;

use super::{SemanticConfig, inspect_deferred_file, project_diagnostic};

#[allow(clippy::too_many_lines)]
pub(super) fn parse_configuration(
    files: &[era_runtime_protocol::SubmittedFile],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> SemanticConfig {
    let mut config = SemanticConfig::default();
    let mut configuration_files = files
        .iter()
        .filter(|file| file.category == FileCategory::Configuration)
        .collect::<Vec<_>>();
    // Emuera has a semantic precedence independent of frontend submission order.
    configuration_files.sort_by_key(|file| configuration_precedence(&file.relative_path));
    for file in configuration_files {
        let FilePayload::Utf8(text) = &file.payload else {
            inspect_deferred_file(
                diagnostics,
                &file.relative_path,
                &file.payload,
                true,
                "runtime.configuration_ignored",
                "configuration payload was not UTF-8",
            );
            continue;
        };
        if parse_json_configuration(text, &file.relative_path, &mut config, diagnostics) {
            continue;
        }
        let fixed = is_fixed_configuration(&file.relative_path);
        let debug_configuration = file
            .relative_path
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("debug.config"));
        for (line_index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                diagnostics.push(project_diagnostic(
                    "runtime.invalid_configuration",
                    RuntimeLogLevel::Warning,
                    "configuration line has no ':' separator",
                    Some(SourceLocation {
                        relative_path: file.relative_path.clone(),
                        byte_start: 0,
                        byte_end: 0,
                        line: Some(line_index as u64 + 1),
                        byte_column: None,
                    }),
                ));
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if matches!(name, "UseNewRandom" | "新しい高速な乱数アルゴリズムを使う")
            {
                match parse_boolean(value) {
                    Some(boolean) => config.use_new_random = boolean,
                    None => diagnostics.push(project_diagnostic(
                        "runtime.invalid_configuration",
                        RuntimeLogLevel::Warning,
                        "UseNewRandom must be a boolean value",
                        Some(SourceLocation {
                            relative_path: file.relative_path.clone(),
                            byte_start: 0,
                            byte_end: 0,
                            line: Some(line_index as u64 + 1),
                            byte_column: None,
                        }),
                    )),
                }
                continue;
            }
            let applied = if debug_configuration {
                config.values.apply(name, value, false)
            } else {
                config.values.apply_regular(name, value, fixed)
            };
            if let Err(error) = applied {
                diagnostics.push(project_diagnostic(
                    match error {
                        erabasic_config::ConfigParseError::UnknownKey => {
                            "runtime.unknown_configuration"
                        }
                        erabasic_config::ConfigParseError::InvalidValue => {
                            "runtime.invalid_configuration"
                        }
                    },
                    RuntimeLogLevel::Warning,
                    format!("configuration assignment {name:?} was not applied"),
                    Some(SourceLocation {
                        relative_path: file.relative_path.clone(),
                        byte_start: 0,
                        byte_end: 0,
                        line: Some(line_index as u64 + 1),
                        byte_column: None,
                    }),
                ));
            }
        }
    }
    if config.use_new_random {
        diagnostics.push(project_diagnostic(
            "runtime.use_new_random_ignored",
            RuntimeLogLevel::Warning,
            "UseNewRandom=true is ignored; the pinned SFMT implementation is always used",
            None,
        ));
    }
    apply_catalog_semantics(&mut config);
    config
}

#[allow(clippy::too_many_lines)]
fn apply_catalog_semantics(config: &mut SemanticConfig) {
    let boolean = |code| match config.values.get_code(code) {
        Some(ConfigValue::Boolean(value)) => Some(*value),
        _ => None,
    };
    let integer = |code| match config.values.get_code(code) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        _ => None,
    };
    let string = |code| match config.values.get_code(code) {
        Some(ConfigValue::String(value) | ConfigValue::Enum { value, .. }) => Some(value.as_str()),
        _ => None,
    };
    if let Some(value) = boolean("IgnoreCase") {
        config.csv.ignore_case = value;
        config.analyzer.ignore_case = value;
    }
    if let Some(value) = boolean("UseRenameFile") {
        config.csv.use_rename_file = value;
    }
    if let Some(value) = boolean("UseReplaceFile") {
        config.csv.use_replace_file = value;
    }
    if let Some(value) = boolean("SearchSubdirectory") {
        config.csv.search_subdirectories = value;
    }
    if let Some(value) = boolean("SortWithFilename") {
        config.csv.sort_with_filename = value;
        config.analyzer.sort_with_filename = value;
    }
    if let Some(value) = boolean("CompatiCALLNAME") {
        config.csv.compatible_call_name = value;
    }
    if let Some(value) = boolean("CompatiSPChara") {
        config.csv.compatible_sp_character = value;
    }
    if let Some(value) = boolean("UseERD") {
        config.csv.use_erd = value;
        config.analyzer.use_erd = value;
    }
    if let Some(value) = boolean("VarsizeDimConfig") {
        config.analyzer.varsize_dimension_is_one_based = value;
    }
    if let Some(ConfigValue::Color(value)) = config.values.get_code("ForeColor") {
        config.analyzer.default_foreground_color = i64::from(*value);
    }
    if let Some(value) = boolean("SystemAllowFullSpace") {
        config.csv.allow_full_width_space = value;
        config.analyzer.allow_full_width_space = value;
    }
    if let Some(value) = boolean("SystemIgnoreTripleSymbol") {
        config.analyzer.ignore_triple_symbols = value;
    }
    if let Some(value) = string("useLanguage") {
        config.legacy_encoding = match value.to_ascii_uppercase().as_str() {
            "KOREAN" => LegacyEncoding::Korean,
            "CHINESE_HANS" => LegacyEncoding::ChineseHans,
            "CHINESE_HANT" => LegacyEncoding::ChineseHant,
            _ => LegacyEncoding::Japanese,
        };
    }
    if let Some(value) = string("ReplaceContinuationBR") {
        let value = value.trim_matches('"').to_owned();
        config.csv.continuation_separator.clone_from(&value);
        config.analyzer.continuation_separator = value;
    }

    if let Some(value) = boolean("AllowFunctionOverloading") {
        config.analyzer.allow_function_overloading = value;
    }
    if let Some(value) = boolean("WarnFunctionOverloading") {
        config.analyzer.warn_function_overloading = value;
    }
    if let Some(value) = integer("DisplayWarningLevel").and_then(|value| u8::try_from(value).ok()) {
        config.analyzer.display_warning_level = value;
    }
    if let Some(value) = boolean("IgnoreUncalledFunction") {
        config.analyzer.ignore_uncalled_functions = value;
    }
    if let Some(value) = string("FunctionNotFoundWarning").and_then(parse_warning_policy) {
        config.analyzer.function_not_found = value;
    }
    if let Some(value) = string("FunctionNotCalledWarning").and_then(parse_warning_policy) {
        config.analyzer.function_not_called = value;
    }
    if let Some(value) = boolean("CompatiFuncArgAutoConvert") {
        config.analyzer.compatible_function_argument_auto_convert = value;
    }
    if let Some(value) = boolean("CompatiFuncArgOptional") {
        config.analyzer.compatible_function_argument_optional = value;
    }
    if let Some(value) = boolean("CompatiCallEvent") {
        config.analyzer.compatible_call_event = value;
    }
    if let Some(value) = boolean("SystemSaveInBinary") {
        config.analyzer.system_save_in_binary = value;
        config.save_in_binary = value;
    }

    if let Some(value) = boolean("AutoSave") {
        config.auto_save = value;
    }
    if let Some(value) = boolean("Ctrl_Z_Enabled") {
        config.ctrl_z_enabled = value;
    }
    if let Some(value) = boolean("AllowLongInputByMouse") {
        config.allow_long_input_by_activation = value;
    }
    if let Some(value) = boolean("ZipSaveData") {
        config.compress_save = value;
    }
    if let Some(value) = integer("SaveDataNos").and_then(|value| u32::try_from(value).ok()) {
        config.save_slot_count = value.clamp(20, 80);
    }
    if let Some(value) = integer("WindowX").and_then(|value| u32::try_from(value).ok()) {
        config.viewport_width = value.max(128);
    }
    if let Some(value) = integer("WindowY").and_then(|value| u32::try_from(value).ok()) {
        config.viewport_height = value.max(128);
    }
    if let Some(value) = integer("FontSize").and_then(|value| u32::try_from(value).ok()) {
        config.font_size = value.max(8);
    }
    if let Some(value) = integer("LineHeight").and_then(|value| u32::try_from(value).ok()) {
        config.line_height = value.max(config.font_size);
    }
    if let Some(value) = integer("PrintCPerLine").and_then(|value| u32::try_from(value).ok()) {
        config.print_c_per_line = value.max(1);
    }
    if let Some(value) = integer("PrintCLength").and_then(|value| u32::try_from(value).ok()) {
        config.print_c_length = value.max(1);
    }
}

fn parse_warning_policy(value: &str) -> Option<WarningPolicy> {
    match value.to_ascii_uppercase().as_str() {
        "IGNORE" => Some(WarningPolicy::Ignore),
        "DISPLAY" => Some(WarningPolicy::Display),
        "ONCE" | "ONCEPERFILE" | "ONCE_PER_FILE" => Some(WarningPolicy::OncePerFile),
        "LATER" => Some(WarningPolicy::Later),
        _ => None,
    }
}

fn parse_json_configuration(
    text: &str,
    path: &str,
    config: &mut SemanticConfig,
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> bool {
    let text = text.trim_start_matches('\u{feff}');
    if !text.trim_start().starts_with('{') {
        return false;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            if let Some(boolean) = value
                .get("UseNewRandom")
                .and_then(serde_json::Value::as_bool)
            {
                config.use_new_random = boolean;
            }
        }
        Err(error) => diagnostics.push(project_diagnostic(
            "runtime.invalid_json_configuration",
            RuntimeLogLevel::Warning,
            error.to_string(),
            Some(SourceLocation {
                relative_path: path.into(),
                byte_start: 0,
                byte_end: u64::try_from(text.len()).unwrap_or(u64::MAX),
                line: None,
                byte_column: None,
            }),
        )),
    }
    true
}

fn configuration_precedence(path: &str) -> (u8, String) {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(&normalized);
    let rank = match name {
        "_default.config" | "default.config" => 0,
        "setting.json" => 2,
        "_fixed.config" | "fixed.config" => 3,
        "debug.config" => 4,
        _ => 1,
    };
    (rank, normalized)
}

fn is_fixed_configuration(path: &str) -> bool {
    matches!(
        path.replace('\\', "/")
            .to_ascii_lowercase()
            .rsplit('/')
            .next(),
        Some("_fixed.config" | "fixed.config")
    )
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_uppercase().as_str() {
        "YES" | "TRUE" | "1" => Some(true),
        "NO" | "FALSE" | "0" => Some(false),
        _ => None,
    }
}

pub(super) fn sync_replace_configuration(
    store: &mut ConfigStore,
    replace: &erabasic_data::ReplaceSettings,
) {
    // Replace.csv is parsed by erabasic-csv, then mirrored into the unified script
    // query catalog. This avoids treating replace keys as emuera.config settings.
    let values = [
        ("MoneyLabel", replace.money_label.clone()),
        (
            "MoneyFirst",
            if replace.money_first {
                "YES".into()
            } else {
                "NO".into()
            },
        ),
        ("LoadLabel", replace.load_label.clone()),
        ("MaxShopItem", replace.max_shop_item.to_string()),
        ("DrawLineString", replace.draw_line_string.clone()),
        ("BarChar1", replace.bar_char_1.to_string()),
        ("BarChar2", replace.bar_char_2.to_string()),
        ("TitleMenuString0", replace.title_menu_string_0.clone()),
        ("TitleMenuString1", replace.title_menu_string_1.clone()),
        ("ComAbleDefault", replace.com_able_default.to_string()),
        (
            "StainDefault",
            replace
                .stain_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        ("TimeupLabel", replace.timeup_label.clone()),
        (
            "ExpLvDef",
            replace
                .exp_lv_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        (
            "PalamLvDef",
            replace
                .palam_lv_default
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        ),
        ("pbandDef", replace.pband_default.to_string()),
        ("RelationDef", replace.relation_default.to_string()),
    ];
    for (name, value) in values {
        let _ = store.apply(name, &value, false);
    }
}
