//! Adapters from submitted frontend files to the CSV and analyzer input contracts.

use era_runtime_protocol::{
    FilePayload, FrontendIoErrorKind, ProtocolDiagnostic, RuntimeLogLevel, SourceLocation,
};
use erabasic_analyzer::{ProjectSource, SourceIoError, SourceIoErrorKind, SourcePayload};
use erabasic_csv::{
    FilePayload as CsvFilePayload, FrontendFile as CsvFrontendFile, FrontendIoError as CsvIoError,
    FrontendIoErrorKind as CsvIoErrorKind,
};
use erabasic_hir::SourceId;

pub(super) struct CompilerSourceIndex {
    entries: Vec<(SourceId, u32)>,
}

impl CompilerSourceIndex {
    pub(super) fn new(source_ids: &[SourceId]) -> Self {
        let mut entries = source_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, source)| (source, u32::try_from(index).unwrap_or(u32::MAX)))
            .collect::<Vec<_>>();
        entries.sort_unstable_by_key(|entry| entry.0);
        Self { entries }
    }

    pub(super) fn get<'a>(
        &self,
        sources: &'a [erabasic_bytecode::SourceRecord],
        source: SourceId,
    ) -> Option<&'a erabasic_bytecode::SourceRecord> {
        let index = self
            .entries
            .binary_search_by_key(&source, |entry| entry.0)
            .ok()
            .and_then(|index| usize::try_from(self.entries[index].1).ok())?;
        sources.get(index)
    }
}

