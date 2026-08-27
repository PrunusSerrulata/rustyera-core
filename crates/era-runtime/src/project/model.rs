//! Normalized project state and client-profile configuration projections.

use era_config::{ConfigStore, ReraConfigDocument};
use era_runtime_protocol::{
    CONFIG_BROWSER, CONFIG_RUNTIME, CONFIG_TAURI, CONFIG_TUI, ConfigurationApplication,
    ConfigurationClientProfile, ConfigurationValueKind, FileCategory, ProjectConfigurationEntry,
    ProjectConfigurationSnapshot, ProjectManifest,
};
use erabasic_analyzer::AnalyzerOptions;
use erabasic_csv::CsvLoadOptions;
use erabasic_data::LegacyEncoding;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::resource::ResourceGraph;

#[derive(Clone)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct NormalizedProjectSnapshot {
    pub(crate) manifest: Arc<ProjectManifest>,
    pub(crate) project_identity: [u8; 32],
    pub(crate) resources: Vec<NormalizedResourceIdentity>,
    pub(crate) resource_graph: ResourceGraph,
    pub(crate) sort_with_filename: bool,
    pub(crate) auto_save: bool,
    pub(crate) ctrl_z_enabled: bool,
    pub(crate) allow_long_input_by_activation: bool,
    pub(crate) save_in_binary: bool,
    pub(crate) compress_save: bool,
    pub(crate) save_slot_count: u32,
    pub(crate) money_label: String,
    pub(crate) money_first: bool,
    pub(crate) maximum_shop_items: u32,
    pub(crate) viewport_width: u32,
    pub(crate) viewport_height: u32,
    pub(crate) font_size: u32,
    pub(crate) line_height: u32,
    pub(crate) print_c_per_line: u32,
    pub(crate) print_c_length: u32,
    pub(crate) configuration_profile: ConfigurationClientProfile,
    /// Complete query-visible configuration, including client-only compatibility values.
    pub(crate) configuration: ConfigStore,
    /// Client presentation projection after applying non-semantic preference layers.
    pub(crate) client_configuration: ConfigStore,
    /// Complete editable TOML values; the protocol applies each client's UI whitelist.
    pub(crate) editable_configuration: ConfigStore,
    pub(crate) configuration_document: ReraConfigDocument,
    pub(crate) configuration_source_digest: era_protocol::ProtocolBytes,
    pub(crate) generated_configuration_source: Option<String>,
    pub(crate) extensions:
        std::collections::BTreeMap<String, era_runtime_protocol::ExtensionDeclaration>,
}

impl NormalizedProjectSnapshot {
    pub(crate) fn configuration_snapshot(&self) -> ProjectConfigurationSnapshot {
        let entries = era_config::catalog()
            .into_iter()
            .filter(|spec| {
                era_config::is_regular_code(spec.code)
                    || matches!(
                        spec.code,
                        "AudioVolume" | "ReplaceFullWidthSpaces" | "CharacterWidthMode"
                    )
            })
            .filter_map(|spec| {
                let value = self.editable_configuration.get_code(spec.code)?;
                let effective = self.configuration.get_code(spec.code)?;
                let client_effective = self.client_configuration.get_code(spec.code)?;
                let applicability = protocol_applicability(spec.clients);
                let preference_eligible = spec.effect
                    == era_config::ConfigEffect::QueryOnlyClientPreference
                    && profile_preference_eligible(self.configuration_profile, spec.code);
                (applicability != 0).then(|| ProjectConfigurationEntry {
                    code: spec.code.into(),
                    japanese: spec.japanese.into(),
                    english: spec.english.into(),
                    value: value.config_text(),
                    kind: configuration_value_kind(value),
                    allowed: match value {
                        era_config::ConfigValue::Enum { allowed, .. } => allowed.clone(),
                        _ => Vec::new(),
                    },
                    fixed: self.editable_configuration.is_fixed(spec.code),
                    applicability,
                    default_value: profile_default(&spec, self.configuration_profile).config_text(),
                    effective_value: effective.config_text(),
                    application: profile_application(spec.code, self.configuration_profile),
                    preference_eligible,
                    client_effective_value: client_effective.config_text(),
                })
            })
            .collect::<Vec<_>>();
        let restart_pending = entries
            .iter()
            .any(|entry| entry.value != entry.effective_value);
        ProjectConfigurationSnapshot {
            compatibility: self.manifest.compatibility.clone(),
            project_revision: self.manifest.project_revision,
            source_digest: self.configuration_source_digest.clone(),
            entries,
            restart_pending,
            generated_source: self
                .generated_configuration_source
                .clone()
                .map(String::into_boxed_str),
        }
    }
}

fn profile_default(
    spec: &era_config::ConfigSpec,
    profile: ConfigurationClientProfile,
) -> era_config::ConfigValue {
    let override_value = match profile {
        ConfigurationClientProfile::Tui => era_config::tui_default(spec.code),
        ConfigurationClientProfile::Browser | ConfigurationClientProfile::Tauri => {
            era_config::web_default(spec.code)
        }
        ConfigurationClientProfile::Reference => None,
    };
    override_value.unwrap_or_else(|| spec.default.clone())
}

