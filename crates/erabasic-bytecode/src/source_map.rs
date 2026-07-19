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
    pub byte_start: u64,
    pub byte_end: u64,
    /// Stable typed-statement identity used to relocate debugger breakpoints
    /// when edits only move an otherwise unchanged statement.
    pub statement_fingerprint: u32,
    /// Parent origins are stored outermost first for future macro expansion/inlining.
    /// Macro/inlining origins are uncommon and absent from ordinary Era source.
    /// Boxing only the non-empty case keeps the hot source-map record compact.
    // `Box<Vec<_>>` is intentional: its thin pointer makes the overwhelmingly
    // common `None` record eight bytes smaller than `Box<[_]>` on 64-bit targets.
    #[allow(clippy::box_collection)]
    pub origin_chain: Option<Box<Vec<(u32, u64, u64)>>>,
    pub source_index: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMap {
    pub sources: Vec<SourceRecord>,
    /// Sorted unique statement identities referenced by compact entry indices.
    pub statement_fingerprints: Vec<Digest>,
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
        self.resolve_entry(entry)
    }

    /// Resolve an entry selected by a caller-maintained source-map index.
    ///
    /// The serialized map intentionally stays index-free and deterministic. Runtime
    /// consumers that resolve locations frequently can build an ephemeral index and
    /// reuse this projection without scanning every function's entries.
    #[must_use]
    pub fn resolve_entry(&self, entry: &SourceMapEntry) -> Option<ResolvedSourceLocation> {
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

    #[must_use]
    pub fn statement_fingerprint(&self, entry: &SourceMapEntry) -> Option<Digest> {
        self.statement_fingerprints
            .get(entry.statement_fingerprint as usize)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_resolution_matches_the_serialized_linear_lookup() {
        let function = SymbolKey::derive("source-map-test", b"function");
        let entry = SourceMapEntry {
            function,
            code_start: 4,
            code_end: 8,
            source_index: 0,
            byte_start: 5,
            byte_end: 7,
            statement_fingerprint: 0,
            origin_chain: None,
        };
        let map = SourceMap {
            sources: vec![SourceRecord {
                relative_path: "utf8.erb".into(),
                content_hash: Digest::hash("source-map-test", &["界\nabc".as_bytes()]),
                byte_len: 7,
                line_starts: vec![0, 4],
            }],
            statement_fingerprints: vec![Digest::default()],
            entries: vec![entry.clone()],
        };
        assert_eq!(map.resolve_entry(&entry), map.resolve(function, 5));
        let location = map.resolve_entry(&entry).expect("valid indexed entry");
        assert_eq!(location.line, 2);
        assert_eq!(location.byte_column, 1);
    }
}
