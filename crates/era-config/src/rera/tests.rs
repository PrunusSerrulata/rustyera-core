use std::collections::BTreeSet;

use crate::{ConfigStore, ConfigValue};

use super::*;

#[test]
fn catalog_has_explicit_unique_ids_paths_codes_and_unified_defaults() {
    catalog::validate_catalog();
    let specs = rera_catalog();
    assert_eq!(specs.len(), 87);
    let mappings = specs
        .iter()
        .map(|spec| (spec.code, spec.id, spec.path))
        .collect::<Vec<_>>();
    for expected in [
        ("IgnoreCase", 1, "script.ignore_case"),
        ("MaxLog", 18, "output.history_lines"),
        ("AudioVolume", 125, "audio.volume"),
        (
            "ReplaceFullWidthSpaces",
            126,
            "text.replace_full_width_spaces",
        ),
        ("CharacterWidthMode", 127, "text.character_width_mode"),
    ] {
        assert!(
            mappings.contains(&expected),
            "missing pinned mapping {expected:?}"
        );
    }
    assert_eq!(RERACONFIG_SCHEMA_VERSION, 4);
    assert_eq!(
        ConfigStore::default().get_code("UseMenu"),
        Some(&ConfigValue::Enum {
            value: "AUTO".into(),
            allowed: vec!["SHOW".into(), "AUTO".into(), "HIDE".into()],
        })
    );
    let active_ids = specs.iter().map(|spec| spec.id).collect::<BTreeSet<_>>();
    assert_eq!(retired::RETIRED_CONFIG_SPECS.len(), 40);
    assert_eq!(
        retired::RETIRED_CONFIG_SPECS
            .iter()
            .map(|spec| spec.code)
            .collect::<BTreeSet<_>>()
            .len(),
        40
    );
    assert_eq!(
        retired::RETIRED_CONFIG_SPECS
            .iter()
            .map(|spec| spec.path)
            .collect::<BTreeSet<_>>()
            .len(),
        40
    );
    assert_eq!(
        retired::RETIRED_CONFIG_SPECS
            .iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>()
            .len(),
        40
    );
    assert!(
        retired::RETIRED_CONFIG_SPECS
            .iter()
            .all(|spec| !active_ids.contains(&spec.id)),
        "retired stable IDs must remain reserved"
    );
    for retired in [
        "TextDrawingMode",
        "EnglishConfigOutput",
        "TextEditor",
        "RikaiEnabled",
        "CBUseClipboard",
        "DebugShowWindow",
        "UseNewRandom",
    ] {
        assert!(specs.iter().all(|spec| spec.code != retired));
    }
    assert_eq!(
        ConfigStore::default().get_code("MaxLog"),
        Some(&ConfigValue::Integer(1000))
    );
    assert_eq!(
        ConfigStore::default().get_code("PrintCPerLine"),
        Some(&ConfigValue::Integer(5))
    );
    assert_eq!(
        ConfigStore::default().get_code("PrintCLength"),
        Some(&ConfigValue::Integer(24))
    );
    assert_eq!(
        ConfigStore::default().get_code("AudioVolume"),
        Some(&ConfigValue::Integer(100))
    );
}

#[test]
fn missing_fields_bom_and_line_endings_use_defaults() {
    for input in [
        "[meta]\nschema_version = 3\n[text]\nfont_size = 20\n",
        "\u{feff}[meta]\r\nschema_version = 3\r\n[text]\r\nfont_size = 20\r\n",
    ] {
        let document = ReraConfigDocument::parse(input).unwrap();
        let values = document.values().unwrap();
        assert_eq!(values.get_code("FontSize"), Some(&ConfigValue::Integer(20)));
        assert_eq!(values.get_code("MaxLog"), Some(&ConfigValue::Integer(1000)));
    }
}

#[test]
fn editing_preserves_surrounding_and_inline_comments() {
    let mut document = ReraConfigDocument::parse(
        "[meta]\nschema_version = 3\n\n[text]\n# before\nfont_size = 20 # inline\n\n# adjacent\nline_height = 21\n",
    )
    .unwrap();
    document
        .set_code("FontSize", &ConfigValue::Integer(22))
        .unwrap();
    let output = document.to_lf_string();
    assert!(output.contains("# before\nfont_size = 22 # inline"));
    assert!(output.contains("\n\n# adjacent\nline_height = 21"));
    assert!(!output.contains('\r'));
}

