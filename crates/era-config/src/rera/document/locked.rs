//! Validation and source-aware parsing of `meta.locked_settings`.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{DocumentMut, Item};

use super::super::{ByteSpan, ReraConfigError, ReraConfigErrorKind, error_at};
use super::source::{available_span, offset_span};

pub(super) struct LockedPath {
    pub(super) path: String,
    pub(super) span: Option<ByteSpan>,
}

pub(super) fn validate_v1_locked_settings(
    item: &Item,
    offset: usize,
    spans: &BTreeMap<String, ByteSpan>,
) -> Result<(), ReraConfigError> {
    parse_locked_paths(item, offset, spans).map(|_| ())
}

pub(super) fn locked_paths(
    document: &DocumentMut,
    offset: usize,
    spans: &BTreeMap<String, ByteSpan>,
) -> Result<Vec<LockedPath>, ReraConfigError> {
    let Some(item) = document
        .get("meta")
        .and_then(|meta| meta.get("locked_settings"))
    else {
        return Ok(Vec::new());
    };
    parse_locked_paths(item, offset, spans)
}

fn parse_locked_paths(
    item: &Item,
    offset: usize,
    spans: &BTreeMap<String, ByteSpan>,
) -> Result<Vec<LockedPath>, ReraConfigError> {
    let array = item.as_array().ok_or_else(|| {
        error_at(
            ReraConfigErrorKind::InvalidMetadata,
            Some("meta.locked_settings"),
            available_span(item, "meta.locked_settings", offset, spans),
            "必须是字符串数组",
        )
    })?;
    let mut paths = BTreeSet::new();
    let mut locked = Vec::new();
    for value in array {
        let span = value
            .span()
            .map(ByteSpan::from)
            .map(|span| offset_span(span, offset))
            .or_else(|| spans.get("meta.locked_settings").copied());
        let path = value.as_str().ok_or_else(|| {
            error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "数组项必须是字符串",
            )
        })?;
        if !paths.insert(path.to_owned()) {
            return Err(error_at(
                ReraConfigErrorKind::InvalidMetadata,
                Some("meta.locked_settings"),
                span,
                "不允许重复路径",
            ));
        }
        locked.push(LockedPath {
            path: path.to_owned(),
            span,
        });
    }
    Ok(locked)
}
