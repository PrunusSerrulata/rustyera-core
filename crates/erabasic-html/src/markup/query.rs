//! Source-preserving query planning. Rendering and service scheduling stay with the caller.

mod length;
mod source;
mod split;

use serde::{Deserialize, Serialize};

use super::{HtmlDocument, HtmlElementKind, HtmlError, HtmlNode};

pub use length::{
    HtmlLengthCut, HtmlLengthImageResolution, HtmlLengthMeasuredValue, HtmlLengthMeasurement,
    HtmlLengthProbe, HtmlLengthProbeKind, HtmlStringLengthPlan, HtmlStringLengthPoll,
    HtmlStringLengthResult, HtmlStringLengthSettings, html_string_length_units,
};
pub(super) use source::decode_for_parser;
pub use source::{decode_query_entities, parse_document_with_source_map};
pub use split::{HtmlLinesPoll, HtmlStringLinesPlan, HtmlSubstringPlan, HtmlSubstringPoll};

/// Limits apply to a whole query, not to each measurement response independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlQueryLimits {
    pub maximum_source_bytes: usize,
    pub maximum_output_bytes: usize,
    pub maximum_scalars: usize,
    pub maximum_nodes: usize,
    pub maximum_depth: usize,
    pub maximum_measurements: usize,
    pub maximum_lines: usize,
    pub maximum_work_bytes: usize,
}

impl Default for HtmlQueryLimits {
    fn default() -> Self {
        Self {
            maximum_source_bytes: 1024 * 1024,
            maximum_output_bytes: 1024 * 1024,
            maximum_scalars: 65_536,
            maximum_nodes: 16_384,
            maximum_depth: 64,
            maximum_measurements: 1_048_576,
            maximum_lines: 65_536,
            maximum_work_bytes: 64 * 1024 * 1024,
        }
    }
}

/// All source ranges are half-open UTF-8 byte ranges in the explicitly named source.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlSourceRange {
    pub start: usize,
    pub end: usize,
}

/// Query failures never contain a successful partial substring or line count.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlQueryErrorKind {
    InvalidEntity,
    InvalidUnicode,
    InvalidMarkup,
    UnsupportedTag,
    InvalidMeasurement,
    ResourceLimit,
    NoProgress,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlQueryError {
    pub kind: HtmlQueryErrorKind,
    pub range: HtmlSourceRange,
    pub message: String,
}

impl std::fmt::Display for HtmlQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:?} at {}..{}: {}",
            self.kind, self.range.start, self.range.end, self.message
        )
    }
}

impl std::error::Error for HtmlQueryError {}

impl HtmlQueryError {
    fn new(kind: HtmlQueryErrorKind, start: usize, end: usize, message: &str) -> Self {
        Self {
            kind,
            range: HtmlSourceRange { start, end },
            message: message.into(),
        }
    }

    fn markup(error: &HtmlError) -> Self {
        let kind = match error.kind {
            super::HtmlErrorKind::InvalidEntity => HtmlQueryErrorKind::InvalidEntity,
            super::HtmlErrorKind::UnknownTag => HtmlQueryErrorKind::UnsupportedTag,
            _ => HtmlQueryErrorKind::InvalidMarkup,
        };
        Self::new(kind, error.start, error.end, &format!("{:?}", error.kind))
    }
}

/// Existing preserves the normal parser's entity policy. `ReferenceQuery` only affects new APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlQueryEntityPolicy {
    Existing,
    ReferenceQuery,
}

/// A boundary of an entire decoded scalar, including any complete entity lexeme(s).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlScalarBoundary {
    pub decoded_utf8: usize,
    pub decoded_utf16: usize,
    pub source_byte: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlDecodedSource {
    pub text: String,
    pub boundaries: Vec<HtmlScalarBoundary>,
}