#[test]
fn locked_settings_cannot_be_changed_and_lock_comments_survive_updates() {
    let mut document = ReraConfigDocument::parse(
        "[meta]\nschema_version = 3\nlocked_settings = [\"text.font_size\"] # locked\n\n[text]\nfont_size = 20\n",
    )
    .unwrap();
    let error = document
        .set_code("FontSize", &ConfigValue::Integer(22))
        .unwrap_err();
    assert_eq!(error.kind, ReraConfigErrorKind::LockedSetting);
    document
        .set_locked_codes(["LineHeight".to_owned()])
        .unwrap();
    assert!(
        document
            .to_lf_string()
            .contains("locked_settings = [\"text.line_height\"] # locked")
    );
}

#[test]
fn strict_validation_reports_stable_kind_and_utf8_byte_span() {
    let input = "\u{feff}[meta]\r\nschema_version = 3\r\n[audio]\r\nvolume = 101\r\n";
    let error = ReraConfigDocument::parse(input).unwrap_err();
    assert_eq!(error.kind, ReraConfigErrorKind::OutOfRange);
    assert_eq!(error.path.as_deref(), Some("audio.volume"));
    let span = error.span.expect("field error has a source span");
    assert_eq!(&input.as_bytes()[span.start..span.end], b"101");

    for invalid in [
        "[unknown]\nvalue = true\n",
        "[text]\ncharacter_width_mode = \"other\"\n",
        "text = { font_size = 20 }\n",
        "text.font_size = 20\n",
        "[meta]\nschema_version = 3\n[text]\ndrawing_method = \"textrenderer\"\n",
    ] {
        assert!(
            ReraConfigDocument::parse(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
}

#[test]
fn legacy_migration_merges_precedence_locks_json_and_replace() {
    let migration = migrate_legacy_configuration(&[
        LegacyConfigSource {
            relative_path: "CSV/_default.config",
            contents: "Font size:19\n",
        },
        LegacyConfigSource {
            relative_path: "emuera.config",
            contents: "Font size:20\nUse _Replace.csv file:YES\n",
        },
        LegacyConfigSource {
            relative_path: "setting.json",
            contents: r#"{"UseNewRandom":true}"#,
        },
        LegacyConfigSource {
            relative_path: "CSV/_fixed.config",
            contents: "Font size:21\n",
        },
        LegacyConfigSource {
            relative_path: "debug.config",
            contents: "Debug window width:640\n",
        },
        LegacyConfigSource {
            relative_path: "CSV/_Replace.csv",
            contents: "MONEYLABEL,円\nPALAMLVの初期値,0/10/20\n",
        },
    ]);
    assert_eq!(
        migration
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect::<Vec<_>>(),
        vec![
            LegacyMigrationDiagnosticKind::RetiredSettingIgnored,
            LegacyMigrationDiagnosticKind::RetiredSettingIgnored,
        ]
    );
    assert_eq!(
        migration.values.get_code("FontSize"),
        Some(&ConfigValue::Integer(21))
    );
    assert!(migration.values.is_fixed("FontSize"));
    assert!(migration.values.get_code("UseNewRandom").is_none());
    assert_eq!(
        migration.values.get_code("MoneyLabel"),
        Some(&ConfigValue::String("円".into()))
    );
    assert_eq!(migration.document.values().unwrap(), migration.values);
}

#[test]
fn legacy_menu_boolean_migration_records_the_intentional_client_difference() {
    let automatic = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "emuera.config",
        contents: "Show menu:YES\n",
    }]);
    assert_eq!(
        automatic.values.get_code("UseMenu"),
        Some(&ConfigValue::Enum {
            value: "AUTO".into(),
            allowed: vec!["SHOW".into(), "AUTO".into(), "HIDE".into()],
        })
    );
    assert!(!automatic.document.to_lf_string().contains("menu_mode"));

    let hidden = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "emuera.config",
        contents: "Show menu:NO\n",
    }]);
    assert_eq!(
        hidden.values.get_code("UseMenu"),
        Some(&ConfigValue::Enum {
            value: "HIDE".into(),
            allowed: vec!["SHOW".into(), "AUTO".into(), "HIDE".into()],
        })
    );
    assert!(
        hidden
            .document
            .to_lf_string()
            .contains("menu_mode = \"hide\"")
    );
}

