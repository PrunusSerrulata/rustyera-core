use std::io::{Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveExtension,
    SaveFileKind, SaveFormat, SaveMetadata, SaveValue,
};

const HEADER: u64 = 0x0A1A_0A0D_4152_4589;
const ZIP_HEADER: u64 = 0x0A50_495A_4152_4589;
const VERSION: u32 = 1808;
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
#[allow(clippy::too_many_lines)]
pub fn decode_binary(data: &[u8], limits: SaveCodecLimits) -> Result<SaveDocument, SaveCodecError> {
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
        let mut decoded = Vec::new();
        GzDecoder::new(outer.remaining())
            .take((limits.maximum_bytes + 1) as u64)
            .read_to_end(&mut decoded)
            .map_err(|error| SaveCodecError::Compression(error.to_string()))?;
        if decoded.len() > limits.maximum_bytes {
            return Err(SaveCodecError::LimitExceeded("decompressed bytes"));
        }
        decoded
    } else {
        outer.remaining().to_vec()
    };
    let mut reader = Cursor::new(&body, limits);
    let kind = match reader.u8()? {
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
    let variables = if reader.remaining().is_empty() {
        Vec::new()
    } else {
        reader.entries(EOF)?
    };
    let mut opaque_extensions = Vec::new();
    if matches!(kind, SaveFileKind::Normal | SaveFileKind::Global) && !reader.remaining().is_empty()
    {
        while !reader.remaining().is_empty() {
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

/// Decode one already-delimited Emuera extension record without losing entry order.
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
            SaveValue::Integers { dimensions, .. } => integer_tag(dimensions.len())?,
            SaveValue::Strings { dimensions, .. } => string_tag(dimensions.len())?,
        };
        output.push(tag);
        write_string(output, &entry.name, limits)?;
        match &entry.value {
            SaveValue::Integer(value) => write_packed_integer(output, *value),
            SaveValue::String(value) => write_string(output, value, limits)?,
            SaveValue::Integers { dimensions, values } => {
                write_integer_array(output, dimensions, values, limits)?;
            }
            SaveValue::Strings { dimensions, values } => {
                write_string_array(output, dimensions, values, limits)?;
            }
        }
    }
    Ok(())
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

fn element_count(dimensions: &[u32], limits: SaveCodecLimits) -> Result<usize, SaveCodecError> {
    if dimensions.contains(&0) {
        return Err(SaveCodecError::InvalidFormat(
            "array dimensions must be positive".into(),
        ));
    }
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

fn write_integer_array(
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

fn write_string_array(
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

struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
    limits: SaveCodecLimits,
    entries: usize,
}
impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], limits: SaveCodecLimits) -> Self {
        Self {
            data,
            position: 0,
            limits,
            entries: 0,
        }
    }
    fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], SaveCodecError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| SaveCodecError::InvalidFormat("offset overflow".into()))?;
        let value = self
            .data
            .get(self.position..end)
            .ok_or_else(|| SaveCodecError::InvalidFormat("truncated save".into()))?;
        self.position = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, SaveCodecError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, SaveCodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("exact")))
    }
    fn u64(&mut self) -> Result<u64, SaveCodecError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("exact")))
    }
    fn i64(&mut self) -> Result<i64, SaveCodecError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("exact")))
    }
    fn packed_integer(&mut self, first: Option<u8>) -> Result<i64, SaveCodecError> {
        let tag = first.map_or_else(|| self.u8(), Ok)?;
        match tag {
            0..=0xCF => Ok(i64::from(tag)),
            0xD0 => Ok(i64::from(i16::from_le_bytes(
                self.take(2)?.try_into().expect("exact"),
            ))),
            0xD1 => Ok(i64::from(i32::from_le_bytes(
                self.take(4)?.try_into().expect("exact"),
            ))),
            0xD2 => self.i64(),
            _ => Err(SaveCodecError::InvalidFormat(
                "invalid packed integer".into(),
            )),
        }
    }
    fn seven_bit(&mut self) -> Result<usize, SaveCodecError> {
        let mut result = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = self.u8()?;
            result |= usize::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
        Err(SaveCodecError::InvalidFormat(
            "invalid string length".into(),
        ))
    }
    fn string(&mut self) -> Result<String, SaveCodecError> {
        let length = self.seven_bit()?;
        if length > self.limits.maximum_string_bytes || length % 2 != 0 {
            return Err(SaveCodecError::LimitExceeded("string bytes"));
        }
        let units = self
            .take(length)?
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units)
            .map_err(|_| SaveCodecError::InvalidFormat("invalid UTF-16 string".into()))
    }
    fn entries(&mut self, terminator: u8) -> Result<Vec<SaveEntry>, SaveCodecError> {
        self.entries_with_separator(terminator)
            .map(|(entries, _)| entries)
    }
    fn character_entries(&mut self) -> Result<(Vec<SaveEntry>, Option<usize>), SaveCodecError> {
        self.entries_with_separator(EOC)
    }
    fn entries_with_separator(
        &mut self,
        terminator: u8,
    ) -> Result<(Vec<SaveEntry>, Option<usize>), SaveCodecError> {
        let mut result = Vec::new();
        let mut separator = None;
        loop {
            let tag = self.u8()?;
            if tag == terminator {
                break;
            }
            if tag == SEPARATOR {
                if separator.replace(result.len()).is_some() {
                    return Err(SaveCodecError::InvalidFormat(
                        "duplicate character section separator".into(),
                    ));
                }
                continue;
            }
            if tag == EOF || tag == EOC {
                return Err(SaveCodecError::InvalidFormat(
                    "unexpected section terminator".into(),
                ));
            }
            self.entries += 1;
            if self.entries > self.limits.maximum_entries {
                return Err(SaveCodecError::LimitExceeded("maximum entries"));
            }
            let name = self.string()?;
            let value = match tag {
                0x00 => SaveValue::Integer(self.packed_integer(None)?),
                0x10 => SaveValue::String(self.string()?),
                0x01..=0x03 => self.array(tag as usize, false)?,
                0x11..=0x13 => self.array((tag - 0x10) as usize, true)?,
                _ => {
                    return Err(SaveCodecError::InvalidFormat(format!(
                        "unknown variable type {tag:#x}"
                    )));
                }
            };
            result.push(SaveEntry { name, value });
        }
        Ok((result, separator))
    }
    fn array(&mut self, rank: usize, strings: bool) -> Result<SaveValue, SaveCodecError> {
        let mut dimensions = Vec::with_capacity(rank);
        for _ in 0..rank {
            dimensions.push(self.u32()?);
        }
        let count = element_count(&dimensions, self.limits)?;
        let mut ints = vec![0i64; count];
        let mut strs = vec![String::new(); count];
        let mut index = 0usize;
        loop {
            let tag = self.u8()?;
            match tag {
                0xFF => break,
                0xE0 => {
                    let row = *dimensions.last().unwrap_or(&1) as usize;
                    index = align_to_next_boundary(index, row);
                }
                0xE1 => {
                    let plane = dimensions.iter().skip(1).fold(1usize, |value, dimension| {
                        value.saturating_mul(*dimension as usize)
                    });
                    index = align_to_next_boundary(index, plane);
                }
                0xF0 => {
                    let zeroes = usize::try_from(self.packed_integer(None)?)
                        .map_err(|_| SaveCodecError::InvalidFormat("negative zero run".into()))?;
                    index = index.saturating_add(zeroes);
                }
                0xF1 => {
                    let rows = usize::try_from(self.packed_integer(None)?)
                        .map_err(|_| SaveCodecError::InvalidFormat("negative row run".into()))?;
                    index = index.saturating_add(
                        rows.saturating_mul(*dimensions.last().unwrap_or(&1) as usize),
                    );
                }
                0xF2 => {
                    let planes = usize::try_from(self.packed_integer(None)?)
                        .map_err(|_| SaveCodecError::InvalidFormat("negative plane run".into()))?;
                    let plane = dimensions.iter().skip(1).fold(1usize, |value, dimension| {
                        value.saturating_mul(*dimension as usize)
                    });
                    index = index.saturating_add(planes.saturating_mul(plane));
                }
                0xD8 if strings => {
                    if index >= count {
                        return Err(SaveCodecError::InvalidFormat(
                            "array data exceeds dimensions".into(),
                        ));
                    }
                    strs[index] = self.string()?;
                    index += 1;
                }
                tag if !strings => {
                    if index >= count {
                        return Err(SaveCodecError::InvalidFormat(
                            "array data exceeds dimensions".into(),
                        ));
                    }
                    ints[index] = self.packed_integer(Some(tag))?;
                    index += 1;
                }
                0..=0xCF if strings => {
                    index += 1;
                }
                _ => return Err(SaveCodecError::InvalidFormat("invalid array token".into())),
            }
            if index > count {
                return Err(SaveCodecError::InvalidFormat(
                    "array run exceeds dimensions".into(),
                ));
            }
        }
        Ok(if strings {
            SaveValue::Strings {
                dimensions,
                values: strs,
            }
        } else {
            SaveValue::Integers {
                dimensions,
                values: ints,
            }
        })
    }
}

fn align_to_next_boundary(index: usize, boundary: usize) -> usize {
    let remainder = index % boundary.max(1);
    if remainder == 0 {
        index
    } else {
        index.saturating_add(boundary - remainder)
    }
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
        assert_eq!(
            decode_binary(&encoded, SaveCodecLimits::default()).unwrap(),
            document
        );
    }
}
