use erabasic_ast::{SourceKind, Span};
use serde::{Deserialize, Serialize};

use crate::SourceId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: SourceId,
    pub relative_path: String,
    pub kind: SourceKind,
    /// BLAKE3 digest of the exact UTF-8 bytes submitted by the frontend.
    pub content_hash: [u8; 32],
    pub byte_len: u64,
    /// UTF-8 byte offsets for the beginning of each source line.
    pub line_starts: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub source: SourceId,
    pub span: Span,
}

impl SourceLocation {
    #[must_use]
    pub const fn new(source: SourceId, span: Span) -> Self {
        Self { source, span }
    }
}
