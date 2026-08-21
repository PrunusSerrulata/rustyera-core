//! Portable, I/O-free representation of the Era project configuration catalog.
//!
//! This crate deliberately stores client-specific settings as compatibility values.
//! Consumers decide which settings have portable runtime semantics; parsing a setting
//! never grants access to a device or forces a frontend rendering choice.

mod rera;

pub use rera::{
    ByteSpan, LegacyConfigSource, LegacyMigration, LegacyMigrationDiagnostic,
    LegacyMigrationDiagnosticKind, RERACONFIG_SCHEMA_VERSION, ReraConfigDocument, ReraConfigError,
    ReraConfigErrorKind, ReraConfigSpec, generate_annotated_example, generate_json_schema,
    migrate_legacy_configuration, normalize_line_endings, rera_catalog,
};

mod catalog;
mod store;
mod value;

pub use catalog::{
    ConfigApplication, ConfigClient, ConfigEffect, ConfigSpec, browser_application,
    browser_configurable, catalog, tauri_application, tauri_configurable, tui_application,
    tui_configurable, tui_default, web_default,
};
pub use store::{ConfigParseError, ConfigStore, is_regular_code};
pub(crate) use store::{is_replace_code, resolve_code};
pub use value::{ConfigValue, ScriptConfigValue};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_aliases_types_and_fixed_precedence_are_deterministic() {
        assert_eq!(catalog().len(), 87, "Era configuration catalog drifted");
        let mut store = ConfigStore::default();
        assert_eq!(
            store.get("フォントサイズ").unwrap().script_value(),
            ScriptConfigValue::Integer(18)
        );
        assert_eq!(
            store.get("文字色").unwrap().script_value(),
            ScriptConfigValue::Integer(0x00C0_C0C0)
        );
        assert_eq!(
            store.get("汚れの初期値").unwrap().script_value(),
            ScriptConfigValue::String("System.Collections.Generic.List`1[System.Int64]".into())
        );
        store.apply("Text color", "1,2,3", false).unwrap();
        assert_eq!(
            store.get("文字色").unwrap().script_value(),
            ScriptConfigValue::Integer(0x0001_0203)
        );
        store.apply("Font size", "21", true).unwrap();
        store.apply("Font size", "99", false).unwrap();
        assert_eq!(store.get("FontSize"), Some(&ConfigValue::Integer(21)));
        store.apply("Make autosaves", "-2", false).unwrap();
        assert_eq!(store.get("AutoSave"), Some(&ConfigValue::Boolean(true)));
        store.apply("BAR character 1", "β", false).unwrap();
        assert_eq!(store.get("BarChar1"), Some(&ConfigValue::Character('β')));
        let keys = store.iter().map(|(key, _)| key).collect::<Vec<_>>();
        assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
        let font = catalog()
            .into_iter()
            .find(|spec| spec.code == "FontSize")
            .unwrap();
        assert_eq!(font.clients, &[ConfigClient::Browser, ConfigClient::Tauri]);
        assert!(store.get("Drawing interface").is_none());
    }

    #[test]
    fn explicit_source_tracking_is_derived_and_not_serialized() {
        let document =
            ReraConfigDocument::parse("[meta]\nschema_version = 3\n\n[text]\nfont_size = 21\n")
                .unwrap();
        let store = document.values().unwrap();
        assert!(store.is_specified("FontSize"));
        let encoded = serde_json::to_value(&store).unwrap();
        assert!(encoded.get("specified").is_none());
        let decoded: ConfigStore = serde_json::from_value(encoded).unwrap();
        assert!(!decoded.is_specified("FontSize"));
    }

    #[test]
    fn menu_visibility_defaults_to_auto_and_accepts_legacy_boolean_values() {
        let allowed = vec!["SHOW".into(), "AUTO".into(), "HIDE".into()];
        let mut store = ConfigStore::with_web_defaults();
        assert_eq!(
            store.get_code("UseMenu"),
            Some(&ConfigValue::Enum {
                value: "AUTO".into(),
                allowed: allowed.clone(),
            })
        );

        for (input, expected) in [
            ("YES", "AUTO"),
            ("TRUE", "AUTO"),
            ("1", "AUTO"),
            ("NO", "HIDE"),
            ("FALSE", "HIDE"),
            ("0", "HIDE"),
            ("SHOW", "SHOW"),
        ] {
            store.apply("UseMenu", input, false).unwrap();
            assert_eq!(
                store.get_code("UseMenu"),
                Some(&ConfigValue::Enum {
                    value: expected.into(),
                    allowed: allowed.clone(),
                }),
                "failed to normalize {input}",
            );
        }
    }

    #[test]
    fn tui_profile_has_exact_surface_defaults_and_application_policies() {
        let exposed = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Tui))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let expected = [
            "AllowFunctionOverloading",
            "AllowLongInputByMouse",
            "AutoSave",
            "BackColor",
            "ButtonWrap",
            "CompatiCALLNAME",
            "CompatiCallEvent",
            "CompatiFuncArgAutoConvert",
            "CompatiFuncArgOptional",
            "CompatiLinefeedAs1739",
            "CompatiSPChara",
            "Ctrl_Z_Enabled",
            "DisplayWarningLevel",
            "FocusColor",
            "ForeColor",
            "FunctionNotCalledWarning",
            "FunctionNotFoundWarning",
            "IgnoreCase",
            "IgnoreUncalledFunction",
            "MaxLog",
            "PrintCLength",
            "PrintCPerLine",
            "ReplaceFullWidthSpaces",
            "ReplaceContinuationBR",
            "SaveDataNos",
            "SearchSubdirectory",
            "SortWithFilename",
            "SystemAllowFullSpace",
            "SystemIgnoreTripleSymbol",
            "SystemSaveInBinary",
            "UseERD",
            "UseMouse",
            "UseRenameFile",
            "UseReplaceFile",
            "VarsizeDimConfig",
            "WarnFunctionOverloading",
            "ZipSaveData",
            "CharacterWidthMode",
            "useLanguage",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(exposed, expected);

        let store = ConfigStore::with_tui_defaults();
        assert_eq!(store.get_code("MaxLog"), Some(&ConfigValue::Integer(1_000)));
        assert_eq!(
            store.get_code("PrintCPerLine"),
            Some(&ConfigValue::Integer(5))
        );
        assert_eq!(
            store.get_code("PrintCLength"),
            Some(&ConfigValue::Integer(24))
        );
        assert_eq!(
            expected
                .iter()
                .filter(|code| tui_application(code) == Some(ConfigApplication::Hot))
                .count(),
            13
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn browser_and_tauri_profiles_have_exact_surfaces_defaults_and_policies() {
        for spec in catalog() {
            assert_eq!(
                spec.clients.contains(&ConfigClient::Tui),
                tui_application(spec.code).is_some(),
                "TUI surface and policy drifted for {}",
                spec.code
            );
            assert_eq!(
                spec.clients.contains(&ConfigClient::Browser),
                browser_application(spec.code).is_some(),
                "browser surface and policy drifted for {}",
                spec.code
            );
            assert_eq!(
                spec.clients.contains(&ConfigClient::Tauri),
                tauri_application(spec.code).is_some(),
                "Tauri surface and policy drifted for {}",
                spec.code
            );
        }
        let browser = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Browser))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let expected = [
            "AllowFunctionOverloading",
            "AllowLongInputByMouse",
            "AudioVolume",
            "AutoSave",
            "BackColor",
            "ButtonWrap",
            "CompatiCALLNAME",
            "CompatiCallEvent",
            "CompatiFuncArgAutoConvert",
            "CompatiFuncArgOptional",
            "CompatiLinefeedAs1739",
            "CompatiSPChara",
            "Ctrl_Z_Enabled",
            "DisplayWarningLevel",
            "FocusColor",
            "FontName",
            "FontSize",
            "ForeColor",
            "FunctionNotCalledWarning",
            "FunctionNotFoundWarning",
            "IgnoreCase",
            "IgnoreUncalledFunction",
            "LineHeight",
            "MaxLog",
            "PrintCLength",
            "PrintCPerLine",
            "ReplaceFullWidthSpaces",
            "ReplaceContinuationBR",
            "SaveDataNos",
            "ScrollHeight",
            "SearchSubdirectory",
            "SortWithFilename",
            "SystemAllowFullSpace",
            "SystemIgnoreTripleSymbol",
            "SystemSaveInBinary",
            "UseERD",
            "UseMenu",
            "UseMouse",
            "UseRenameFile",
            "UseReplaceFile",
            "VarsizeDimConfig",
            "WarnFunctionOverloading",
            "ZipSaveData",
            "CharacterWidthMode",
            "useLanguage",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(browser, expected);

        let tauri = catalog()
            .into_iter()
            .filter(|spec| spec.clients.contains(&ConfigClient::Tauri))
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>();
        let mut expected_tauri = expected;
        expected_tauri.extend(["WindowMaximixed", "WindowX", "WindowY"]);
        assert_eq!(tauri, expected_tauri);
        assert_eq!(
            expected_tauri
                .iter()
                .filter(|code| tauri_application(code) == Some(ConfigApplication::Hot))
                .count(),
            22
        );

        let store = ConfigStore::with_web_defaults();
        assert_eq!(store.get_code("MaxLog"), Some(&ConfigValue::Integer(1_000)));
        assert_eq!(
            store.get_code("PrintCPerLine"),
            Some(&ConfigValue::Integer(5))
        );
        assert_eq!(
            store.get_code("PrintCLength"),
            Some(&ConfigValue::Integer(24))
        );
        for code in ["SizableWindow", "SetWindowPos", "WindowPosX", "WindowPosY"] {
            assert!(!tauri_configurable(code));
        }
    }
}
