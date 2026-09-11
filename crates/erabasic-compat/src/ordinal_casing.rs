//! Fixed .NET 8 / ICU72 ordinal casing shared by loaders and the VM.
//! Source: dotnet/runtime System.Globalization.OrdinalCasing.Icu.cs.

#[path = "ordinal_casing_data.rs"]
mod data;

/// A complete, sorted sparse BMP simple-uppercase table; omitted entries are identity.
/// This is immutable product data, never an input supplied by scripts or frontends.
pub struct OrdinalCasing {
    bmp_simple_upper_low: &'static [(u16, u16)],
    bmp_simple_upper_high: &'static [(u16, u16)],
}

impl OrdinalCasing {
    /// Fixed .NET 8 ICU-mode casing, bound to Unicode 15 / ICU72 input.
    #[must_use]
    pub const fn fixed_dotnet8_icu72() -> Self {
        Self {
            bmp_simple_upper_low: data::ICU72_BMP_SIMPLE_UPPER_LOW,
            bmp_simple_upper_high: data::ICU72_BMP_SIMPLE_UPPER_HIGH,
        }
    }

    #[must_use]
    pub fn equals(&self, left: &str, right: &str, ignore_case: bool) -> bool {
        if !ignore_case || left == right {
            return left == right;
        }
        // .NET StringComparer rejects different UTF-16 lengths before casing.
        if left.encode_utf16().count() != right.encode_utf16().count() {
            return false;
        }
        let mut left = left.chars();
        let mut right = right.chars();
        loop {
            match (left.next(), right.next()) {
                (None, None) => return true,
                (Some(a), Some(b)) if a == b => {}
                (Some(a), Some(b))
                    if a.len_utf16() == b.len_utf16() && self.upper(a) == self.upper(b) => {}
                _ => return false,
            }
        }
    }

    /// Compare valid UTF-8 strings using the fixed .NET 8 ICU ordinal casing rules.
    /// The upstream comparator orders any valid surrogate pair after every BMP
    /// character, then compares uppercased supplementary scalars. This differs
    /// from lexicographically sorting the resulting UTF-16 code units.
    #[must_use]
    pub fn compare(&self, left: &str, right: &str) -> std::cmp::Ordering {
        left.chars()
            .map(|value| self.upper(value))
            .cmp(right.chars().map(|value| self.upper(value)))
    }

    fn upper(&self, value: char) -> u32 {
        let scalar = u32::from(value);
        if scalar > 0xffff {
            // In ICU mode, .NET uses its own CharUnicodeInfo table for pairs.
            return data::DOTNET_SUPPLEMENTARY
                .binary_search_by_key(&scalar, |pair| pair.0)
                .map_or(scalar, |index| data::DOTNET_SUPPLEMENTARY[index].1);
        }
        let Ok(unit) = u16::try_from(scalar) else {
            return scalar;
        };
        if unit < 256 {
            return u32::from(data::LATIN_UPPER[usize::from(unit)]);
        }
        let page = usize::from(unit >> 8);
        // These pages are identity even if a newer ICU adds a casing mapping.
        if data::NO_CASING_PAGES[page / 8] & (0x80 >> (page % 8)) != 0
            || matches!(unit, 0x0131 | 0x017f)
        {
            return scalar;
        }
        let table = if unit < 0x214e {
            self.bmp_simple_upper_low
        } else {
            self.bmp_simple_upper_high
        };
        table
            .binary_search_by_key(&unit, |pair| pair.0)
            .map_or(scalar, |index| u32::from(table[index].1))
    }
}

#[cfg(test)]
mod tests {
    use super::OrdinalCasing;
    use std::cmp::Ordering::{Equal, Greater, Less};

    #[test]
    fn ordinal_compare_reuses_fixed_simple_casing_without_expansion() {
        let casing = OrdinalCasing::fixed_dotnet8_icu72();
        for (left, right) in [
            ("ä", "Ä"),
            ("σ", "ς"),
            ("µ", "Μ"),
            ("\u{10428}", "\u{10400}"),
        ] {
            assert_eq!(casing.compare(left, right), Equal);
            assert!(casing.equals(left, right, true));
            assert!(!casing.equals(left, right, false));
        }
        for (left, right) in [("ß", "SS"), ("ı", "I"), ("ſ", "S")] {
            assert_ne!(casing.compare(left, right), Equal);
            assert!(!casing.equals(left, right, true));
        }
    }

    #[test]
    fn ordinal_compare_places_surrogate_pairs_after_bmp_and_handles_prefixes() {
        let casing = OrdinalCasing::fixed_dotnet8_icu72();
        assert_eq!(casing.compare("\u{ffff}", "\u{10000}"), Less);
        assert_eq!(casing.compare("\u{10000}", "\u{e000}"), Greater);
        assert_eq!(casing.compare("\u{10428}", "\u{10401}"), Less);
        assert_eq!(casing.compare("Ä", "ä/x"), Less);
        assert_eq!(casing.compare("", ""), Equal);
        assert_eq!(casing.compare("x", ""), Greater);
    }
}
