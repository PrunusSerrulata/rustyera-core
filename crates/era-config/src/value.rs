use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConfigValue {
    Boolean(bool),
    Integer(i64),
    String(String),
    Enum { value: String, allowed: Vec<String> },
    Color(u32),
    Character(char),
    IntegerList(Vec<i64>),
    StringList(Vec<String>),
}

impl ConfigValue {
    /// Render the value using the canonical syntax accepted by Emuera config files.
    #[must_use]
    pub fn config_text(&self) -> String {
        match self {
            Self::Boolean(value) => if *value { "YES" } else { "NO" }.into(),
            Self::Integer(value) => value.to_string(),
            Self::String(value) | Self::Enum { value, .. } => value.clone(),
            Self::Color(value) => format!(
                "{},{},{}",
                (value >> 16) & 0xff,
                (value >> 8) & 0xff,
                value & 0xff
            ),
            Self::Character(value) => value.to_string(),
            Self::IntegerList(values) => values
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            Self::StringList(values) => values.join(","),
        }
    }

    /// Convert to the exact scalar family exposed by GETCONFIG/GETCONFIGS.
    #[must_use]
    pub fn script_value(&self) -> ScriptConfigValue {
        match self {
            Self::Boolean(value) => ScriptConfigValue::Integer(i64::from(*value)),
            Self::Integer(value) => ScriptConfigValue::Integer(*value),
            Self::Color(value) => ScriptConfigValue::Integer(i64::from(*value)),
            Self::String(value) | Self::Enum { value, .. } => {
                ScriptConfigValue::String(value.clone())
            }
            Self::Character(value) => ScriptConfigValue::String(value.to_string()),
            // The pinned reference falls through to List<Int64>.ToString() here.
            // Keep that odd script-visible result rather than exposing a nicer list.
            Self::IntegerList(_) => {
                ScriptConfigValue::String("System.Collections.Generic.List`1[System.Int64]".into())
            }
            Self::StringList(values) => ScriptConfigValue::String(values.join(",")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScriptConfigValue {
    Integer(i64),
    String(String),
}
