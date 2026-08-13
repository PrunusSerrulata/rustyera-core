use std::io::Read;

use flate2::read::GzDecoder;

use crate::format::{HEADER, VERSION, ZIP_HEADER};
use crate::{SaveCodecError, SaveCodecLimits, SaveFileKind, SaveFormat, SaveMetadata};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveMetadataInspection {
    Complete {
        format: SaveFormat,
        kind: SaveFileKind,
        metadata: SaveMetadata,
    },
    NeedMore,
}

/// Inspect the leading metadata of a save without materializing its variable payload.
///
/// `complete` tells the parser that no more bytes can arrive. A truncated prefix returns
/// [`SaveMetadataInspection::NeedMore`], while the same bytes at EOF return a format error.
///
/// # Errors
///
/// Returns an error for malformed, unsupported, or over-limit metadata.
pub fn inspect_metadata(
    data: &[u8],
    complete: bool,
    limits: SaveCodecLimits,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    if data.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let header = data
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes);
    match header {
        Some(header) if header == HEADER || header == ZIP_HEADER => {
            inspect_binary(data, complete, limits, header)
        }
        _ => inspect_text(data, complete, limits),
    }
}

fn inspect_text(
    data: &[u8],
    complete: bool,
    limits: SaveCodecLimits,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let source = match std::str::from_utf8(data) {
        Ok(source) => source,
        Err(error) if !complete && error.error_len().is_none() => {
            return Ok(SaveMetadataInspection::NeedMore);
        }
        Err(_) => {
            return Err(SaveCodecError::InvalidFormat(
                "text save is not UTF-8".into(),
            ));
        }
    };
    let mut lines = source.split_inclusive('\n');
    let Some(unique) = lines.next() else {
        return incomplete_or(
            complete,
            SaveCodecError::InvalidFormat("text save lacks unique code".into()),
        );
    };
    let Some(version) = lines.next() else {
        return incomplete_or(
            complete,
            SaveCodecError::InvalidFormat("text save lacks script version".into()),
        );
    };
    let Some(description) = lines.next() else {
        return incomplete_or(
            complete,
            SaveCodecError::InvalidFormat("text save lacks description".into()),
        );
    };
    if !complete
        && (!unique.ends_with('\n') || !version.ends_with('\n') || !description.ends_with('\n'))
    {
        return Ok(SaveMetadataInspection::NeedMore);
    }
    let unique = unique.trim_end_matches(['\r', '\n']);
    let version = version.trim_end_matches(['\r', '\n']);
    let description = description.trim_end_matches(['\r', '\n']);
    if description.len() > limits.maximum_string_bytes {
        return Err(SaveCodecError::LimitExceeded("string bytes"));
    }
    let unique_code = unique
        .parse()
        .map_err(|_| SaveCodecError::InvalidFormat("text save has invalid unique code".into()))?;
    let version = version.parse().map_err(|_| {
        SaveCodecError::InvalidFormat("text save has invalid script version".into())
    })?;
    Ok(SaveMetadataInspection::Complete {
        format: SaveFormat::Text1808,
        kind: SaveFileKind::Normal,
        metadata: SaveMetadata {
            unique_code,
            version,
            description: description.to_owned(),
        },
    })
}

fn inspect_binary(
    data: &[u8],
    complete: bool,
    limits: SaveCodecLimits,
    header: u64,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    let Some(version) = read_u32(data, 8) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    if version != VERSION {
        return Err(SaveCodecError::UnsupportedVersion(version));
    }
    let Some(data_count) = read_u32(data, 12) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    let body_offset = 16usize
        .checked_add(
            (data_count as usize)
                .checked_mul(4)
                .ok_or(SaveCodecError::LimitExceeded("header data"))?,
        )
        .ok_or(SaveCodecError::LimitExceeded("header data"))?;
    let Some(body) = data.get(body_offset..) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    if header == HEADER {
        inspect_binary_body(body, complete, SaveFormat::Binary1808, limits)
    } else {
        inspect_compressed_body(body, complete, limits)
    }
}

