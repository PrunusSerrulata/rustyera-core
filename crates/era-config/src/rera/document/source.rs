//! Source-span preservation for parsed RERA TOML documents.

use std::collections::BTreeMap;

use toml_edit::{Document, Item};

use super::super::{ByteSpan, ReraConfigError};

pub(super) fn available_span(
    item: &Item,
    path: &str,
    offset: usize,
    spans: &BTreeMap<String, ByteSpan>,
) -> Option<ByteSpan> {
    item.span()
        .map(ByteSpan::from)
        .map(|span| offset_span(span, offset))
        .or_else(|| spans.get(path).copied())
}

pub(super) fn shift_error(
    mut error: ReraConfigError,
    offset: usize,
    fallback: Option<ByteSpan>,
) -> ReraConfigError {
    error.span = error
        .span
        .map(|span| offset_span(span, offset))
        .or(fallback);
    error
}

pub(super) fn collect_source_spans(
    document: &Document<String>,
    offset: usize,
) -> BTreeMap<String, ByteSpan> {
    let mut spans = BTreeMap::new();
    for (section, item) in document.iter() {
        if let Some(span) = item.span() {
            spans.insert(section.to_owned(), offset_span(span.into(), offset));
        }
        if let Some(table) = item.as_table() {
            for (key, field) in table {
                if let Some(span) = field.span() {
                    spans.insert(format!("{section}.{key}"), offset_span(span.into(), offset));
                }
            }
        }
    }
    spans
}

pub(super) fn offset_span(span: ByteSpan, offset: usize) -> ByteSpan {
    ByteSpan {
        start: span.start + offset,
        end: span.end + offset,
    }
}
