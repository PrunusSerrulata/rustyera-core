use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredIndexFile {
    pub relative_path: String,
    pub content: String,
    /// Only the alias file beside this primary table, in the same submitted file root.
    #[serde(default)]
    pub aliases: Option<DeferredIndexAliases>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredIndexAliases {
    pub relative_path: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredIndexCatalog {
    /// The key is the upper-case file stem used by `ErhLoader` to match a `#DIM` name.
    pub groups: BTreeMap<String, Vec<DeferredIndexFile>>,
    pub resolved: BTreeMap<String, ResolvedUserIndex>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserIndexRegistration {
    pub variable_name: String,
    pub source_stem: String,
    /// One-based dimension suffix. `None` represents the first/only dimension.
    pub dimension: Option<usize>,
    pub length: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedUserIndex {
    pub variable_name: String,
    /// Aliases may contain any signed i32 index; array access checks bounds separately.
    pub entries: BTreeMap<String, i64>,
    /// First inserted name for each index, after all primary tables and then aliases.
    pub canonical_names: BTreeMap<i64, String>,
}
