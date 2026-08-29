//! ICU72 root, normalization OFF, numeric OFF, .NET CompareOptions.None.
//! Requires the root-owned validated full-closure dataset; never accepts the
//! canonically pruned ICU4X testdata collation trie as a replacement.
#![forbid(unsafe_code)]

pub(crate) mod ce;
mod fcd_data;
pub(crate) mod fixed_data;
pub(crate) mod raw_off;
pub(crate) mod simple_affix;

#[cfg(test)]
use ce::{CeError, CeLimits, CeStream};
#[cfg(test)]
use raw_off::{RawRootData, raw_off_elements};
#[cfg(test)]
use simple_affix::{Direction, simple_affix};

/// The provider must be constructed by trusted fixed-data validation/binding.
/// This internal wrapper has no locale/options/provider selection from script.
#[cfg(test)]
pub(crate) struct FixedIcu72Root<D: RawRootData> {
    data: D,
}

#[cfg(test)]
impl<D: RawRootData> FixedIcu72Root<D> {
    pub(crate) fn from_validated_data(data: D) -> Self {
        Self { data }
    }

    pub(crate) fn elements_utf16(
        &self,
        input: &[u16],
        limits: CeLimits,
    ) -> Result<CeStream, CeError> {
        raw_off_elements(&self.data, input, limits)
    }

    pub(crate) fn starts_with_utf16(
        &self,
        source: &[u16],
        pattern: &[u16],
        limits: CeLimits,
    ) -> Result<bool, CeError> {
        self.affix(source, pattern, limits, Direction::Forward)
    }

    pub(crate) fn ends_with_utf16(
        &self,
        source: &[u16],
        pattern: &[u16],
        limits: CeLimits,
    ) -> Result<bool, CeError> {
        self.affix(source, pattern, limits, Direction::Backward)
    }

    fn affix(
        &self,
        source: &[u16],
        pattern: &[u16],
        limits: CeLimits,
        direction: Direction,
    ) -> Result<bool, CeError> {
        // Managed CompareInfo.IsPrefix/IsSuffix resolves an actually empty
        // pattern before native SimpleAffix; an ignorable nonempty pattern
        // must still run the native CE control flow.
        if pattern.is_empty() {
            return Ok(true);
        }
        let source_len = source.len();
        let pattern_len = pattern.len();
        let source = self.elements_utf16(source, limits)?;
        let pattern = self.elements_utf16(pattern, limits)?;
        let source_legacy = source.legacy_elements()?;
        let pattern_legacy = pattern.legacy_elements()?;
        Ok(simple_affix(
            &source_legacy,
            source_len,
            &pattern_legacy,
            pattern_len,
            direction,
        )
        .matched)
    }
}
