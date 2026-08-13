use std::io::Write;

use flate2::{Compression, write::GzEncoder};

use crate::format::{HEADER, VERSION, ZIP_HEADER};
use crate::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveExtension,
    SaveFileKind, SaveFormat, SaveValue,
};

use super::{EOC, EOF, SEPARATOR};

/// Encode an Emuera 1808 binary or compressed-binary save.
///
/// # Errors
///
/// Returns an error when values cannot be represented or a resource limit is exceeded.
pub fn encode_binary(
    document: &SaveDocument,
    format: SaveFormat,
    limits: SaveCodecLimits,
) -> Result<Vec<u8>, SaveCodecError> {
    if document.characters.len() > limits.maximum_characters {
        return Err(SaveCodecError::LimitExceeded("maximum characters"));
    }
    if document.character_user_defined_starts.len() != document.characters.len() {
        return Err(SaveCodecError::InvalidFormat(
            "character section offsets differ from character count".into(),
        ));
    }
    let mut body = Vec::new();
    body.push(document.kind as u8);
    body.extend(document.metadata.unique_code.to_le_bytes());
    body.extend(document.metadata.version.to_le_bytes());
    write_string(&mut body, &document.metadata.description, limits)?;
    if matches!(
        document.kind,
        SaveFileKind::Normal | SaveFileKind::Character
    ) {
        let count = i64::try_from(document.characters.len())
            .map_err(|_| SaveCodecError::LimitExceeded("maximum characters"))?;
        body.extend(count.to_le_bytes());
        for (character, user_defined_start) in document
            .characters
            .iter()
            .zip(&document.character_user_defined_starts)
        {
            write_character_entries(&mut body, character, *user_defined_start, limits)?;
            body.push(EOC);
        }
    }
    write_entries(&mut body, &document.variables, limits)?;
    body.push(EOF);
    if matches!(document.kind, SaveFileKind::Normal | SaveFileKind::Global) {
        for extension in &document.opaque_extensions {
            if !(0x20..=0x22).contains(&extension.type_tag) {
                return Err(SaveCodecError::InvalidFormat(
                    "invalid extension tag".into(),
                ));
            }
            body.push(extension.type_tag);
            write_string(&mut body, &extension.key, limits)?;
            body.extend(&extension.payload);
        }
        body.push(EOF);
    }
    let compressed = format == SaveFormat::Binary1808Gzip;
    let payload = if compressed {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&body)
            .map_err(|error| SaveCodecError::Compression(error.to_string()))?;
        encoder
            .finish()
            .map_err(|error| SaveCodecError::Compression(error.to_string()))?
    } else {
        body
    };
    let mut output = Vec::with_capacity(16 + payload.len());
    output.extend(if compressed { ZIP_HEADER } else { HEADER }.to_le_bytes());
    output.extend(VERSION.to_le_bytes());
    output.extend(0u32.to_le_bytes());
    output.extend(payload);
    if output.len() > limits.maximum_bytes {
        return Err(SaveCodecError::LimitExceeded("maximum bytes"));
    }
    Ok(output)
}

/// Encode one typed Emuera extension as the exact payload consumed by the binary codec.
///
/// # Errors
///
/// Returns an error when a string or entry count exceeds the configured limits.
pub fn encode_save_extension(
    extension: &SaveExtension,
    limits: SaveCodecLimits,
) -> Result<OpaqueSaveExtension, SaveCodecError> {
    let (type_tag, key, payload) = match extension {
        SaveExtension::Map { key, entries } => {
            if entries.len() > limits.maximum_entries {
                return Err(SaveCodecError::LimitExceeded("map entries"));
            }
            let count = u32::try_from(entries.len())
                .map_err(|_| SaveCodecError::LimitExceeded("map entries"))?;
            let mut payload = count.to_le_bytes().to_vec();
            for (entry_key, value) in entries {
                write_string(&mut payload, entry_key, limits)?;
                write_string(&mut payload, value, limits)?;
            }
            (0x20, key, payload)
        }
        SaveExtension::Xml { key, document } => {
            let mut payload = Vec::new();
            write_string(&mut payload, document, limits)?;
            (0x21, key, payload)
        }
        SaveExtension::DataTable { key, schema, data } => {
            let mut payload = Vec::new();
            write_string(&mut payload, schema, limits)?;
            write_string(&mut payload, data, limits)?;
            (0x22, key, payload)
        }
    };
    Ok(OpaqueSaveExtension {
        type_tag,
        key: key.clone(),
        payload,
    })
}

