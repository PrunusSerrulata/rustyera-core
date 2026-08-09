use era_config::{
    ConfigStore, ConfigValue, LegacyConfigSource, ReraConfigDocument, migrate_legacy_configuration,
};
use era_runtime_protocol::{
    FileCategory, FilePayload, ProtocolDiagnostic, RuntimeLogLevel, SourceLocation,
};
use erabasic_analyzer::WarningPolicy;
use erabasic_data::LegacyEncoding;

use super::{SemanticConfig, inspect_deferred_file, project_diagnostic};

pub(super) struct ParsedConfiguration {
    pub(super) semantic: SemanticConfig,
    pub(super) document: ReraConfigDocument,
    pub(super) generated_source: Option<String>,
}

pub(super) fn parse_configuration(
    files: &[era_runtime_protocol::SubmittedFile],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> ParsedConfiguration {
    let root = files.iter().find(|file| {
        file.category == FileCategory::Configuration
            && file
                .relative_path
                .replace('\\', "/")
                .eq_ignore_ascii_case("reraconfig.toml")
    });
    let (document, values, generated_source) = match root {
        Some(file) => parse_reraconfig(file, diagnostics),
        None => migrate_configuration(files, diagnostics),
    };
    let mut config = SemanticConfig {
        values,
        ..SemanticConfig::default()
    };
    config.use_new_random = matches!(
        config.values.get_code("UseNewRandom"),
        Some(ConfigValue::Boolean(true))
    );
    if config.use_new_random {
        diagnostics.push(project_diagnostic(
            "runtime.use_new_random_ignored",
            RuntimeLogLevel::Warning,
            "UseNewRandom=true is ignored; the pinned SFMT implementation is always used",
            None,
        ));
    }
    apply_catalog_semantics(&mut config);
    ParsedConfiguration {
        semantic: config,
        document,
        generated_source,
    }
}

fn parse_reraconfig(
    file: &era_runtime_protocol::SubmittedFile,
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> (ReraConfigDocument, ConfigStore, Option<String>) {
    let FilePayload::Utf8(text) = &file.payload else {
        inspect_deferred_file(
            diagnostics,
            &file.relative_path,
            &file.payload,
            true,
            "runtime.configuration_ignored",
            "reraconfig.toml payload was not UTF-8",
        );
        return (ReraConfigDocument::empty(), ConfigStore::default(), None);
    };
    match ReraConfigDocument::parse(text) {
        Ok(document) => {
            let values = document
                .values()
                .expect("a parsed reraconfig remains valid");
            (document, values, None)
        }
        Err(error) => {
            diagnostics.push(project_diagnostic(
                "runtime.invalid_reraconfig",
                RuntimeLogLevel::Error,
                error.to_string(),
                Some(SourceLocation {
                    relative_path: file.relative_path.clone(),
                    byte_start: error.span.map_or(0, |span| span.start as u64),
                    byte_end: error.span.map_or(0, |span| span.end as u64),
                    line: None,
                    byte_column: None,
                }),
            ));
            (ReraConfigDocument::empty(), ConfigStore::default(), None)
        }
    }
}

fn migrate_configuration(
    files: &[era_runtime_protocol::SubmittedFile],
    diagnostics: &mut Vec<ProtocolDiagnostic>,
) -> (ReraConfigDocument, ConfigStore, Option<String>) {
    for file in files
        .iter()
        .filter(|file| is_legacy_configuration_source(file))
    {
        if !matches!(file.payload, FilePayload::Utf8(_)) {
            inspect_deferred_file(
                diagnostics,
                &file.relative_path,
                &file.payload,
                true,
                "runtime.legacy_configuration_ignored",
                "legacy configuration payload could not be decoded",
            );
        }
    }
    let sources = files
        .iter()
        .filter_map(|file| match &file.payload {
            FilePayload::Utf8(contents) if is_legacy_configuration_source(file) => {
                Some(LegacyConfigSource {
                    relative_path: &file.relative_path,
                    contents,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let migration = migrate_legacy_configuration(&sources);
    for diagnostic in migration.diagnostics {
        diagnostics.push(project_diagnostic(
            "runtime.legacy_configuration_migration",
            RuntimeLogLevel::Warning,
            diagnostic.message,
            Some(SourceLocation {
                relative_path: diagnostic.relative_path,
                byte_start: diagnostic.span.map_or(0, |span| span.start as u64),
                byte_end: diagnostic.span.map_or(0, |span| span.end as u64),
                line: diagnostic.line.map(|line| line as u64),
                byte_column: None,
            }),
        ));
    }
    let generated_source = (!sources.is_empty()).then(|| migration.document.to_lf_string());
    (migration.document, migration.values, generated_source)
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn is_legacy_configuration_source(file: &era_runtime_protocol::SubmittedFile) -> bool {
    let name = basename(&file.relative_path);
    matches!(
        name.to_ascii_lowercase().as_str(),
        "_default.config"
            | "default.config"
            | "emuera.config"
            | "setting.json"
            | "_fixed.config"
            | "fixed.config"
            | "debug.config"
            | "_replace.csv"
    )
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

pub(super) fn apply_replace_configuration(
    store: &ConfigStore,
    replace: &mut erabasic_data::ReplaceSettings,
) {
    let string = |code| match store.get_code(code) {
        Some(ConfigValue::String(value) | ConfigValue::Enum { value, .. }) => Some(value.clone()),
        _ => None,
    };
    let boolean = |code| match store.get_code(code) {
        Some(ConfigValue::Boolean(value)) => Some(*value),
        _ => None,
    };
    let integer = |code| match store.get_code(code) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        _ => None,
    };
    let character = |code| match store.get_code(code) {
        Some(ConfigValue::Character(value)) => Some(*value),
        _ => None,
    };
    let integer_list = |code| match store.get_code(code) {
        Some(ConfigValue::IntegerList(value)) => Some(value.clone()),
        _ => None,
    };

    if let Some(value) = string("MoneyLabel") {
        replace.money_label = value;
    }
    if let Some(value) = boolean("MoneyFirst") {
        replace.money_first = value;
    }
    if let Some(value) = string("LoadLabel") {
        replace.load_label = value;
    }
    if let Some(value) = integer("MaxShopItem").and_then(|value| i32::try_from(value).ok()) {
        replace.max_shop_item = value;
    }
    if let Some(value) = string("DrawLineString") {
        replace.draw_line_string = value;
    }
    if let Some(value) = character("BarChar1") {
        replace.bar_char_1 = value;
    }
    if let Some(value) = character("BarChar2") {
        replace.bar_char_2 = value;
    }
    if let Some(value) = string("TitleMenuString0") {
        replace.title_menu_string_0 = value;
    }
    if let Some(value) = string("TitleMenuString1") {
        replace.title_menu_string_1 = value;
    }
    if let Some(value) = integer("ComAbleDefault").and_then(|value| i32::try_from(value).ok()) {
        replace.com_able_default = value;
    }
    if let Some(value) = integer_list("StainDefault") {
        replace.stain_default = value;
    }
    if let Some(value) = string("TimeupLabel") {
        replace.timeup_label = value;
    }
    if let Some(value) = integer_list("ExpLvDef") {
        replace.exp_lv_default = value;
    }
    if let Some(value) = integer_list("PalamLvDef") {
        replace.palam_lv_default = value;
    }
    if let Some(value) = integer("pbandDef") {
        replace.pband_default = value;
    }
    if let Some(value) = integer("RelationDef") {
        replace.relation_default = value;
    }
}
