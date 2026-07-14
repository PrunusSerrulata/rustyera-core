use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeferredIndexFile {
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
    pub entries: BTreeMap<String, usize>,
}
