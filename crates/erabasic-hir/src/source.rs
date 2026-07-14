use erabasic_ast::{SourceKind, Span};
use serde::{Deserialize, Serialize};

use crate::SourceId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: SourceId,
    pub relative_path: String,
    pub kind: SourceKind,
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
