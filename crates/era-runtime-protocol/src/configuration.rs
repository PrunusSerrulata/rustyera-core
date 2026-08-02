use era_protocol::ProtocolBytes;
use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationValueKind {
    #[n(0)]
    Boolean,
    #[n(1)]
    Integer,
    #[n(2)]
    String,
    #[n(3)]
    Enum,
    #[n(4)]
    Color,
    #[n(5)]
    Character,
    #[n(6)]
    IntegerList,
    #[n(7)]
    StringList,
}

/// Client applicability flags for an effective configuration entry.
pub const CONFIG_RUNTIME: u32 = 1;
pub const CONFIG_TUI: u32 = 1 << 1;
pub const CONFIG_BROWSER: u32 = 1 << 2;
pub const CONFIG_TAURI: u32 = 1 << 3;

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectConfigurationEntry {
    #[n(0)]
    pub code: String,
    #[n(1)]
    pub japanese: String,
    #[n(2)]
    pub english: String,
    /// Canonical config-file spelling of the effective value.
    #[n(3)]
    pub value: String,
    #[n(4)]
    pub kind: ConfigurationValueKind,
    #[n(5)]
    pub allowed: Vec<String>,
    #[n(6)]
    pub fixed: bool,
    #[n(7)]
    pub applicability: u32,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ProjectConfigurationSnapshot {
    #[n(0)]
    pub project_revision: u64,
    /// BLAKE3 of the submitted root emuera.config, or an empty byte string if absent.
    #[n(1)]
    pub source_digest: ProtocolBytes,
    #[n(2)]
    pub entries: Vec<ProjectConfigurationEntry>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ConfigurationChange {
    #[n(0)]
    pub code: String,
    /// Config-file syntax, validated against the catalog type.
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct PrepareConfigurationUpdate {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub expected_source_digest: ProtocolBytes,
    #[n(2)]
    pub changes: Vec<ConfigurationChange>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct ConfigurationUpdatePrepared {
    #[n(0)]
    pub project_revision: u64,
    #[n(1)]
    pub expected_source_digest: ProtocolBytes,
    #[n(2)]
    pub contents: String,
    #[n(3)]
    pub restart_required: bool,
}
