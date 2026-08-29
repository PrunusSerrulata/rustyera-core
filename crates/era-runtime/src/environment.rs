//! One capability truth source, populated only from negotiated records.
use era_protocol::{ProtocolVersion, VersionRange, negotiate_version};
use era_runtime_protocol::{
    DEVICE_PUMP_OPERATION, DEVICE_PUMP_OPERATION_VERSION, EnvironmentCapability,
    INPUT_DEVICE_LATCH_CAPABILITY, INPUT_DEVICE_PUMP_CAPABILITY, INPUT_ENVIRONMENT_VERSION,
    INPUT_MACROS_CAPABILITY, INPUT_SEQUENCE_CAPABILITY, INPUT_TIMED_VIEWPORT_CAPABILITY,
    ServiceCapability, ServiceKind,
};
use std::collections::BTreeMap;

pub(crate) fn select_environment(
    client: &[EnvironmentCapability],
    services: &[ServiceCapability],
) -> Vec<EnvironmentCapability> {
    let mut selected = BTreeMap::new();
    for capability in client {
        if !matches!(
            capability.name.as_str(),
            INPUT_TIMED_VIEWPORT_CAPABILITY
                | INPUT_DEVICE_LATCH_CAPABILITY
                | INPUT_DEVICE_PUMP_CAPABILITY
        ) {
            continue;
        }
        let Some(version) = negotiate_version(
            capability.versions,
            VersionRange::exact(INPUT_ENVIRONMENT_VERSION),
        ) else {
            continue;
        };
        if capability.name == INPUT_DEVICE_PUMP_CAPABILITY
            && !services.iter().any(|service| {
                service.kind == ServiceKind::InputState
                    && service.operation == DEVICE_PUMP_OPERATION
                    && service.versions == VersionRange::exact(DEVICE_PUMP_OPERATION_VERSION)
            })
        {
            continue;
        }
        selected.insert(capability.name.clone(), version);
    }
    // These are runtime behavior, never inferred from a frontend modality.
    selected.insert(INPUT_SEQUENCE_CAPABILITY.into(), INPUT_ENVIRONMENT_VERSION);
    selected.insert(INPUT_MACROS_CAPABILITY.into(), INPUT_ENVIRONMENT_VERSION);
    selected
        .into_iter()
        .map(|(name, version)| EnvironmentCapability {
            name,
            versions: VersionRange::exact(version),
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Environment {
    selected: BTreeMap<String, ProtocolVersion>,
}
impl Environment {
    pub(crate) fn from_selected(values: &[EnvironmentCapability]) -> Self {
        Self {
            selected: values
                .iter()
                .map(|value| (value.name.clone(), value.versions.maximum))
                .collect(),
        }
    }
    pub(crate) fn has(&self, name: &str, major: i64) -> bool {
        // Canonical ASCII names are case-sensitive. Unknown names and unavailable
        // majors, including non-positive values, return 0 at the script edge.
        self.selected
            .get(name)
            .is_some_and(|version| i64::from(version.major) == major)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_versions_and_modality_names_do_not_grant_environment_capabilities() {
        let offered = [
            EnvironmentCapability {
                name: INPUT_TIMED_VIEWPORT_CAPABILITY.into(),
                versions: VersionRange::exact(ProtocolVersion::new(2, 0)),
            },
            EnvironmentCapability {
                name: INPUT_DEVICE_PUMP_CAPABILITY.into(),
                versions: VersionRange::exact(INPUT_ENVIRONMENT_VERSION),
            },
            EnvironmentCapability {
                name: "keyboard".into(),
                versions: VersionRange::exact(INPUT_ENVIRONMENT_VERSION),
            },
        ];
        let values = select_environment(&offered, &[]);
        let selected = Environment::from_selected(&values);
        assert!(!selected.has(INPUT_TIMED_VIEWPORT_CAPABILITY, 1));
        assert!(!selected.has(INPUT_DEVICE_PUMP_CAPABILITY, 1));
        assert!(!selected.has("keyboard", 1));
        assert!(selected.has(INPUT_SEQUENCE_CAPABILITY, 1));
        assert!(selected.has(INPUT_MACROS_CAPABILITY, 1));
    }
}
