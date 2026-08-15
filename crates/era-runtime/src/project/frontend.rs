//! Adapters from submitted frontend files to the CSV and analyzer input contracts.

use era_runtime_protocol::{
    FilePayload, FrontendIoErrorKind, ProjectManifest, ProtocolDiagnostic, RuntimeLogLevel,
    SourceLocation, validate_relative_path,
};
use erabasic_analyzer::{ProjectSource, SourceIoError, SourceIoErrorKind, SourcePayload};
use erabasic_csv::{
    FilePayload as CsvFilePayload, FrontendFile as CsvFrontendFile, FrontendIoError as CsvIoError,
    FrontendIoErrorKind as CsvIoErrorKind,
};

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

pub(super) fn manifest_source_texts(
    manifest: &ProjectManifest,
) -> std::collections::BTreeMap<String, &str> {
    manifest
        .files
        .iter()
        .filter_map(|file| {
            let FilePayload::Utf8(text) = &file.payload else {
                return None;
            };
            let path = validate_relative_path(&file.relative_path).ok()?;
            Some((path.to_ascii_lowercase(), text.as_str()))
        })
        .collect()
}

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
}