#[test]
fn legacy_sources_cannot_modify_settings_outside_their_own_catalog() {
    let migration = migrate_legacy_configuration(&[
        LegacyConfigSource {
            relative_path: "debug.config",
            contents: "Font size:40\nDebug window width:640\n",
        },
        LegacyConfigSource {
            relative_path: "_Replace.csv",
            contents: "Font size,40\nMONEYLABEL,円\n",
        },
        LegacyConfigSource {
            relative_path: "plugin.config",
            contents: "Font size:99\n",
        },
    ]);
    assert_eq!(
        migration.values.get_code("FontSize"),
        Some(&ConfigValue::Integer(18))
    );
    assert!(migration.values.get_code("DebugWindowWidth").is_none());
    assert_eq!(
        migration.values.get_code("MoneyLabel"),
        Some(&ConfigValue::String("円".into()))
    );
    assert_eq!(
        migration
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect::<Vec<_>>(),
        vec![
            LegacyMigrationDiagnosticKind::UnknownSetting,
            LegacyMigrationDiagnosticKind::RetiredSettingIgnored,
            LegacyMigrationDiagnosticKind::UnknownSetting,
        ]
    );
    assert!(
        migration
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.kind == LegacyMigrationDiagnosticKind::UnknownSetting)
            .all(|diagnostic| diagnostic.span.is_some())
    );
}

#[test]
fn deprecated_drawline_alias_moves_value_and_lock_to_supported_setting() {
    let migration = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "CSV/_fixed.config",
        contents: "Always start DRAWLINE in a new line:YES\r\n",
    }]);
    assert!(migration.diagnostics.is_empty());
    assert_eq!(
        migration.values.get_code("CompatiLinefeedAs1739"),
        Some(&ConfigValue::Boolean(true))
    );
    assert!(migration.values.is_fixed("CompatiLinefeedAs1739"));
    assert!(migration.values.get_code("CompatiDRAWLINE").is_none());
}

#[test]
fn debug_config_cannot_change_the_drawline_compatibility_setting() {
    let migration = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "debug.config",
        contents: "Always start DRAWLINE in a new line:YES\n",
    }]);
    assert_eq!(
        migration.values.get_code("CompatiLinefeedAs1739"),
        Some(&ConfigValue::Boolean(false))
    );
    assert_eq!(migration.diagnostics.len(), 1);
    assert_eq!(
        migration.diagnostics[0].kind,
        LegacyMigrationDiagnosticKind::RetiredSettingIgnored
    );
}

#[test]
fn legacy_narrow_ranges_are_normalized_and_document_matches_effective_values() {
    let migration = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "emuera.config",
        contents: concat!(
            "Max history log lines:100\n",
            "Window width:1\n",
            "Font size:1\n",
            "Save data count per page:999\n",
        ),
    }]);
    assert!(migration.diagnostics.is_empty());
    for (code, expected) in [
        ("MaxLog", 500),
        ("WindowX", 128),
        ("FontSize", 8),
        ("SaveDataNos", 80),
    ] {
        assert_eq!(
            migration.values.get_code(code),
            Some(&ConfigValue::Integer(expected))
        );
    }
    assert_eq!(migration.document.values().unwrap(), migration.values);
}

#[test]
fn retired_legacy_editor_settings_are_reported_and_not_migrated() {
    let migration = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "emuera.config",
        contents: "Text editor command line arguments:  --line :ignored\n",
    }]);
    assert_eq!(migration.diagnostics.len(), 1);
    assert_eq!(
        migration.diagnostics[0].kind,
        LegacyMigrationDiagnosticKind::RetiredSettingIgnored
    );
    assert!(migration.values.get_code("EditorArgument").is_none());
    assert_eq!(migration.document.values().unwrap(), migration.values);
}