pub(crate) fn profile_application(
    code: &str,
    profile: ConfigurationClientProfile,
) -> ConfigurationApplication {
    let application = match profile {
        ConfigurationClientProfile::Tui => era_config::tui_application(code),
        ConfigurationClientProfile::Browser => era_config::browser_application(code),
        ConfigurationClientProfile::Tauri => era_config::tauri_application(code),
        ConfigurationClientProfile::Reference => None,
    };
    if application == Some(era_config::ConfigApplication::Hot) {
        ConfigurationApplication::Hot
    } else {
        ConfigurationApplication::Restart
    }
}

pub(crate) fn profile_preference_eligible(profile: ConfigurationClientProfile, code: &str) -> bool {
    let browser = matches!(
        code,
        "UseMenu"
            | "UseMouse"
            | "ScrollHeight"
            | "ButtonWrap"
            | "FontName"
            | "FontSize"
            | "LineHeight"
            | "ForeColor"
            | "BackColor"
            | "FocusColor"
            | "AudioVolume"
            | "ReplaceFullWidthSpaces"
    );
    match profile {
        ConfigurationClientProfile::Tui => matches!(
            code,
            "UseMouse"
                | "ButtonWrap"
                | "ForeColor"
                | "BackColor"
                | "FocusColor"
                | "ReplaceFullWidthSpaces"
        ),
        ConfigurationClientProfile::Browser => browser,
        ConfigurationClientProfile::Tauri => {
            browser || matches!(code, "WindowMaximixed" | "WindowX" | "WindowY")
        }
        ConfigurationClientProfile::Reference => false,
    }
}

pub(crate) fn profile_applicability(profile: ConfigurationClientProfile) -> Option<u32> {
    match profile {
        ConfigurationClientProfile::Reference => None,
        ConfigurationClientProfile::Tui => Some(CONFIG_TUI),
        ConfigurationClientProfile::Browser => Some(CONFIG_BROWSER),
        ConfigurationClientProfile::Tauri => Some(CONFIG_TAURI),
    }
}

fn protocol_applicability(clients: &[era_config::ConfigClient]) -> u32 {
    use era_config::ConfigClient;
    clients.iter().fold(0, |flags, client| {
        flags
            | match client {
                ConfigClient::Runtime => CONFIG_RUNTIME,
                ConfigClient::Tui => CONFIG_TUI,
                ConfigClient::Browser => CONFIG_BROWSER,
                ConfigClient::Tauri => CONFIG_TAURI,
            }
    })
}

fn configuration_value_kind(value: &era_config::ConfigValue) -> ConfigurationValueKind {
    use era_config::ConfigValue;
    match value {
        ConfigValue::Boolean(_) => ConfigurationValueKind::Boolean,
        ConfigValue::Integer(_) => ConfigurationValueKind::Integer,
        ConfigValue::String(_) => ConfigurationValueKind::String,
        ConfigValue::Enum { .. } => ConfigurationValueKind::Enum,
        ConfigValue::Color(_) => ConfigurationValueKind::Color,
        ConfigValue::Character(_) => ConfigurationValueKind::Character,
        ConfigValue::IntegerList(_) => ConfigurationValueKind::IntegerList,
        ConfigValue::StringList(_) => ConfigurationValueKind::StringList,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct NormalizedResourceIdentity {
    pub(crate) relative_path: String,
    pub(crate) category: FileCategory,
    pub(crate) payload_digest: [u8; 32],
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct SemanticConfig {
    pub(super) values: ConfigStore,
    pub(super) csv: CsvLoadOptions,
    pub(super) analyzer: AnalyzerOptions,
    pub(super) auto_save: bool,
    pub(super) ctrl_z_enabled: bool,
    pub(super) allow_long_input_by_activation: bool,
    pub(super) save_in_binary: bool,
    pub(super) compress_save: bool,
    pub(super) save_slot_count: u32,
    pub(super) money_label: String,
    pub(super) money_first: bool,
    pub(super) maximum_shop_items: u32,
    pub(super) viewport_width: u32,
    pub(super) viewport_height: u32,
    pub(super) font_size: u32,
    pub(super) line_height: u32,
    pub(super) print_c_per_line: u32,
    pub(super) print_c_length: u32,
    pub(super) legacy_encoding: LegacyEncoding,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            values: ConfigStore::default(),
            csv: CsvLoadOptions::default(),
            analyzer: AnalyzerOptions::default(),
            auto_save: true,
            ctrl_z_enabled: false,
            allow_long_input_by_activation: false,
            save_in_binary: false,
            compress_save: false,
            save_slot_count: 20,
            money_label: "$".into(),
            money_first: true,
            maximum_shop_items: 100,
            viewport_width: 760,
            viewport_height: 480,
            font_size: 18,
            line_height: 19,
            print_c_per_line: 3,
            print_c_length: 25,
            legacy_encoding: LegacyEncoding::Japanese,
        }
    }
}
