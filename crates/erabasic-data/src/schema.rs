use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::NameTableKind;

/// A variable identity remains explicit in JSON so user variables cannot collide with
/// names reserved by the pinned Emuera build.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum VariableId {
    Builtin(String),
    User(String),
}

impl VariableId {
    #[must_use]
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin(name.into())
    }

    #[must_use]
    pub fn user(name: impl Into<String>) -> Self {
        Self::User(name.into())
    }

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin(name) | Self::User(name) => name,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Integer,
    String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageScope {
    Normal,
    Local,
    Global,
    Character,
    Constant,
    Calculated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    None,
    GameSave,
    GlobalSave,
    ExtendedSave,
}

/// Fully resolved storage shape for one built-in or user-defined variable.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VariableSchema {
    pub id: VariableId,
    pub value_type: ValueType,
    pub storage: StorageScope,
    pub dimensions: Vec<usize>,
    pub mutable: bool,
    pub persistence: Persistence,
    pub can_forbid: bool,
}

impl VariableSchema {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.dimensions.iter().all(|length| *length > 0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IndexSpaceSchema {
    pub kind: NameTableKind,
    pub length: usize,
}

/// Project-wide variable and named-index schema after `VariableSize.csv` reconciliation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectSchema {
    pub variables: BTreeMap<String, VariableSchema>,
    /// Reference declaration order for project-defined variables.
    #[serde(default)]
    pub user_variable_order: Vec<String>,
    pub index_spaces: BTreeMap<NameTableKind, IndexSpaceSchema>,
}

impl ProjectSchema {
    #[must_use]
    pub fn builtin_defaults() -> Self {
        crate::builtin_schema()
    }

    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&VariableSchema> {
        self.variables.get(&name.to_ascii_uppercase())
    }

    pub fn variable_mut(&mut self, name: &str) -> Option<&mut VariableSchema> {
        self.variables.get_mut(&name.to_ascii_uppercase())
    }

    /// Registering an ERH variable is intentionally separate from CSV loading: Emuera
    /// does not know a user variable's dimensions until its `#DIM` line is analyzed.
    pub fn register_user_variable(&mut self, variable: VariableSchema) -> Option<VariableSchema> {
        let key = variable.id.name().to_ascii_uppercase();
        if !self.variables.contains_key(&key) {
            self.user_variable_order.push(key.clone());
        }
        self.variables.insert(key, variable)
    }
}