#[test]
fn schema_v1_is_upgraded_and_retired_locks_and_fields_are_removed() {
    let document = ReraConfigDocument::parse(
        "[meta]\nschema_version = 1\nlocked_settings = [\"text.drawing_method\", \"compatibility.drawline_starts_new_line\", \"text.font_size\"]\n\n[text]\ndrawing_method = \"winapi\"\nfont_size = 20 # keep\n\n[compatibility]\ndrawline_starts_new_line = true\n",
    )
    .unwrap();
    assert!(document.was_upgraded());
    assert_eq!(
        document.retired_codes(),
        &["TextDrawingMode", "CompatiDRAWLINE"]
    );
    let output = document.to_lf_string();
    assert!(output.contains("schema_version = 4"));
    assert!(output.contains("font_size = 20 # keep"));
    assert!(output.contains("legacy_nonbutton_wrapping = true"));
    assert!(!output.contains("drawing_method"));
    assert!(!output.contains("drawline_starts_new_line"));
    let values = document.values().unwrap();
    assert!(values.is_fixed("FontSize"));
    assert!(values.is_fixed("CompatiLinefeedAs1739"));
}

#[test]
fn schema_v2_menu_visibility_is_upgraded_to_menu_mode() {
    for (visible, expected) in [(true, "AUTO"), (false, "HIDE")] {
        let document = ReraConfigDocument::parse(&format!(
            "[meta]\nschema_version = 2\nlocked_settings = [\"interface.menu_visible\"]\n\n[interface]\nmenu_visible = {visible} # keep\n",
        ))
        .unwrap();
        assert!(document.was_upgraded());
        assert_eq!(
            document.values().unwrap().get_code("UseMenu"),
            Some(&ConfigValue::Enum {
                value: expected.into(),
                allowed: vec!["SHOW".into(), "AUTO".into(), "HIDE".into()],
            })
        );
        assert!(document.values().unwrap().is_fixed("UseMenu"));
        let output = document.to_lf_string();
        assert!(output.contains("schema_version = 4"));
        assert!(output.contains(&format!(
            "menu_mode = \"{}\" # keep",
            expected.to_lowercase()
        )));
        assert!(output.contains("locked_settings = [\"interface.menu_mode\"]"));
        assert!(!output.contains("menu_visible"));
    }

    let defaults = ReraConfigDocument::parse("[meta]\nschema_version = 2\n")
        .unwrap()
        .values()
        .unwrap();
    assert_eq!(
        defaults.get_code("UseMenu"),
        Some(&ConfigValue::Enum {
            value: "AUTO".into(),
            allowed: vec!["SHOW".into(), "AUTO".into(), "HIDE".into()],
        })
    );
}

#[test]
fn canonical_materialization_rejects_values_from_an_old_catalog_type() {
    let mut serialized = serde_json::to_value(ConfigStore::default()).unwrap();
    serialized["values"]["USEMENU"] = serde_json::json!({ "Boolean": true });
    let stale: ConfigStore = serde_json::from_value(serialized).unwrap();

    let error = ReraConfigDocument::from_values(&stale).unwrap_err();
    assert_eq!(error.kind, ReraConfigErrorKind::InvalidType);
    assert_eq!(error.path.as_deref(), Some("interface.menu_mode"));

    let current = ConfigStore::default();
    assert_eq!(
        ReraConfigDocument::from_values(&current)
            .unwrap()
            .values()
            .unwrap(),
        current
    );
}

#[test]
fn schema_v2_menu_upgrade_preserves_unrelated_lock_formatting() {
    let input = "[meta]\nschema_version = 2\nlocked_settings = [\n  \"text.font_size\", # font\n  \"interface.menu_visible\", # menu\n  \"input.mouse_enabled\", # mouse\n] # locks\n\n[interface]\nmenu_visible = true # value\n";
    let expected = input
        .replace("schema_version = 2", "schema_version = 4")
        .replace("interface.menu_visible", "interface.menu_mode")
        .replace("menu_visible = true", "menu_mode = \"auto\"");
    let document = ReraConfigDocument::parse(input).unwrap();
    assert_eq!(document.to_lf_string(), expected);
}

