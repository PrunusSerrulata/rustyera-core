use super::encode::element_count;
use super::{EOC, EOF, SEPARATOR};
use crate::{SaveCodecError, SaveCodecLimits, SaveEntry, SaveValue};

pub(super) struct Cursor<'a> {
    data: &'a [u8],
    pub(super) position: usize,
    limits: SaveCodecLimits,
    entries: usize,
    sparse_arrays: bool,
}
impl<'a> Cursor<'a> {
    pub(super) fn new(data: &'a [u8], limits: SaveCodecLimits) -> Self {
        Self {
            data,
            position: 0,
            limits,
            entries: 0,
            sparse_arrays: false,
        }
    }
    pub(super) fn new_sparse(data: &'a [u8], limits: SaveCodecLimits) -> Self {
        Self {
            sparse_arrays: true,
            ..Self::new(data, limits)
        }
    }
    pub(super) fn remaining(&self) -> &'a [u8] {
        &self.data[self.position..]
    }
    pub(super) fn take(&mut self, count: usize) -> Result<&'a [u8], SaveCodecError> {
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
    pub(super) fn u8(&mut self) -> Result<u8, SaveCodecError> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn u32(&mut self) -> Result<u32, SaveCodecError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("exact")))
    }
    pub(super) fn u64(&mut self) -> Result<u64, SaveCodecError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("exact")))
    }
    pub(super) fn i64(&mut self) -> Result<i64, SaveCodecError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("exact")))
    }
    pub(super) fn packed_integer(&mut self, first: Option<u8>) -> Result<i64, SaveCodecError> {
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
    pub(super) fn seven_bit(&mut self) -> Result<usize, SaveCodecError> {
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
    pub(super) fn string(&mut self) -> Result<String, SaveCodecError> {
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
    pub(super) fn entries(&mut self, terminator: u8) -> Result<Vec<SaveEntry>, SaveCodecError> {
        self.entries_with_separator(terminator)
            .map(|(entries, _)| entries)
    }
    pub(super) fn character_entries(
        &mut self,
    ) -> Result<(Vec<SaveEntry>, Option<usize>), SaveCodecError> {
        self.entries_with_separator(EOC)
    }
    pub(super) fn entries_with_separator(
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
    pub(super) fn array(
        &mut self,
        rank: usize,
        strings: bool,
    ) -> Result<SaveValue, SaveCodecError> {
        let mut dimensions = Vec::with_capacity(rank);
        for _ in 0..rank {
            dimensions.push(self.u32()?);
        }
        let count = element_count(&dimensions, self.limits)?;
        if strings {
            if self.sparse_arrays {
                let values = self.sparse_entries(
                    &dimensions,
                    count,
                    |reader, tag| match tag {
                        0xD8 => reader.string().map(Some),
                        0..=0xCF => Ok(None),
                        _ => Err(SaveCodecError::InvalidFormat("invalid array token".into())),
                    },
                    String::is_empty,
                )?;
                return Ok(SaveValue::SparseStrings { dimensions, values });
            }
            let values = self.dense_array(&dimensions, count, |reader, tag| match tag {
                0xD8 => reader.string().map(Some),
                0..=0xCF => Ok(None),
                _ => Err(SaveCodecError::InvalidFormat("invalid array token".into())),
            })?;
            return Ok(SaveValue::Strings { dimensions, values });
        }
        if self.sparse_arrays {
            let values = self.sparse_entries(
                &dimensions,
                count,
                |reader, tag| reader.packed_integer(Some(tag)).map(Some),
                |value| *value == 0,
            )?;
            return Ok(SaveValue::SparseIntegers { dimensions, values });
        }
        let values = self.dense_array(&dimensions, count, |reader, tag| {
            reader.packed_integer(Some(tag)).map(Some)
        })?;
        Ok(SaveValue::Integers { dimensions, values })
    }

    fn dense_array<T: Default>(
        &mut self,
        dimensions: &[u32],
        count: usize,
        decode_value: impl FnMut(&mut Self, u8) -> Result<Option<T>, SaveCodecError>,
    ) -> Result<Vec<T>, SaveCodecError> {
        let mut values = Vec::new();
        values.resize_with(count, T::default);
        self.walk_sparse_array(dimensions, count, decode_value, |index, value| {
            values[index] = value;
        })?;
        Ok(values)
    }

    fn sparse_entries<T>(
        &mut self,
        dimensions: &[u32],
        count: usize,
        decode_value: impl FnMut(&mut Self, u8) -> Result<Option<T>, SaveCodecError>,
        is_default: impl Fn(&T) -> bool,
    ) -> Result<Vec<(u64, T)>, SaveCodecError> {
        let mut values = Vec::new();
        self.walk_sparse_array(dimensions, count, decode_value, |index, value| {
            if !is_default(&value) {
                values.push((index as u64, value));
            }
        })?;
        Ok(values)
    }

    fn walk_sparse_array<T>(
        &mut self,
        dimensions: &[u32],
        count: usize,
        mut decode_value: impl FnMut(&mut Self, u8) -> Result<Option<T>, SaveCodecError>,
        mut store: impl FnMut(usize, T),
    ) -> Result<(), SaveCodecError> {
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
                tag => {
                    if index >= count {
                        return Err(SaveCodecError::InvalidFormat(
                            "array data exceeds dimensions".into(),
                        ));
                    }
                    if let Some(value) = decode_value(self, tag)? {
                        store(index, value);
                    }
                    index += 1;
                }
            }
            if index > count {
                return Err(SaveCodecError::InvalidFormat(
                    "array run exceeds dimensions".into(),
                ));
            }
        }
        Ok(())
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
