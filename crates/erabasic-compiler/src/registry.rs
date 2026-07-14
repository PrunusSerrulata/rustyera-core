use std::collections::BTreeMap;

use erabasic_bytecode::{HostCapability, HostEffect, HostSnapshotCapability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostBinding {
    pub namespace: String,
    pub name: String,
    pub abi_version: u32,
    pub effect: HostEffect,
    pub capability: HostCapability,
    pub snapshot_capability: HostSnapshotCapability,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HostRegistry {
    overrides: BTreeMap<String, HostBinding>,
}

impl HostRegistry {
    pub fn register(&mut self, era_name: impl Into<String>, binding: HostBinding) -> bool {
        self.overrides
            .insert(era_name.into().to_ascii_uppercase(), binding)
            .is_none()
    }

    #[must_use]
    pub fn resolve(&self, era_name: &str) -> Option<HostBinding> {
        let key = era_name.to_ascii_uppercase();
        self.overrides
            .get(&key)
            .cloned()
            .or_else(|| default_binding(&key))
    }
}

#[must_use]
pub fn default_host_registry() -> HostRegistry {
    HostRegistry::default()
}

fn default_binding(name: &str) -> Option<HostBinding> {
    let (namespace, operation, capability, may_suspend, mutates_runtime) = if name
        .starts_with("PRINT")
        || name.starts_with("DEBUGPRINT")
        || name.starts_with("HTML_PRINT")
        || matches!(name, "DRAWLINE" | "CLEARLINE" | "REUSELASTLINE")
    {
        ("rustyera.text", name, HostCapability::Text, false, true)
    } else if matches!(name, "GETTIME" | "GETTIMES" | "GETMILLISECOND") {
        ("rustyera.clock", name, HostCapability::Clock, false, false)
    } else if name.starts_with('G') || name.contains("SPRITE") || name.contains("BGIMAGE") {
        (
            "rustyera.graphics",
            name,
            HostCapability::Graphics,
            false,
            true,
        )
    } else if name.contains("SOUND") || name.contains("BGM") {
        ("rustyera.audio", name, HostCapability::Audio, false, true)
    } else if name.contains("INPUT")
        || matches!(
            name,
            "WAIT" | "WAITANYKEY" | "TWAIT" | "AWAIT" | "FORCEWAIT"
        )
    {
        ("rustyera.input", name, HostCapability::Input, true, true)
    } else if name.contains("SAVE")
        || name.contains("LOAD")
        || name.contains("FILE")
        || name.contains("TEXT")
    {
        (
            "rustyera.storage",
            name,
            HostCapability::Storage,
            true,
            true,
        )
    } else {
        return None;
    };
    Some(HostBinding {
        namespace: namespace.into(),
        name: operation.to_ascii_lowercase(),
        abi_version: 1,
        effect: HostEffect {
            pure: !mutates_runtime,
            may_suspend,
            may_error: true,
            mutates_runtime,
        },
        capability,
        snapshot_capability: if may_suspend && matches!(name, "TWAIT" | "AWAIT" | "FORCEWAIT") {
            HostSnapshotCapability::Never
        } else {
            HostSnapshotCapability::StableWait
        },
    })
}

pub(crate) fn extension_binding(name: &str) -> HostBinding {
    HostBinding {
        namespace: "rustyera.extension".into(),
        name: name.to_ascii_lowercase(),
        abi_version: 1,
        effect: HostEffect {
            pure: false,
            may_suspend: true,
            may_error: true,
            mutates_runtime: true,
        },
        capability: HostCapability::Extension,
        snapshot_capability: HostSnapshotCapability::Never,
    }
}
