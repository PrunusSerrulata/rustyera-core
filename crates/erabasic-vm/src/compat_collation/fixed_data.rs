//! Safe, allocation-free views of the embedded ICU72 full root data.
//! Serialized `UTrie2` and `UCol` layouts follow ICU release-72-1, not the host ICU.
use std::sync::OnceLock;
use zerovec::ZeroSlice;

use super::{ce::CeError, fcd_data, raw_off::RawRootData};

const ROOT_BYTES: &[u8] = include_bytes!("data/icu72-root.icu");
static ROOT: OnceLock<Result<FixedRootData<'static>, CeError>> = OnceLock::new();

pub(crate) fn fixed_root_data() -> Result<FixedRootData<'static>, CeError> {
    *ROOT.get_or_init(|| FixedRootData::parse(ROOT_BYTES))
}

fn word16(bytes: &[u8], offset: usize) -> Result<u16, CeError> {
    let pair = bytes
        .get(offset..offset.checked_add(2).ok_or(CeError::MalformedProvider)?)
        .ok_or(CeError::MalformedProvider)?;
    Ok(u16::from_le_bytes([pair[0], pair[1]]))
}

fn word32(bytes: &[u8], offset: usize) -> Result<u32, CeError> {
    let word = bytes
        .get(offset..offset.checked_add(4).ok_or(CeError::MalformedProvider)?)
        .ok_or(CeError::MalformedProvider)?;
    Ok(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

#[derive(Clone, Copy)]
struct Trie2<'a> {
    index: &'a ZeroSlice<u16>,
    values: &'a ZeroSlice<u32>,
    high_start: u32,
}

impl<'a> Trie2<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, CeError> {
        // Only the fixed little-endian 32-bit Trie2 representation is accepted.
        if word32(bytes, 0)? != 0x5472_6932 || word16(bytes, 4)? != 1 {
            return Err(CeError::MalformedProvider);
        }
        let index_length = usize::from(word16(bytes, 6)?);
        let data_length = usize::from(word16(bytes, 8)?) << 2;
        let high_start = u32::from(word16(bytes, 14)?) << 11;
        if index_length < 0x840
            || data_length < 0xc4
            || !(0x1_0000..=0x11_0000).contains(&high_start)
        {
            return Err(CeError::MalformedProvider);
        }
        let index_end = 16 + index_length * 2;
        let data_end = index_end + data_length * 4;
        let index =
            ZeroSlice::parse_bytes(bytes.get(16..index_end).ok_or(CeError::MalformedProvider)?)
                .map_err(|_| CeError::MalformedProvider)?;
        let values = ZeroSlice::parse_bytes(
            bytes
                .get(index_end..data_end)
                .ok_or(CeError::MalformedProvider)?,
        )
        .map_err(|_| CeError::MalformedProvider)?;
        Ok(Self {
            index,
            values,
            high_start,
        })
    }

    fn index(&self, position: usize) -> Result<usize, CeError> {
        self.index
            .get(position)
            .map(usize::from)
            .ok_or(CeError::MalformedProvider)
    }

    fn get(&self, cp: u32) -> Result<u32, CeError> {
        if cp > 0x10_ffff {
            return Err(CeError::MalformedProvider);
        }
        let value_index = if cp <= 0xffff {
            // Lead surrogate *code points* have their own BMP index block.
            // This is not the single UTF-16 lead-unit fast path.
            let offset = if (0xd800..=0xdbff).contains(&cp) {
                0x800 - (0xd800 >> 5)
            } else {
                0
            };
            (self.index(offset + (cp as usize >> 5))? << 2) + (cp as usize & 31)
        } else if cp >= self.high_start {
            self.values.len() - 4
        } else {
            let first = self.index((0x840 - 32) + (cp as usize >> 11))?;
            let second = self.index(first + ((cp as usize >> 5) & 63))?;
            (second << 2) + (cp as usize & 31)
        };
        self.values
            .get(value_index)
            .ok_or(CeError::MalformedProvider)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FixedRootData<'a> {
    trie: Trie2<'a>,
    ce32s: &'a ZeroSlice<u32>,
    ces: &'a ZeroSlice<u64>,
    contexts: &'a ZeroSlice<u16>,
    jamo_start: usize,
}

