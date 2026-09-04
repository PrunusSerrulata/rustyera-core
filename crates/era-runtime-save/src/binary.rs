mod cursor;
mod encode;

use std::borrow::Cow;
use std::io::Read;

use flate2::bufread::GzDecoder;

use cursor::Cursor;
pub use encode::{encode_binary, encode_save_extension};
#[cfg(test)]
use encode::{write_integer_array, write_string_array};

use crate::format::{HEADER, VERSION, ZIP_HEADER};
use crate::model::decode_file_kind;
use crate::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveExtension,
    SaveFileKind, SaveFormat, SaveMetadata,
};
#[cfg(test)]
use crate::{SaveEntry, SaveValue};

const EOF: u8 = 0xFF;
const EOC: u8 = 0xFE;
const SEPARATOR: u8 = 0xFD;

pub(crate) fn is_binary(data: &[u8]) -> bool {
    data.get(..8)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_le_bytes)
        .is_some_and(|header| header == HEADER || header == ZIP_HEADER)
}

/// Decode an Emuera 1808 binary or compressed-binary save.
///
/// # Errors
///
/// Returns an error for malformed, unsupported, compressed, or oversized input.
pub fn decode_binary(data: &[u8], limits: SaveCodecLimits) -> Result<SaveDocument, SaveCodecError> {
    decode_binary_with_array_mode(data, limits, false)
}

/// Decode an Emuera 1808 binary save while preserving sparse array runs.
///
/// # Errors
///
/// Returns an error for malformed, unsupported, compressed, or oversized input.
pub fn decode_binary_sparse(
    data: &[u8],
    limits: SaveCodecLimits,
) -> Result<SaveDocument, SaveCodecError> {
    decode_binary_with_array_mode(data, limits, true)
}

#[allow(clippy::too_many_lines)]
fn decode_binary_with_array_mode(
    data: &[u8],
    limits: SaveCodecLimits,
    sparse_arrays: bool,
) -> Result<SaveDocument, SaveCodecError> {
    if data.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    let mut outer = Cursor::new(data, limits);
    let header = outer.u64()?;
    if header != HEADER && header != ZIP_HEADER {
        return Err(SaveCodecError::InvalidHeader);
    }
    let version = outer.u32()?;
    if version != VERSION {
        return Err(SaveCodecError::UnsupportedVersion(version));
    }
    let data_count = outer.u32()? as usize;
    outer.take(
        data_count
            .checked_mul(4)
            .ok_or(SaveCodecError::LimitExceeded("header data"))?,
    )?;
    let body = if header == ZIP_HEADER {
        let compressed = outer.remaining();
        let mut gzip = GzDecoder::new(std::io::Cursor::new(compressed));
        let mut decompressed = Vec::new();
        gzip.by_ref()
            .take(u64::try_from(limits.maximum_bytes.saturating_add(1)).unwrap_or(u64::MAX))
            .read_to_end(&mut decompressed)
            .map_err(|error| SaveCodecError::Compression(error.to_string()))?;
        if decompressed.len() > limits.maximum_bytes {
            return Err(SaveCodecError::LimitExceeded("decompressed bytes"));
        }
        if usize::try_from(gzip.into_inner().position()).unwrap_or(usize::MAX) != compressed.len() {
            return Err(SaveCodecError::InvalidFormat(
                "trailing data follows the compressed save".into(),
            ));
        }
        Cow::Owned(decompressed)
    } else {
        Cow::Borrowed(outer.remaining())
    };
    let mut reader = if sparse_arrays {
        Cursor::new_sparse(body.as_ref(), limits)
    } else {
        Cursor::new(body.as_ref(), limits)
    };
    let kind = decode_file_kind(reader.u8()?)?;
    let metadata = SaveMetadata {
        unique_code: reader.i64()?,
        version: reader.i64()?,
        description: reader.string()?,
    };
    let mut characters = Vec::new();
    let mut character_user_defined_starts = Vec::new();
    if matches!(kind, SaveFileKind::Normal | SaveFileKind::Character) {
        let count = usize::try_from(reader.i64()?)
            .map_err(|_| SaveCodecError::InvalidFormat("negative character count".into()))?;
        if count > limits.maximum_characters {
            return Err(SaveCodecError::LimitExceeded("maximum characters"));
        }
        for _ in 0..count {
            let (entries, user_defined_start) = reader.character_entries()?;
            characters.push(entries);
            character_user_defined_starts.push(user_defined_start);
        }
    }
    let variables = reader.entries(EOF)?;
    let mut opaque_extensions = Vec::new();
    if matches!(kind, SaveFileKind::Normal | SaveFileKind::Global) && !reader.remaining().is_empty()
    {
        loop {
            let tag = reader.u8()?;
            if tag == EOF {
                break;
            }
            if !(0x20..=0x22).contains(&tag) {
                return Err(SaveCodecError::InvalidFormat(
                    "non-extension entry follows the primary EOF".into(),
                ));
            }
            let key = reader.string()?;
            let start = reader.position;
            match tag {
                0x20 => {
                    let count = reader.u32()? as usize;
                    if count > limits.maximum_entries {
                        return Err(SaveCodecError::LimitExceeded("map entries"));
                    }
                    for _ in 0..count {
                        reader.string()?;
                        reader.string()?;
                    }
                }
                0x21 => {
                    reader.string()?;
                }
                0x22 => {
                    reader.string()?;
                    reader.string()?;
                }
                _ => unreachable!(),
            }
            opaque_extensions.push(OpaqueSaveExtension {
                type_tag: tag,
                key,
                payload: body[start..reader.position].to_vec(),
            });
        }
    }
    if !reader.remaining().is_empty() {
        return Err(SaveCodecError::InvalidFormat(
            "trailing data follows the save terminator".into(),
        ));
    }
    Ok(SaveDocument {
        format: if header == ZIP_HEADER {
            SaveFormat::Binary1808Gzip
        } else {
            SaveFormat::Binary1808
        },
        kind,
        metadata,
        characters,
        character_user_defined_starts,
        variables,
        opaque_extensions,
        text_payload: None,
    })
}