fn inspect_compressed_body(
    body: &[u8],
    complete: bool,
    limits: SaveCodecLimits,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    let maximum = limits.maximum_string_bytes.saturating_add(32);
    let mut decoder = GzDecoder::new(body);
    let mut prefix = Vec::with_capacity(8 * 1024);
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => {
                return inspect_binary_body(&prefix, complete, SaveFormat::Binary1808Gzip, limits);
            }
            Ok(count) => {
                prefix.extend_from_slice(&chunk[..count]);
                if prefix.len() > maximum {
                    return Err(SaveCodecError::LimitExceeded("string bytes"));
                }
                if let SaveMetadataInspection::Complete { metadata, kind, .. } =
                    inspect_binary_body(&prefix, false, SaveFormat::Binary1808Gzip, limits)?
                {
                    return Ok(SaveMetadataInspection::Complete {
                        format: SaveFormat::Binary1808Gzip,
                        kind,
                        metadata,
                    });
                }
            }
            Err(_error) if !complete => return Ok(SaveMetadataInspection::NeedMore),
            Err(error) => return Err(SaveCodecError::Compression(error.to_string())),
        }
    }
}

fn inspect_binary_body(
    body: &[u8],
    complete: bool,
    format: SaveFormat,
    limits: SaveCodecLimits,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    let Some(kind) = body.first().copied() else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    let kind = match kind {
        0 => SaveFileKind::Normal,
        1 => SaveFileKind::Global,
        2 => SaveFileKind::Variable,
        3 => SaveFileKind::Character,
        _ => {
            return Err(SaveCodecError::InvalidFormat(
                "unknown save file kind".into(),
            ));
        }
    };
    let Some(unique_code) = read_i64(body, 1) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    let Some(version) = read_i64(body, 9) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    let Some((length, prefix)) = read_7bit(body.get(17..).unwrap_or_default())? else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    if length > limits.maximum_string_bytes || length % 2 != 0 {
        return Err(SaveCodecError::LimitExceeded("string bytes"));
    }
    let start = 17usize.saturating_add(prefix);
    let Some(bytes) = body.get(start..start.saturating_add(length)) else {
        return incomplete_or(complete, SaveCodecError::InvalidHeader);
    };
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let description = String::from_utf16(&units)
        .map_err(|_| SaveCodecError::InvalidFormat("invalid UTF-16 string".into()))?;
    Ok(SaveMetadataInspection::Complete {
        format,
        kind,
        metadata: SaveMetadata {
            unique_code,
            version,
            description,
        },
    })
}

fn read_7bit(bytes: &[u8]) -> Result<Option<(usize, usize)>, SaveCodecError> {
    let mut result = 0usize;
    for shift in (0..35).step_by(7) {
        let Some(byte) = bytes.get(shift / 7).copied() else {
            return Ok(None);
        };
        result |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(Some((result, shift / 7 + 1)));
        }
    }
    Err(SaveCodecError::InvalidFormat(
        "invalid string length".into(),
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn incomplete_or(
    complete: bool,
    error: SaveCodecError,
) -> Result<SaveMetadataInspection, SaveCodecError> {
    if complete {
        Err(error)
    } else {
        Ok(SaveMetadataInspection::NeedMore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SaveDocument, encode};

    fn document(format: SaveFormat) -> SaveDocument {
        SaveDocument {
            format,
            kind: SaveFileKind::Normal,
            metadata: SaveMetadata {
                unique_code: 7,
                version: 9,
                description: "slot description".into(),
            },
            characters: Vec::new(),
            character_user_defined_starts: Vec::new(),
            variables: Vec::new(),
            opaque_extensions: Vec::new(),
            text_payload: None,
        }
    }

    #[test]
    fn binary_and_gzip_metadata_stop_before_variable_payload() {
        for format in [SaveFormat::Binary1808, SaveFormat::Binary1808Gzip] {
            let encoded = encode(&document(format), format, SaveCodecLimits::default()).unwrap();
            let inspected = inspect_metadata(&encoded, true, SaveCodecLimits::default()).unwrap();
            assert!(matches!(
                inspected,
                SaveMetadataInspection::Complete {
                    kind: SaveFileKind::Normal,
                    metadata: SaveMetadata {
                        unique_code: 7,
                        version: 9,
                        ref description,
                    },
                    ..
                } if description == "slot description"
            ));
        }
    }

    #[test]
    fn text_prefix_requests_more_until_the_description_is_complete() {
        let prefix = b"7\n9\nslot";
        assert_eq!(
            inspect_metadata(prefix, false, SaveCodecLimits::default()).unwrap(),
            SaveMetadataInspection::NeedMore
        );
        assert!(matches!(
            inspect_metadata(prefix, true, SaveCodecLimits::default()).unwrap(),
            SaveMetadataInspection::Complete { metadata, .. }
                if metadata.description == "slot"
        ));
    }
}
