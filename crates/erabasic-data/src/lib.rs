//! Stable, I/O-free project-data contract for application frontends and future
//! `RustyEra` runtime components.
//!
//! This crate deliberately does not know how CSV files are discovered or parsed. It
//! represents project-loading results as deterministic Serde values. The VM consumes
//! this contract, while the higher-level runtime remains a separate component.

mod catalog;
mod deferred;
mod initialization;
mod schema;
mod static_data;

pub use catalog::builtin_schema;
pub use deferred::{
    DeferredIndexAliases, DeferredIndexCatalog, DeferredIndexFile, ResolvedUserIndex,
    UserIndexRegistration,
};
pub use initialization::{
    CharacterSelection, NewGameSeed, RuntimeDefaults, SaveCompatibility, SaveLoadContext,
};
pub use schema::{
    IndexSpaceSchema, Persistence, ProjectSchema, StorageScope, ValueType, VariableId,
    VariableSchema,
};
pub use static_data::{
    CharacterTemplate, ExtensionData, GameBase, LegacyEncoding, NameAlias, NameTable,
    NameTableKind, ProjectData, ProjectStaticData, ReplaceSettings,
};

/// Version of the serialized `ProjectData` contract.
pub const PROJECT_DATA_FORMAT_VERSION: u32 = 3;
