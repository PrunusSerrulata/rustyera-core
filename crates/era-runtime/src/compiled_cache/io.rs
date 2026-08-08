use std::io::{Read, Write};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::de::DeserializeOwned;

use super::{EncodedSectionRef, TARGET_PARALLEL_SECTIONS};

pub(super) fn write_varint(writer: &mut dyn Write, mut value: u64) -> Result<(), String> {
    let mut bytes = [0_u8; 10];
    let mut length = 0;
    while value >= 0x80 {
        bytes[length] = u8::try_from(value & 0x7f).expect("masked varint byte fits in u8") | 0x80;
        value >>= 7;
        length += 1;
    }
    bytes[length] = u8::try_from(value).expect("final varint byte fits in u8");
    writer
        .write_all(&bytes[..=length])
        .map_err(|error| error.to_string())
}

pub(super) fn read_stream_varint(reader: &mut dyn Read) -> Result<u64, String> {
    let mut value = 0_u64;
    for index in 0..10 {
        let mut byte = [0_u8; 1];
        reader
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        if index == 9 && byte[0] > 1 {
            return Err("compiled cache varint overflows u64".into());
        }
        value |= u64::from(byte[0] & 0x7f) << (index * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("compiled cache varint is too long".into())
}

pub(super) fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut value = 0_u64;
    for index in 0..10 {
        let byte = *bytes
            .get(*cursor)
            .ok_or("compiled cache source varint is truncated")?;
        *cursor += 1;
        if index == 9 && byte > 1 {
            return Err("compiled cache source varint overflows u64".into());
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("compiled cache source varint is too long".into())
}

pub(super) fn read_section<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    digest_offset: usize,
) -> Result<EncodedSectionRef<'a>, String> {
    let decoded_length = read_u64(bytes, cursor)?;
    let compressed_length = usize::try_from(read_u64(bytes, cursor)?)
        .map_err(|_| "compiled cache section is not addressable")?;
    let end = cursor
        .checked_add(compressed_length)
        .ok_or("compiled cache section length overflow")?;
    if end > digest_offset {
        return Err("compiled cache section is truncated".into());
    }
    let compressed = &bytes[*cursor..end];
    *cursor = end;
    Ok(EncodedSectionRef {
        decoded_length,
        compressed,
    })
}

pub(super) fn decode_section<T: DeserializeOwned>(
    section: &EncodedSectionRef<'_>,
) -> Result<T, String> {
    decode_raw_section(section, |reader| {
        rmp_serde::from_read(reader).map_err(|error| error.to_string())
    })
}

pub(super) fn encode_raw_section(
    cancelled: Option<&AtomicBool>,
    encode: impl FnOnce(&mut dyn Write) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    let encoder = zstd::stream::Encoder::new(Vec::new(), super::COMPRESSION_LEVEL)
        .map_err(|error| error.to_string())?;
    let mut writer = CountingWriter::new(encoder, cancelled);
    encode(&mut writer)?;
    let decoded_length = writer.bytes;
    let compressed = writer
        .into_inner()
        .finish()
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(16 + compressed.len());
    output.extend_from_slice(&decoded_length.to_le_bytes());
    output.extend_from_slice(
        &u64::try_from(compressed.len())
            .map_err(|_| "compiled cache section is too large")?
            .to_le_bytes(),
    );
    output.extend_from_slice(&compressed);
    Ok(output)
}

pub(super) fn decode_raw_section<T>(
    section: &EncodedSectionRef<'_>,
    decode: impl FnOnce(&mut dyn Read) -> Result<T, String>,
) -> Result<T, String> {
    let decoder =
        zstd::stream::read::Decoder::new(section.compressed).map_err(|error| error.to_string())?;
    let mut reader = CountingReader::new(decoder.take(section.decoded_length.saturating_add(1)));
    let value = decode(&mut reader)?;
    let mut tail = [0_u8; 1];
    if reader.read(&mut tail).map_err(|error| error.to_string())? != 0
        || reader.bytes != section.decoded_length
    {
        return Err("compiled cache decoded section length differs".into());
    }
    Ok(value)
}

pub(super) fn equal_ranges(length: usize) -> Vec<Range<usize>> {
    if length == 0 {
        return Vec::new();
    }
    let chunk_length = length.div_ceil(TARGET_PARALLEL_SECTIONS);
    (0..length)
        .step_by(chunk_length)
        .map(|start| start..start.saturating_add(chunk_length).min(length))
        .collect()
}

pub(super) fn read_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let end = cursor.saturating_add(4);
    let value = bytes
        .get(*cursor..end)
        .ok_or("compiled project cache is truncated")?;
    *cursor = end;
    Ok(u32::from_le_bytes(
        value.try_into().expect("four-byte slice"),
    ))
}

pub(super) fn read_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let end = cursor.saturating_add(8);
    let value = bytes
        .get(*cursor..end)
        .ok_or("compiled project cache is truncated")?;
    *cursor = end;
    Ok(u64::from_le_bytes(
        value.try_into().expect("eight-byte slice"),
    ))
}

pub(super) struct CountingWriter<'a, W> {
    inner: W,
    pub(super) bytes: u64,
    cancelled: Option<&'a AtomicBool>,
}

impl<'a, W> CountingWriter<'a, W> {
    pub(super) const fn new(inner: W, cancelled: Option<&'a AtomicBool>) -> Self {
        Self {
            inner,
            bytes: 0,
            cancelled,
        }
    }

    pub(super) fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self
            .cancelled
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "compiled cache build cancelled",
            ));
        }
        let written = self.inner.write(buffer)?;
        self.bytes = self.bytes.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(super) struct CountingReader<R> {
    inner: R,
    bytes: u64,
}

impl<R> CountingReader<R> {
    const fn new(inner: R) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes = self.bytes.saturating_add(read as u64);
        Ok(read)
    }
}

pub(super) struct HashWriter {
    hasher: blake3::Hasher,
}

impl HashWriter {
    pub(super) fn new(domain: &str) -> Self {
        Self {
            hasher: blake3::Hasher::new_derive_key(domain),
        }
    }

    pub(super) fn finish(self) -> [u8; 32] {
        *self.hasher.finalize().as_bytes()
    }
}

impl Write for HashWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