impl<'a> FixedRootData<'a> {
    /// Internal parser; scripts and frontends cannot select or replace data.
    fn parse(bytes: &'a [u8]) -> Result<Self, CeError> {
        if bytes.get(2..4) != Some(&[0xda, 0x27])
            || bytes.get(8..11) != Some(&[0, 0, 2])
            || bytes.get(12..20) != Some(b"UCol\x05\0\0\0")
            || bytes.get(20..24) != Some(&[9, 120, 0, 0])
        {
            return Err(CeError::MalformedProvider);
        }
        let header = usize::from(word16(bytes, 0)?);
        if header < 24 {
            return Err(CeError::MalformedProvider);
        }
        let payload = bytes.get(header..).ok_or(CeError::MalformedProvider)?;
        if word32(payload, 0)? != 20 || word32(payload, 4)? != 0x0f02_2010 {
            return Err(CeError::MalformedProvider);
        }
        let mut indexes = [0usize; 20];
        for (position, index) in indexes.iter_mut().enumerate() {
            *index = usize::try_from(word32(payload, position * 4)?)
                .map_err(|_| CeError::MalformedProvider)?;
        }
        if indexes[5] < 80
            || indexes[19] > payload.len()
            || indexes[5..].windows(2).any(|pair| pair[0] > pair[1])
            || indexes[5] != indexes[7]
        {
            return Err(CeError::MalformedProvider);
        }
        let part = |index: usize| &payload[indexes[index]..indexes[index + 1]];
        let trie = Trie2::parse(part(7))?;
        let ce32s: &ZeroSlice<u32> =
            ZeroSlice::parse_bytes(part(11)).map_err(|_| CeError::MalformedProvider)?;
        let ces = ZeroSlice::parse_bytes(part(9)).map_err(|_| CeError::MalformedProvider)?;
        let contexts = ZeroSlice::parse_bytes(part(13)).map_err(|_| CeError::MalformedProvider)?;
        let jamo_start = indexes[4];
        if jamo_start
            .checked_add(67)
            .is_none_or(|last| last >= ce32s.len())
        {
            return Err(CeError::MalformedProvider);
        }
        Ok(Self {
            trie,
            ce32s,
            ces,
            contexts,
            jamo_start,
        })
    }
}

impl RawRootData for FixedRootData<'_> {
    fn ce32(&self, cp: u32) -> Result<u32, CeError> {
        self.trie.get(cp)
    }
    fn ce32_at(&self, index: usize) -> Result<u32, CeError> {
        self.ce32s.get(index).ok_or(CeError::MalformedProvider)
    }
    fn ce_at(&self, index: usize) -> Result<u64, CeError> {
        self.ces.get(index).ok_or(CeError::MalformedProvider)
    }
    fn contexts(&self) -> &ZeroSlice<u16> {
        self.contexts
    }
    fn jamo_ce32_at(&self, index: usize) -> Result<u32, CeError> {
        if index >= 68 {
            return Err(CeError::MalformedProvider);
        }
        self.ce32_at(self.jamo_start + index)
    }
    fn fcd16(&self, cp: u32) -> Result<u16, CeError> {
        if cp > 0x10_ffff {
            return Err(CeError::MalformedProvider);
        }
        let position = fcd_data::FCD16.partition_point(|(_, end, _)| *end < cp);
        Ok(fcd_data::FCD16
            .get(position)
            .filter(|(start, _, _)| *start <= cp)
            .map_or(0, |(_, _, value)| *value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_provider_rejects_truncated_or_wrong_format_data() {
        for length in [0, 23, 31, 32, 111, ROOT_BYTES.len() / 2] {
            assert!(matches!(
                FixedRootData::parse(&ROOT_BYTES[..length]),
                Err(CeError::MalformedProvider)
            ));
        }
        let mut wrong = ROOT_BYTES.to_vec();
        wrong[16] = 4;
        assert!(matches!(
            FixedRootData::parse(&wrong),
            Err(CeError::MalformedProvider)
        ));
        wrong[16] = 5;
        wrong[32 + 19 * 4..32 + 20 * 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            FixedRootData::parse(&wrong),
            Err(CeError::MalformedProvider)
        ));
    }

    #[test]
    fn root_provider_preserves_supplementary_and_fcd_boundaries() {
        let data = fixed_root_data().unwrap();
        assert_eq!(data.fcd16(0x41), Ok(0));
        assert_eq!(data.fcd16(0xe9), Ok(230));
        assert_eq!(data.fcd16(0x301), Ok(0xe6e6));
        assert_eq!(data.fcd16(0x1d165), Ok(0xd8d8));
        assert_eq!(data.fcd16(0xac00), Ok(0));
        assert_eq!(data.fcd16(0xd800), Ok(0));
        assert_eq!(data.ce32(0x11_0000), Err(CeError::MalformedProvider));
        assert_eq!(data.ce32(0x10_ffff), data.ce32(0x10_fffe));
        assert!(data.ce32(0xd800).is_ok());
        assert!(data.ce32(0x10000).is_ok());
        assert!(data.jamo_ce32_at(67).is_ok());
        assert_eq!(data.jamo_ce32_at(68), Err(CeError::MalformedProvider));
    }
}