pub(super) fn csv_file(path: String, payload: FilePayload) -> CsvFrontendFile {
    CsvFrontendFile {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => CsvFilePayload::Utf8(value),
            FilePayload::Bytes(_) | FilePayload::ExternalResource(_) => {
                CsvFilePayload::IoError(CsvIoError {
                    kind: CsvIoErrorKind::InvalidData,
                    message: "CSV and EraBasic sources must be submitted as UTF-8".into(),
                })
            }
            FilePayload::IoError(error) => CsvFilePayload::IoError(CsvIoError {
                kind: csv_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

pub(super) fn analyzer_source(path: String, payload: FilePayload) -> ProjectSource {
    ProjectSource {
        relative_path: path,
        payload: match payload {
            FilePayload::Utf8(value) => SourcePayload::Utf8(value),
            FilePayload::Bytes(_) | FilePayload::ExternalResource(_) => {
                SourcePayload::IoError(SourceIoError {
                    kind: SourceIoErrorKind::InvalidData,
                    message: "EraBasic sources must be submitted as UTF-8".into(),
                })
            }
            FilePayload::IoError(error) => SourcePayload::IoError(SourceIoError {
                kind: analyzer_error_kind(error.kind),
                message: error.message,
            }),
        },
    }
}

fn csv_error_kind(kind: FrontendIoErrorKind) -> CsvIoErrorKind {
    match kind {
        FrontendIoErrorKind::NotFound => CsvIoErrorKind::NotFound,
        FrontendIoErrorKind::PermissionDenied => CsvIoErrorKind::PermissionDenied,
        FrontendIoErrorKind::InvalidData => CsvIoErrorKind::InvalidData,
        FrontendIoErrorKind::Interrupted => CsvIoErrorKind::Interrupted,
        FrontendIoErrorKind::ReadOnly
        | FrontendIoErrorKind::AlreadyExists
        | FrontendIoErrorKind::Conflict
        | FrontendIoErrorKind::Other => CsvIoErrorKind::Other,
    }
}

fn analyzer_error_kind(kind: FrontendIoErrorKind) -> SourceIoErrorKind {
    match kind {
        FrontendIoErrorKind::NotFound => SourceIoErrorKind::NotFound,
        FrontendIoErrorKind::PermissionDenied => SourceIoErrorKind::PermissionDenied,
        FrontendIoErrorKind::InvalidData => SourceIoErrorKind::InvalidData,
        FrontendIoErrorKind::Interrupted => SourceIoErrorKind::Interrupted,
        FrontendIoErrorKind::ReadOnly
        | FrontendIoErrorKind::AlreadyExists
        | FrontendIoErrorKind::Conflict
        | FrontendIoErrorKind::Other => SourceIoErrorKind::Other,
    }
}

pub(super) fn payload_hash(payload: &FilePayload) -> Option<blake3::Hash> {
    match payload {
        FilePayload::Utf8(value) => Some(blake3::hash(value.as_bytes())),
        FilePayload::Bytes(value) => Some(blake3::hash(value.as_slice())),
        FilePayload::ExternalResource(_) | FilePayload::IoError(_) => None,
    }
}

#[cfg(test)]
pub(super) fn project_source_location(
    relative_path: String,
    byte_start: usize,
    byte_end: usize,
    fallback_line: Option<u64>,
    text: Option<&str>,
) -> SourceLocation {
    let (line, byte_column) = text.map_or((fallback_line, None), |text| {
        let clamped_start = byte_start.min(text.len());
        let prefix = &text.as_bytes()[..clamped_start];
        let line_start = prefix
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let line = prefix
            .iter()
            .fold(0u64, |count, byte| count + u64::from(*byte == b'\n'));
        (
            Some(line),
            Some(u64::try_from(clamped_start - line_start).unwrap_or(u64::MAX)),
        )
    });
    SourceLocation {
        relative_path,
        byte_start: u64::try_from(byte_start).unwrap_or(u64::MAX),
        byte_end: u64::try_from(byte_end).unwrap_or(u64::MAX),
        line,
        byte_column,
    }
}

pub(super) fn indexed_project_source_location(
    relative_path: String,
    byte_start: usize,
    byte_end: usize,
    fallback_line: Option<u64>,
    source: Option<&erabasic_hir::SourceFile>,
) -> SourceLocation {
    indexed_source_location(
        relative_path,
        byte_start,
        byte_end,
        fallback_line,
        source.map(|source| (source.byte_len, source.line_starts.as_slice())),
    )
}

pub(super) fn indexed_source_record_location(
    relative_path: String,
    byte_start: usize,
    byte_end: usize,
    source: Option<&erabasic_bytecode::SourceRecord>,
) -> SourceLocation {
    indexed_source_location(
        relative_path,
        byte_start,
        byte_end,
        None,
        source.map(|source| (source.byte_len, source.line_starts.as_slice())),
    )
}

fn indexed_source_location(
    relative_path: String,
    byte_start: usize,
    byte_end: usize,
    fallback_line: Option<u64>,
    source: Option<(u64, &[u64])>,
) -> SourceLocation {
    let (line, byte_column) = source.map_or((fallback_line, None), |(byte_len, line_starts)| {
        let clamped_start = u64::try_from(byte_start).unwrap_or(u64::MAX).min(byte_len);
        let line_index = line_starts
            .partition_point(|line_start| *line_start <= clamped_start)
            .saturating_sub(1);
        let line_start = line_starts.get(line_index).copied().unwrap_or(0);
        (
            Some(u64::try_from(line_index).unwrap_or(u64::MAX)),
            Some(clamped_start.saturating_sub(line_start)),
        )
    });
    SourceLocation {
        relative_path,
        byte_start: u64::try_from(byte_start).unwrap_or(u64::MAX),
        byte_end: u64::try_from(byte_end).unwrap_or(u64::MAX),
        line,
        byte_column,
    }
}

pub(super) fn project_diagnostic(
    code: &str,
    level: RuntimeLogLevel,
    message: impl Into<String>,
    source: Option<SourceLocation>,
) -> ProtocolDiagnostic {
    ProtocolDiagnostic {
        code: code.into(),
        level,
        message: message.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use era_protocol::ProtocolBytes;
    use era_runtime_protocol::FrontendIoError;
    use erabasic_bytecode::{Digest, SourceRecord};

    use super::*;

    #[test]
    fn binary_source_payloads_remain_invalid_utf8_errors() {
        let csv = csv_file(
            "CSV/test.csv".into(),
            FilePayload::Bytes(ProtocolBytes::new([0xff])),
        );
        let CsvFilePayload::IoError(csv_error) = csv.payload else {
            panic!("binary CSV payload must become an I/O error");
        };
        assert_eq!(csv_error.kind, CsvIoErrorKind::InvalidData);
        assert_eq!(
            csv_error.message,
            "CSV and EraBasic sources must be submitted as UTF-8"
        );

        let source = analyzer_source(
            "ERB/test.erb".into(),
            FilePayload::Bytes(ProtocolBytes::new([0xff])),
        );
        let SourcePayload::IoError(source_error) = source.payload else {
            panic!("binary EraBasic payload must become an I/O error");
        };
        assert_eq!(source_error.kind, SourceIoErrorKind::InvalidData);
        assert_eq!(
            source_error.message,
            "EraBasic sources must be submitted as UTF-8"
        );
    }

    #[test]
    fn unsupported_frontend_error_kinds_remain_other_errors() {
        let payload = FilePayload::IoError(FrontendIoError {
            kind: FrontendIoErrorKind::Conflict,
            message: "changed".into(),
            platform_code: Some(17),
        });
        let csv = csv_file("CSV/test.csv".into(), payload.clone());
        let CsvFilePayload::IoError(csv_error) = csv.payload else {
            panic!("frontend error must remain an I/O error");
        };
        assert_eq!(csv_error.kind, CsvIoErrorKind::Other);
        assert_eq!(csv_error.message, "changed");

        let source = analyzer_source("ERB/test.erb".into(), payload);
        let SourcePayload::IoError(source_error) = source.payload else {
            panic!("frontend error must remain an I/O error");
        };
        assert_eq!(source_error.kind, SourceIoErrorKind::Other);
        assert_eq!(source_error.message, "changed");
    }

    #[test]
    fn compiler_source_index_uses_ids_instead_of_vector_positions() {
        let sources = [
            SourceRecord {
                relative_path: "ERB/seven.erb".into(),
                content_hash: Digest::default(),
                byte_len: 0,
                line_starts: vec![0],
            },
            SourceRecord {
                relative_path: "ERB/two.erb".into(),
                content_hash: Digest::default(),
                byte_len: 0,
                line_starts: vec![0],
            },
        ];
        let index = CompilerSourceIndex::new(&[SourceId(7), SourceId(2)]);

        assert_eq!(
            index
                .get(&sources, SourceId(2))
                .map(|source| source.relative_path.as_str()),
            Some("ERB/two.erb")
        );
        assert_eq!(
            index
                .get(&sources, SourceId(7))
                .map(|source| source.relative_path.as_str()),
            Some("ERB/seven.erb")
        );
        assert!(index.get(&sources, SourceId(0)).is_none());
    }
}
