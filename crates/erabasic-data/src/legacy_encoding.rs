//! Script-visible length policy; source strings and FORM layout remain UTF-8.

use crate::LegacyEncoding;

/// Counting policy chosen by the caller's compatibility identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LegacyStringCounting {
    /// Existing scalar-based counting, including its one-byte unmappable fallback.
    #[default]
    Scalar,
    /// Count each UTF-16 unit with the reference code-page roundtrip fallback.
    Utf16Roundtrip,
}

// These tables are generated from the pinned reference's actual .NET provider.
// Each bit represents one UTF-16 unit: zero is one byte, one is two bytes.
// Precomputing the selected-page/CP932 roundtrip decision also preserves .NET
// best-fit and private-use mappings without allocating or encoding per character.
const CP932: &[u8; 8192] = include_bytes!("legacy_encoding_maps/cp932.final-width.bits");
const CP949: &[u8; 8192] = include_bytes!("legacy_encoding_maps/cp949.final-width.bits");
const CP936: &[u8; 8192] = include_bytes!("legacy_encoding_maps/cp936.final-width.bits");
const CP950: &[u8; 8192] = include_bytes!("legacy_encoding_maps/cp950.final-width.bits");

impl LegacyEncoding {
    /// Count a string under the caller's explicit compatibility policy.
    #[must_use]
    pub fn encoded_len_with_policy(self, value: &str, policy: LegacyStringCounting) -> usize {
        if value.is_ascii() {
            return value.len();
        }
        match policy {
            LegacyStringCounting::Scalar => self.encoded_len(value),
            LegacyStringCounting::Utf16Roundtrip => value
                .encode_utf16()
                .map(|unit| self.utf16_unit_len(unit))
                .sum(),
        }
    }

    /// Count one UTF-16 unit, including an isolated surrogate, as the reference does.
    ///
    /// This is the modern roundtrip policy regardless of the scalar default.
    #[must_use]
    pub fn utf16_unit_len(self, unit: u16) -> usize {
        if unit < 0x80 {
            return 1;
        }
        let widths = match self {
            Self::Japanese => CP932,
            Self::Korean => CP949,
            Self::ChineseHans => CP936,
            Self::ChineseHant => CP950,
        };
        let unit = usize::from(unit);
        1 + usize::from((widths[unit >> 3] >> (unit & 7)) & 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCODINGS: [LegacyEncoding; 4] = [
        LegacyEncoding::Japanese,
        LegacyEncoding::Korean,
        LegacyEncoding::ChineseHans,
        LegacyEncoding::ChineseHant,
    ];

    #[test]
    fn pinned_codepages_preserve_best_fit_and_non_whatwg_mappings() {
        for (unit, widths) in [
            (0x00a5, [1, 2, 2, 2]),
            (0x203e, [1, 1, 2, 2]),
            (0x2212, [1, 1, 1, 1]),
            (0xff0d, [2, 2, 2, 2]),
            (0xe758, [1, 1, 2, 2]),
            (0xe800, [1, 1, 2, 2]),
            (0x79d4, [1, 1, 2, 1]),
            (0x43f0, [1, 1, 1, 1]),
            (0x00ca, [1, 1, 2, 1]),
        ] {
            for (encoding, expected) in ENCODINGS.into_iter().zip(widths) {
                assert_eq!(
                    encoding.utf16_unit_len(unit),
                    expected,
                    "{encoding:?} U+{unit:04X}"
                );
            }
        }
    }

    #[test]
    fn scalar_policy_preserves_existing_encoding_behavior() {
        assert_eq!(
            LegacyStringCounting::default(),
            LegacyStringCounting::Scalar
        );
        for encoding in ENCODINGS {
            for input in ["", "ascii\0", "界", "¥−", "\u{e000}", "😀", "\u{79d4}"] {
                assert_eq!(
                    encoding.encoded_len_with_policy(input, LegacyStringCounting::Scalar),
                    encoding.encoded_len(input)
                );
            }
        }
        assert_eq!(LegacyEncoding::Japanese.encoded_char_len('\u{e000}'), 1);
    }

    #[test]
    fn roundtrip_counts_ascii_and_shared_cjk_for_all_code_pages() {
        for encoding in ENCODINGS {
            for (input, expected) in [("", 0), ("ascii\0", 6), ("界", 2), ("A界B", 4)] {
                assert_eq!(
                    encoding.encoded_len_with_policy(input, LegacyStringCounting::Utf16Roundtrip),
                    expected
                );
            }
        }
    }

    #[test]
    fn roundtrip_counts_surrogates_separately_and_preserves_cp932_private_use() {
        for encoding in ENCODINGS {
            for unit in [0xd800, 0xdbff, 0xdc00, 0xdfff] {
                assert_eq!(encoding.utf16_unit_len(unit), 1);
            }
            assert_eq!(
                encoding.encoded_len_with_policy("😀", LegacyStringCounting::Utf16Roundtrip),
                2
            );
            // CP932 private-use units are exact double-byte roundtrips, including
            // when selected by the secondary fallback of another language.
            for unit in [0xe000, 0xe757] {
                assert_eq!(encoding.utf16_unit_len(unit), 2);
            }
        }
    }
}