impl HtmlDecodedSource {
    /// Resolve only exact scalar boundaries; surrogate or entity interiors are rejected.
    #[must_use]
    pub fn source_byte_for_utf16(&self, offset: usize) -> Option<usize> {
        self.boundaries
            .binary_search_by_key(&offset, |boundary| boundary.decoded_utf16)
            .ok()
            .map(|index| self.boundaries[index].source_byte)
    }

    /// Resolve an exact UTF-8 scalar boundary to the complete source lexeme boundary.
    #[must_use]
    pub fn source_byte_for_utf8(&self, offset: usize) -> Option<usize> {
        self.boundaries
            .binary_search_by_key(&offset, |boundary| boundary.decoded_utf8)
            .ok()
            .map(|index| self.boundaries[index].source_byte)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlSourceEventKind {
    Text,
    Open {
        kind: HtmlElementKind,
        raw_name: HtmlSourceRange,
    },
    Close {
        kind: HtmlElementKind,
    },
    Void {
        kind: HtmlElementKind,
    },
    Comment,
    ImplicitClose {
        opening_event: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlSourceEvent {
    pub id: usize,
    pub range: HtmlSourceRange,
    pub kind: HtmlSourceEventKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlMappedText {
    pub event_id: usize,
    pub node_path: Vec<usize>,
    pub range: HtmlSourceRange,
    pub boundaries: Vec<HtmlScalarBoundary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlMappedDocument {
    pub document: HtmlDocument,
    pub events: Vec<HtmlSourceEvent>,
    pub texts: Vec<HtmlMappedText>,
}

impl HtmlMappedDocument {
    /// Return a working-source cut only when both decoded coordinate systems agree.
    #[must_use]
    pub fn source_cut(&self, node_path: &[usize], utf8: usize, utf16: usize) -> Option<usize> {
        self.texts
            .iter()
            .find(|text| text.node_path.as_slice() == node_path)?
            .boundaries
            .iter()
            .find(|boundary| boundary.decoded_utf8 == utf8 && boundary.decoded_utf16 == utf16)
            .map(|boundary| boundary.source_byte)
    }
}

/// Each probe is a fresh `HtmlLength` call, not a prefix of another probe's DOM.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlQueryProbeKind {
    Scalar,
    Atomic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlQueryProbe {
    pub id: u64,
    pub kind: HtmlQueryProbeKind,
    pub document: HtmlDocument,
    /// Range in the substring plan's whole-unescaped working source.
    pub source: HtmlSourceRange,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HtmlOutputOrigin {
    Working(HtmlSourceRange),
    GeneratedClose { opening: HtmlSourceRange },
    Reopened { opening: HtmlSourceRange },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlOutputPiece {
    pub output: HtmlSourceRange,
    pub origin: HtmlOutputOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HtmlSubstringResult {
    pub head: String,
    pub tail: String,
    pub head_pieces: Vec<HtmlOutputPiece>,
    pub tail_pieces: Vec<HtmlOutputPiece>,
    pub consumed_working_bytes: usize,
}

fn check_source(source: &str, limits: HtmlQueryLimits) -> Result<(), HtmlQueryError> {
    if source.len() > limits.maximum_source_bytes || source.chars().count() > limits.maximum_scalars
    {
        return Err(HtmlQueryError::new(
            HtmlQueryErrorKind::ResourceLimit,
            0,
            source.len(),
            "HTML query source exceeds its limit",
        ));
    }
    Ok(())
}

fn check_document(document: &HtmlDocument, limits: HtmlQueryLimits) -> Result<(), HtmlQueryError> {
    let mut pending = document
        .nodes
        .iter()
        .map(|node| (node, 1))
        .collect::<Vec<_>>();
    let mut count = 0_usize;
    while let Some((node, depth)) = pending.pop() {
        count += 1;
        if count > limits.maximum_nodes || depth > limits.maximum_depth {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                0,
                0,
                "HTML query tree exceeds its limit",
            ));
        }
        if let HtmlNode::Element { children, .. } = node {
            pending.extend(children.iter().map(|child| (child, depth + 1)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