#[test]
fn schema_v1_upgrade_keeps_metadata_validation_strict() {
    for input in [
        "[meta]\nschema_version = 1\nlocked_settings = [1]\n",
        "[meta]\nschema_version = 1\nlocked_settings = [\"text.font_size\", \"text.font_size\"]\n",
        "[meta]\nschema_version = 1\nunknown = true\n",
    ] {
        assert!(
            ReraConfigDocument::parse(input).is_err(),
            "accepted invalid version 1 metadata: {input:?}"
        );
    }

    let error = ReraConfigDocument::parse("compatibility = { drawline_starts_new_line = true }\n")
        .unwrap_err();
    assert_eq!(error.kind, ReraConfigErrorKind::UnsupportedStructure);
}

#[test]
fn every_retired_setting_is_upgraded_rejected_and_recognized_by_legacy_aliases() {
    for spec in retired::RETIRED_CONFIG_SPECS {
        let (section, key) = spec.path.split_once('.').unwrap();
        let version_1 = format!("[meta]\nschema_version = 1\n\n[{section}]\n{key} = true\n");
        let upgraded = ReraConfigDocument::parse(&version_1).unwrap();
        assert!(upgraded.retired_codes().contains(&spec.code));

        let version_2 = format!("[meta]\nschema_version = 2\n\n[{section}]\n{key} = true\n");
        assert_eq!(
            ReraConfigDocument::parse(&version_2).unwrap_err().kind,
            ReraConfigErrorKind::UnknownField,
            "schema version 2 accepted {}",
            spec.code
        );

        if spec.code == "CompatiDRAWLINE" {
            continue;
        }
        for alias in [spec.code, spec.japanese, spec.english] {
            let contents = format!("{alias}:ignored\n");
            let migration = migrate_legacy_configuration(&[LegacyConfigSource {
                relative_path: "emuera.config",
                contents: &contents,
            }]);
            assert!(migration.diagnostics.iter().any(|diagnostic| {
                diagnostic.kind == LegacyMigrationDiagnosticKind::RetiredSettingIgnored
                    && diagnostic.message.contains(spec.code)
            }));
        }
    }
}

#[test]
fn generated_artifacts_are_current_deterministic_and_document_every_setting() {
    let schema = generate_json_schema();
    let example = generate_annotated_example();
    let parsed: serde_json::Value = serde_json::from_str(&schema).unwrap();
    let properties = parsed["properties"].as_object().unwrap();
    for spec in rera_catalog() {
        let (section, key) = spec.path.split_once('.').unwrap();
        assert!(
            properties[section]["properties"].get(key).is_some(),
            "missing {}",
            spec.path
        );
        assert!(example.contains(&format!("{key} = ")));
    }
    assert_eq!(
        specs_as_ids().len(),
        rera_catalog()
            .iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>()
            .len()
    );
    assert_eq!(schema, include_str!("../../schema/reraconfig.schema.json"));
    assert_eq!(
        example,
        include_str!("../../schema/reraconfig.example.toml")
    );
}

fn specs_as_ids() -> BTreeSet<u16> {
    rera_catalog().into_iter().map(|spec| spec.id).collect()
}

#[test]
fn compatibility_profile_is_strict_and_survives_canonicalization() {
    use erabasic_compat::CompatibilityProfileId;
    let source = "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n";
    let document = ReraConfigDocument::parse(source).unwrap();
    let values = document.values().unwrap();
    assert_eq!(
        values.compatibility_profile(),
        CompatibilityProfileId::EmueraSkiaSnake
    );
    let rebuilt = ReraConfigDocument::from_values(&values).unwrap();
    assert_eq!(
        rebuilt.values().unwrap().compatibility_profile(),
        values.compatibility_profile()
    );
    for invalid in [
        source.replace("emuera.skia.snake", "snake"),
        source.replace("schema_version = 4", "schema_version = 3"),
        source.replace("\"emuera.skia.snake\"", "true"),
    ] {
        assert!(ReraConfigDocument::parse(&invalid).is_err());
    }
    assert_eq!(
        ReraConfigDocument::parse("[meta]\nschema_version = 3\n")
            .unwrap()
            .values()
            .unwrap()
            .compatibility_profile(),
        CompatibilityProfileId::EmueraEm
    );
}