fn write_entries(
    output: &mut Vec<u8>,
    entries: &[SaveEntry],
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    if entries.len() > limits.maximum_entries {
        return Err(SaveCodecError::LimitExceeded("maximum entries"));
    }
    for entry in entries {
        let tag = match &entry.value {
            SaveValue::Integer(_) => 0x00,
            SaveValue::String(_) => 0x10,
            SaveValue::Integers { dimensions, .. }
            | SaveValue::SparseIntegers { dimensions, .. } => integer_tag(dimensions.len())?,
            SaveValue::Strings { dimensions, .. } | SaveValue::SparseStrings { dimensions, .. } => {
                string_tag(dimensions.len())?
            }
        };
        output.push(tag);
        write_string(output, &entry.name, limits)?;
        match &entry.value {
            SaveValue::Integer(value) => write_packed_integer(output, *value),
            SaveValue::String(value) => write_string(output, value, limits)?,
            SaveValue::Integers { dimensions, values } => {
                write_integer_array(output, dimensions, values, limits)?;
            }
            SaveValue::SparseIntegers { dimensions, values } => {
                let dense = materialize_sparse_integers(dimensions, values, limits)?;
                write_integer_array(output, dimensions, &dense, limits)?;
            }
            SaveValue::Strings { dimensions, values } => {
                write_string_array(output, dimensions, values, limits)?;
            }
            SaveValue::SparseStrings { dimensions, values } => {
                let dense = materialize_sparse_strings(dimensions, values, limits)?;
                write_string_array(output, dimensions, &dense, limits)?;
            }
        }
    }
    Ok(())
}

fn materialize_sparse_integers(
    dimensions: &[u32],
    entries: &[(u64, i64)],
    limits: SaveCodecLimits,
) -> Result<Vec<i64>, SaveCodecError> {
    let mut values = vec![0; element_count(dimensions, limits)?];
    for (index, value) in entries {
        let index =
            usize::try_from(*index).map_err(|_| SaveCodecError::LimitExceeded("array elements"))?;
        let target = values.get_mut(index).ok_or_else(|| {
            SaveCodecError::InvalidFormat("sparse array index exceeds dimensions".into())
        })?;
        *target = *value;
    }
    Ok(values)
}

fn materialize_sparse_strings(
    dimensions: &[u32],
    entries: &[(u64, String)],
    limits: SaveCodecLimits,
) -> Result<Vec<String>, SaveCodecError> {
    let count = element_count(dimensions, limits)?;
    let mut values = Vec::new();
    values.resize_with(count, String::new);
    for (index, value) in entries {
        let index =
            usize::try_from(*index).map_err(|_| SaveCodecError::LimitExceeded("array elements"))?;
        let target = values.get_mut(index).ok_or_else(|| {
            SaveCodecError::InvalidFormat("sparse array index exceeds dimensions".into())
        })?;
        target.clone_from(value);
    }
    Ok(values)
}

fn write_character_entries(
    output: &mut Vec<u8>,
    entries: &[SaveEntry],
    user_defined_start: Option<usize>,
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    let Some(start) = user_defined_start else {
        return write_entries(output, entries, limits);
    };
    if start > entries.len() {
        return Err(SaveCodecError::InvalidFormat(
            "character user-defined section exceeds entry count".into(),
        ));
    }
    write_entries(output, &entries[..start], limits)?;
    output.push(SEPARATOR);
    write_entries(output, &entries[start..], limits)
}

fn integer_tag(dimensions: usize) -> Result<u8, SaveCodecError> {
    match dimensions {
        1 => Ok(1),
        2 => Ok(2),
        3 => Ok(3),
        _ => Err(SaveCodecError::InvalidFormat(
            "integer array dimension must be 1..=3".into(),
        )),
    }
}
fn string_tag(dimensions: usize) -> Result<u8, SaveCodecError> {
    integer_tag(dimensions).map(|tag| tag + 0x10)
}

pub(super) fn element_count(
    dimensions: &[u32],
    limits: SaveCodecLimits,
) -> Result<usize, SaveCodecError> {
    let count = dimensions
        .iter()
        .try_fold(1usize, |count, dimension| {
            count.checked_mul(*dimension as usize)
        })
        .ok_or(SaveCodecError::LimitExceeded("array elements"))?;
    if count > limits.maximum_elements {
        return Err(SaveCodecError::LimitExceeded("array elements"));
    }
    Ok(count)
}

fn write_dimensions(output: &mut Vec<u8>, dimensions: &[u32]) {
    for dimension in dimensions {
        output.extend(dimension.to_le_bytes());
    }
}

