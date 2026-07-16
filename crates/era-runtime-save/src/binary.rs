use std::io::{Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::{
    OpaqueSaveExtension, SaveCodecError, SaveCodecLimits, SaveDocument, SaveEntry, SaveFileKind,
    SaveFormat, SaveMetadata, SaveValue,
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
    if matches!(kind, SaveFileKind::Normal | SaveFileKind::Character) {
        let count = usize::try_from(reader.i64()?)
            .map_err(|_| SaveCodecError::InvalidFormat("negative character count".into()))?;
        if count > limits.maximum_characters {
            return Err(SaveCodecError::LimitExceeded("maximum characters"));
        }
        for _ in 0..count {
            characters.push(reader.entries(EOC)?);
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
        for character in &document.characters {
            write_entries(&mut body, character, limits)?;
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
    for (index, value) in values.iter().enumerate() {
        write_packed_integer(output, *value);
        write_boundaries(output, dimensions, index + 1);
    }
    output.push(0xFF);
    Ok(())
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
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() {
            // String arrays use the explicit zero-run token. A literal zero is an integer-array
            // token and is not accepted by the reference string-array reader.
            output.push(0xF0);
            write_packed_integer(output, 1);
        } else {
            output.push(0xD8);
            write_string(output, value, limits)?;
        }
        write_boundaries(output, dimensions, index + 1);
    }
    output.push(0xFF);
    Ok(())
}

fn write_boundaries(output: &mut Vec<u8>, dimensions: &[u32], next: usize) {
    let last = usize::try_from(dimensions.last().copied().unwrap_or(1)).unwrap_or(usize::MAX);
    if dimensions.len() >= 2 && next.is_multiple_of(last) {
        output.push(0xE0);
    }
    if dimensions.len() == 3 {
        let plane = usize::try_from(dimensions[1])
            .unwrap_or(usize::MAX)
            .saturating_mul(usize::try_from(dimensions[2]).unwrap_or(usize::MAX));
        if next.is_multiple_of(plane) {
            output.push(0xE1);
        }
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
        let mut result = Vec::new();
        loop {
            let tag = self.u8()?;
            if tag == terminator {
                break;
            }
            if tag == SEPARATOR {
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
        Ok(result)
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
                0xE0 | 0xE1 => {}
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
