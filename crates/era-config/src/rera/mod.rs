mod catalog;
mod document;
mod migration;
mod retired;
mod schema;

#[cfg(test)]
mod tests;

use crate::{ConfigClient, ConfigValue};

pub use catalog::rera_catalog;
pub use document::ReraConfigDocument;
pub use migration::{
    LegacyConfigSource, LegacyMigration, LegacyMigrationDiagnostic, LegacyMigrationDiagnosticKind,
    migrate_legacy_configuration,
};
pub use schema::{generate_annotated_example, generate_json_schema};

pub const RERACONFIG_SCHEMA_VERSION: i64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

impl From<std::ops::Range<usize>> for ByteSpan {
    fn from(range: std::ops::Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReraConfigErrorKind {
    TomlSyntax,
    UnsupportedStructure,
    UnknownField,
    InvalidMetadata,
    InvalidType,
    InvalidValue,
    OutOfRange,
    LockedSetting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReraConfigError {
    pub kind: ReraConfigErrorKind,
    pub path: Option<String>,
    pub span: Option<ByteSpan>,
    pub message: String,
}

impl std::fmt::Display for ReraConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(formatter, "{path}: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for ReraConfigError {}

#[derive(Clone, Debug)]
pub struct ReraConfigSpec {
    pub id: u16,
    pub code: &'static str,
    pub path: &'static str,
    pub description_zh_cn: &'static str,
    pub default: ConfigValue,
    pub clients: &'static [ConfigClient],
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub deprecated: bool,
}

#[must_use]
pub fn normalize_line_endings(input: &str) -> String {
    input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

pub(super) fn error_at(
    kind: ReraConfigErrorKind,
    path: Option<&str>,
    span: Option<ByteSpan>,
    message: impl Into<String>,
) -> ReraConfigError {
    ReraConfigError {
        kind,
        path: path.map(Into::into),
        span,
        message: message.into(),
    }
}