pub(super) fn write_integer_array(
    output: &mut Vec<u8>,
    dimensions: &[u32],
    values: &[i64],
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    let count = element_count(dimensions, limits)?;
    if values.len() != count {
        return Err(SaveCodecError::InvalidFormat(
            "integer array length differs from dimensions".into(),
        ));
    }
    write_dimensions(output, dimensions);
    write_sparse_array(
        output,
        dimensions,
        values,
        |value| *value == 0,
        |output, value| {
            write_packed_integer(output, *value);
            Ok(())
        },
    )
}

pub(super) fn write_string_array(
    output: &mut Vec<u8>,
    dimensions: &[u32],
    values: &[String],
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    let count = element_count(dimensions, limits)?;
    if values.len() != count {
        return Err(SaveCodecError::InvalidFormat(
            "string array length differs from dimensions".into(),
        ));
    }
    write_dimensions(output, dimensions);
    write_sparse_array(
        output,
        dimensions,
        values,
        String::is_empty,
        |output, value| {
            output.push(0xD8);
            write_string(output, value, limits)
        },
    )
}

fn write_sparse_array<T>(
    output: &mut Vec<u8>,
    dimensions: &[u32],
    values: &[T],
    is_zero: impl Fn(&T) -> bool,
    mut write_value: impl FnMut(&mut Vec<u8>, &T) -> Result<(), SaveCodecError>,
) -> Result<(), SaveCodecError> {
    // Emuera writes a zero-length array as its dimensions followed directly by EoD.
    // Avoid calling `chunks(0)` when any declared dimension is zero.
    if values.is_empty() {
        output.push(0xFF);
        return Ok(());
    }
    let row_length = dimensions.last().copied().unwrap_or(1) as usize;
    let rows_per_plane = dimensions.get(1).copied().unwrap_or(1) as usize;
    let plane_length = if dimensions.len() == 3 {
        row_length.saturating_mul(rows_per_plane)
    } else {
        values.len()
    };

    let mut zero_planes = 0usize;
    for plane in values.chunks(plane_length) {
        if dimensions.len() == 3 && plane.iter().all(&is_zero) {
            zero_planes += 1;
            continue;
        }
        write_zero_run(output, 0xF2, zero_planes);
        zero_planes = 0;

        let mut zero_rows = 0usize;
        for row in plane.chunks(row_length) {
            if dimensions.len() >= 2 && row.iter().all(&is_zero) {
                zero_rows += 1;
                continue;
            }
            write_zero_run(output, 0xF1, zero_rows);
            zero_rows = 0;

            let mut zeroes = 0usize;
            for value in row {
                if is_zero(value) {
                    zeroes += 1;
                } else {
                    write_zero_run(output, 0xF0, zeroes);
                    zeroes = 0;
                    write_value(output, value)?;
                }
            }
            if dimensions.len() >= 2 {
                output.push(0xE0);
            }
        }
        if dimensions.len() == 3 {
            output.push(0xE1);
        }
    }
    output.push(0xFF);
    Ok(())
}

fn write_zero_run(output: &mut Vec<u8>, tag: u8, count: usize) {
    if count != 0 {
        output.push(tag);
        write_packed_integer(
            output,
            i64::try_from(count).expect("validated array element count fits i64"),
        );
    }
}

fn write_packed_integer(output: &mut Vec<u8>, value: i64) {
    if (0..=0xCF).contains(&value) {
        output.push(u8::try_from(value).expect("packed byte range checked"));
    } else if i16::try_from(value).is_ok() {
        output.push(0xD0);
        output.extend(
            i16::try_from(value)
                .expect("i16 range checked")
                .to_le_bytes(),
        );
    } else if i32::try_from(value).is_ok() {
        output.push(0xD1);
        output.extend(
            i32::try_from(value)
                .expect("i32 range checked")
                .to_le_bytes(),
        );
    } else {
        output.push(0xD2);
        output.extend(value.to_le_bytes());
    }
}

fn write_string(
    output: &mut Vec<u8>,
    value: &str,
    limits: SaveCodecLimits,
) -> Result<(), SaveCodecError> {
    let bytes: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
    if bytes.len() > limits.maximum_string_bytes {
        return Err(SaveCodecError::LimitExceeded("string bytes"));
    }
    write_7bit(output, bytes.len());
    output.extend(bytes);
    Ok(())
}

fn write_7bit(output: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = u8::try_from(value & 0x7F).expect("seven-bit chunk fits u8");
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}
