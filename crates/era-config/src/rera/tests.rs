use std::collections::BTreeSet;

use crate::{ConfigStore, ConfigValue};

use super::*;

#[test]
fn catalog_has_explicit_unique_ids_paths_codes_and_unified_defaults() {
    catalog::validate_catalog();
    let specs = rera_catalog();
    assert_eq!(specs.len(), 127);
    let mappings = specs
        .iter()
        .map(|spec| (spec.code, spec.id, spec.path))
        .collect::<Vec<_>>();
    for expected in [
        ("IgnoreCase", 1, "script.ignore_case"),
        ("MaxLog", 18, "output.history_lines"),
        ("EditorArgument", 49, "editor.arguments"),
        ("UseNewRandom", 124, "legacy.use_new_random"),
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
    assert_eq!(RERACONFIG_SCHEMA_VERSION, 1);
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
        "[text]\nfont_size = 20\n",
        "\u{feff}[text]\r\nfont_size = 20\r\n",
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
        "[text]\n# before\nfont_size = 20 # inline\n\n# adjacent\nline_height = 21\n",
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
        "[meta]\nlocked_settings = [\"text.font_size\"] # locked\n\n[text]\nfont_size = 20\n",
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
    let input = "\u{feff}[audio]\r\nvolume = 101\r\n";
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
    assert!(
        migration.diagnostics.is_empty(),
        "{:?}",
        migration.diagnostics
    );
    assert_eq!(
        migration.values.get_code("FontSize"),
        Some(&ConfigValue::Integer(21))
    );
    assert!(migration.values.is_fixed("FontSize"));
    assert_eq!(
        migration.values.get_code("UseNewRandom"),
        Some(&ConfigValue::Boolean(true))
    );
    assert_eq!(
        migration.values.get_code("MoneyLabel"),
        Some(&ConfigValue::String("円".into()))
    );
    assert_eq!(migration.document.values().unwrap(), migration.values);
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
    assert_eq!(
        migration.values.get_code("DebugWindowWidth"),
        Some(&ConfigValue::Integer(640))
    );
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
            LegacyMigrationDiagnosticKind::UnknownSetting,
        ]
    );
    assert!(
        migration
            .diagnostics
            .iter()
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
    assert_eq!(
        migration.values.get_code("CompatiDRAWLINE"),
        Some(&ConfigValue::Boolean(false))
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
fn legacy_editor_argument_preserves_spaces_and_reference_colon_behavior() {
    let migration = migrate_legacy_configuration(&[LegacyConfigSource {
        relative_path: "emuera.config",
        contents: "Text editor command line arguments:  --line :ignored\n",
    }]);
    assert!(migration.diagnostics.is_empty());
    assert_eq!(
        migration.values.get_code("EditorArgument"),
        Some(&ConfigValue::String("  --line ".into()))
    );
    assert_eq!(migration.document.values().unwrap(), migration.values);
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
