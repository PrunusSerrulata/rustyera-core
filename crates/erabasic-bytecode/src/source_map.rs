use serde::{Deserialize, Serialize};

use crate::{Digest, SymbolKey};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub relative_path: String,
    pub content_hash: Digest,
    pub byte_len: u64,
    pub line_starts: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMapEntry {
    pub function: SymbolKey,
    pub code_start: u64,
    pub code_end: u64,
    pub source_index: u32,
    pub byte_start: u64,
    pub byte_end: u64,
    /// Parent origins are stored outermost first for future macro expansion/inlining.
    pub origin_chain: Vec<(u32, u64, u64)>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMap {
    pub sources: Vec<SourceRecord>,
    pub entries: Vec<SourceMapEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSourceLocation {
    pub relative_path: String,
    pub content_hash: Digest,
    pub byte_start: u64,
    pub byte_end: u64,
    pub line: u64,
    pub byte_column: u64,
}

impl SourceMap {
    /// Resolve a VM instruction offset to a one-based line and zero-based UTF-8 byte column.
    #[must_use]
    pub fn resolve(&self, function: SymbolKey, code_offset: u64) -> Option<ResolvedSourceLocation> {
        let entry = self.entries.iter().find(|entry| {
            entry.function == function
                && entry.code_start <= code_offset
                && code_offset < entry.code_end
        })?;
        let source = self.sources.get(entry.source_index as usize)?;
        let line_index = source
            .line_starts
            .partition_point(|line_start| *line_start <= entry.byte_start)
            .saturating_sub(1);
        let line_start = *source.line_starts.get(line_index)?;
        Some(ResolvedSourceLocation {
            relative_path: source.relative_path.clone(),
            content_hash: source.content_hash,
            byte_start: entry.byte_start,
            byte_end: entry.byte_end,
            line: u64::try_from(line_index).ok()? + 1,
            byte_column: entry.byte_start.checked_sub(line_start)?,
        })
    }
}
