use std::io::{self, Write};
use std::ops::Range;

const CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Container storage that can avoid allocating a contiguous full-project buffer.
#[derive(Debug)]
pub(crate) enum ContainerBytes {
    Contiguous(Vec<u8>),
    Segmented { chunks: Vec<Vec<u8>>, len: usize },
}

impl Default for ContainerBytes {
    fn default() -> Self {
        Self::Contiguous(Vec::new())
    }
}

impl ContainerBytes {
    pub(crate) fn new(segmented: bool, initial: Vec<u8>) -> Self {
        if !segmented {
            return Self::Contiguous(initial);
        }
        let len = initial.len();
        let chunks = if initial.is_empty() {
            Vec::new()
        } else if initial.capacity() <= CHUNK_BYTES {
            vec![initial]
        } else {
            initial.chunks(CHUNK_BYTES).map(<[u8]>::to_vec).collect()
        };
        Self::Segmented { chunks, len }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Contiguous(bytes) => bytes.len(),
            Self::Segmented { len, .. } => *len,
        }
    }

    pub(crate) fn extend_from_slice(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Contiguous(output) => {
                output
                    .try_reserve(bytes.len())
                    .map_err(|error| error.to_string())?;
                output.extend_from_slice(bytes);
            }
            Self::Segmented { chunks, len } => {
                len.checked_add(bytes.len())
                    .ok_or("container length overflow")?;
                while !bytes.is_empty() {
                    if chunks.last().is_none_or(|chunk| chunk.len() == CHUNK_BYTES) {
                        let mut chunk = Vec::new();
                        chunk
                            .try_reserve_exact(CHUNK_BYTES)
                            .map_err(|error| error.to_string())?;
                        chunks.try_reserve(1).map_err(|error| error.to_string())?;
                        chunks.push(chunk);
                    }
                    let chunk = chunks.last_mut().expect("container chunk was allocated");
                    let count = bytes.len().min(CHUNK_BYTES - chunk.len());
                    // Reserve only one fixed-size block, never a geometrically growing payload.
                    chunk
                        .try_reserve_exact(CHUNK_BYTES - chunk.len())
                        .map_err(|error| error.to_string())?;
                    chunk.extend_from_slice(&bytes[..count]);
                    *len += count;
                    bytes = &bytes[count..];
                }
            }
        }
        Ok(())
    }

    pub(crate) fn patch(&mut self, offset: usize, bytes: &[u8]) -> Result<(), String> {
        let end = offset
            .checked_add(bytes.len())
            .ok_or("container patch overflow")?;
        if end > self.len() {
            return Err("container patch is out of bounds".into());
        }
        match self {
            Self::Contiguous(output) => output[offset..end].copy_from_slice(bytes),
            Self::Segmented { chunks, .. } => {
                let mut base = 0;
                for chunk in chunks {
                    let chunk_end = base + chunk.len();
                    let start = offset.max(base);
                    let stop = end.min(chunk_end);
                    if start < stop {
                        chunk[start - base..stop - base]
                            .copy_from_slice(&bytes[start - offset..stop - offset]);
                    }
                    base = chunk_end;
                    if base >= end {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn hash_range(
        &self,
        range: Range<usize>,
        hasher: &mut blake3::Hasher,
    ) -> Result<(), String> {
        self.visit_range(range, |bytes| {
            hasher.update(bytes);
        })
    }

    pub(crate) fn copy_range(&self, range: Range<usize>) -> Vec<u8> {
        assert!(
            range.start <= range.end && range.end <= self.len(),
            "container range is out of bounds"
        );
        let mut output = Vec::with_capacity(range.len());
        self.visit_range(range, |bytes| output.extend_from_slice(bytes))
            .expect("container range was checked");
        output
    }

    pub(crate) fn into_vec(self) -> Vec<u8> {
        match self {
            Self::Contiguous(bytes) => bytes,
            Self::Segmented { mut chunks, len } => {
                if chunks.len() == 1 {
                    return chunks.pop().expect("one container chunk exists");
                }
                let mut output = Vec::with_capacity(len);
                for chunk in chunks {
                    output.extend_from_slice(&chunk);
                }
                output
            }
        }
    }

    fn visit_range(&self, range: Range<usize>, mut visit: impl FnMut(&[u8])) -> Result<(), String> {
        if range.start > range.end || range.end > self.len() {
            return Err("container range is out of bounds".into());
        }
        match self {
            Self::Contiguous(bytes) => visit(&bytes[range]),
            Self::Segmented { chunks, .. } => {
                let mut base = 0;
                for chunk in chunks {
                    let chunk_end = base + chunk.len();
                    let start = range.start.max(base);
                    let end = range.end.min(chunk_end);
                    if start < end {
                        visit(&chunk[start - base..end - base]);
                    }
                    base = chunk_end;
                    if base >= range.end {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

impl Write for ContainerBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.extend_from_slice(bytes).map_err(io::Error::other)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmented_storage_patches_hashes_and_copies_across_bounded_chunks() {
        let mut expected = vec![7; CHUNK_BYTES * 2 + 17];
        let mut output = ContainerBytes::new(true, expected[..3].to_vec());
        output.write_all(&expected[3..]).unwrap();
        let patch = [1, 2, 3, 4, 5];
        let offset = CHUNK_BYTES - 2;
        output.patch(offset, &patch).unwrap();
        expected[offset..offset + patch.len()].copy_from_slice(&patch);
        let range = CHUNK_BYTES - 3..CHUNK_BYTES * 2 + 4;
        assert_eq!(output.copy_range(range.clone()), expected[range.clone()]);
        let mut hasher = blake3::Hasher::new();
        output.hash_range(range.clone(), &mut hasher).unwrap();
        assert_eq!(hasher.finalize(), blake3::hash(&expected[range]));
        assert_eq!(output.len(), expected.len());
        let ContainerBytes::Segmented { chunks, .. } = &output else {
            panic!("expected segmented storage");
        };
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| chunk.capacity() <= CHUNK_BYTES));
        assert!(output.patch(expected.len(), &[1]).is_err());
        assert!(
            output
                .hash_range(0..expected.len() + 1, &mut hasher)
                .is_err()
        );
        assert_eq!(output.into_vec(), expected);
    }

    #[test]
    fn contiguous_storage_moves_its_original_allocation() {
        let bytes = vec![1, 2, 3];
        let pointer = bytes.as_ptr();
        let output = ContainerBytes::new(false, bytes);
        assert_eq!(output.copy_range(1..3), [2, 3]);
        let bytes = output.into_vec();
        assert_eq!(bytes.as_ptr(), pointer);
    }
}