///
/// # Errors
///
/// Returns an error when the tag, payload shape, or configured limits are invalid.
pub fn decode_save_extension(
    extension: &OpaqueSaveExtension,
    limits: SaveCodecLimits,
) -> Result<SaveExtension, SaveCodecError> {
    let mut reader = Cursor::new(&extension.payload, limits);
    let decoded = match extension.type_tag {
        0x20 => {
            let count = reader.u32()? as usize;
            if count > limits.maximum_entries {
                return Err(SaveCodecError::LimitExceeded("map entries"));
            }
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push((reader.string()?, reader.string()?));
            }
            SaveExtension::Map {
                key: extension.key.clone(),
                entries,
            }
        }
        0x21 => SaveExtension::Xml {
            key: extension.key.clone(),
            document: reader.string()?,
        },
        0x22 => SaveExtension::DataTable {
            key: extension.key.clone(),
            schema: reader.string()?,
            data: reader.string()?,
        },
        _ => {
            return Err(SaveCodecError::InvalidFormat(
                "unknown save extension tag".into(),
            ));
        }
    };
    if !reader.remaining().is_empty() {
        return Err(SaveCodecError::InvalidFormat(
            "save extension has trailing bytes".into(),
        ));
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_use_reference_zero_run_row_and_plane_encoding() {
        let limits = SaveCodecLimits::default();

        let mut one_dimension = Vec::new();
        write_integer_array(&mut one_dimension, &[8], &[0, 0, 5, 0, 0, 0, 7, 0], limits).unwrap();
        assert_eq!(one_dimension, [8, 0, 0, 0, 0xF0, 2, 5, 0xF0, 3, 7, 0xFF]);

        let mut two_dimensions = Vec::new();
        write_integer_array(
            &mut two_dimensions,
            &[3, 4],
            &[0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0],
            limits,
        )
        .unwrap();
        assert_eq!(
            two_dimensions,
            [3, 0, 0, 0, 4, 0, 0, 0, 0xF1, 1, 0xF0, 1, 2, 0xE0, 0xFF]
        );

        let mut three_dimensions = Vec::new();
        let mut values = vec![String::new(); 2 * 2 * 3];
        values[8] = "x".into();
        write_string_array(&mut three_dimensions, &[2, 2, 3], &values, limits).unwrap();
        assert_eq!(
            three_dimensions,
            [
                2, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0xF2, 1, 0xF0, 2, 0xD8, 2, b'x', 0, 0xE0, 0xE1,
                0xFF,
            ]
        );
    }

    #[test]
    fn binary_character_separator_round_trips_at_an_empty_section_boundary() {
        let document = SaveDocument {
            format: SaveFormat::Binary1808,
            kind: SaveFileKind::Normal,
            metadata: SaveMetadata {
                unique_code: 1,
                version: 2,
                description: String::new(),
            },
            characters: vec![vec![SaveEntry {
                name: "NO".into(),
                value: SaveValue::Integer(7),
            }]],
            character_user_defined_starts: vec![Some(1)],
            variables: Vec::new(),
            opaque_extensions: Vec::new(),
            text_payload: None,
        };
        let bytes = encode_binary(
            &document,
            SaveFormat::Binary1808,
            SaveCodecLimits::default(),
        )
        .unwrap();

        assert_eq!(&bytes[bytes.len() - 5..], &[7, SEPARATOR, EOC, EOF, EOF]);
        assert_eq!(
            decode_binary(&bytes, SaveCodecLimits::default()).unwrap(),
            document
        );
    }

    #[test]
    fn sparse_multidimensional_arrays_round_trip_without_shifting_values() {
        let document = SaveDocument {
            format: SaveFormat::Binary1808,
            kind: SaveFileKind::Normal,
            metadata: SaveMetadata {
                unique_code: 1,
                version: 2,
                description: String::new(),
            },
            characters: Vec::new(),
            character_user_defined_starts: Vec::new(),
            variables: vec![
                SaveEntry {
                    name: "TWO_D".into(),
                    value: SaveValue::Integers {
                        dimensions: vec![4, 5],
                        values: vec![
                            0, 0, 0, 0, 0, //
                            0, 7, 0, 0, 0, //
                            0, 0, 0, 0, 0, //
                            0, 0, 0, 9, 0,
                        ],
                    },
                },
                SaveEntry {
                    name: "THREE_D".into(),
                    value: SaveValue::Strings {
                        dimensions: vec![3, 3, 4],
                        values: {
                            let mut values = vec![String::new(); 36];
                            values[5] = "first".into();
                            values[32] = "last".into();
                            values
                        },
                    },
                },
            ],
            opaque_extensions: Vec::new(),
            text_payload: None,
        };

        let encoded = encode_binary(
            &document,
            SaveFormat::Binary1808,
            SaveCodecLimits::default(),
        )
        .unwrap();
        let sparse = decode_binary_sparse(&encoded, SaveCodecLimits::default()).unwrap();
        assert!(matches!(
            &sparse.variables[0].value,
            SaveValue::SparseIntegers { dimensions, values }
                if dimensions == &[4, 5] && values == &[(6, 7), (18, 9)]
        ));
        assert!(matches!(
            &sparse.variables[1].value,
            SaveValue::SparseStrings { dimensions, values }
                if dimensions == &[3, 3, 4]
                    && values == &[(5, "first".into()), (32, "last".into())]
        ));
        assert_eq!(
            decode_binary(&encoded, SaveCodecLimits::default()).unwrap(),
            document
        );
    }

    #[test]
    fn zero_length_arrays_use_reference_end_of_data_encoding() {
        let limits = SaveCodecLimits::default();
        let mut integers = Vec::new();
        write_integer_array(&mut integers, &[0], &[], limits).unwrap();
        assert_eq!(integers, [0, 0, 0, 0, 0xFF]);
        assert_eq!(
            Cursor::new(&integers, limits).array(1, false).unwrap(),
            SaveValue::Integers {
                dimensions: vec![0],
                values: Vec::new(),
            }
        );
        assert_eq!(
            Cursor::new_sparse(&integers, limits)
                .array(1, false)
                .unwrap(),
            SaveValue::SparseIntegers {
                dimensions: vec![0],
                values: Vec::new(),
            }
        );

        let mut strings = Vec::new();
        write_string_array(&mut strings, &[2, 0], &[], limits).unwrap();
        assert_eq!(strings, [2, 0, 0, 0, 0, 0, 0, 0, 0xFF]);
        assert_eq!(
            Cursor::new(&strings, limits).array(2, true).unwrap(),
            SaveValue::Strings {
                dimensions: vec![2, 0],
                values: Vec::new(),
            }
        );
        assert_eq!(
            Cursor::new_sparse(&strings, limits).array(2, true).unwrap(),
            SaveValue::SparseStrings {
                dimensions: vec![2, 0],
                values: Vec::new(),
            }
        );

        let mut value_after_empty_shape = Cursor::new_sparse(&[0, 0, 0, 0, 1, 0xFF], limits);
        assert!(matches!(
            value_after_empty_shape.array(1, false),
            Err(SaveCodecError::InvalidFormat(message))
                if message == "array data exceeds dimensions"
        ));
    }

    #[test]
    fn sparse_array_reader_rejects_invalid_tokens_and_runs_past_the_shape() {
        let limits = SaveCodecLimits::default();
        let mut invalid_string = Cursor::new(&[1, 0, 0, 0, 0xD0], limits);
        assert!(matches!(
            invalid_string.array(1, true),
            Err(SaveCodecError::InvalidFormat(message)) if message == "invalid array token"
        ));

        let mut overflowing_run = Cursor::new(&[2, 0, 0, 0, 0xF0, 3, 0xFF], limits);
        assert!(matches!(
            overflowing_run.array(1, false),
            Err(SaveCodecError::InvalidFormat(message)) if message == "array run exceeds dimensions"
        ));
    }
}
