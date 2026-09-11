//! Original upstream legacy indices count UTF-16 units, including lone surrogates.

use erabasic_data::{LegacyEncoding, LegacyStringCounting};

// The reference first casts each script integer to a signed 32-bit index.
#[allow(clippy::cast_possible_truncation)]
fn reference_index(value: i64) -> i32 {
    value as i32
}

/// Round a byte position up to a UTF-16 boundary, returning units and byte width.
fn start_boundary(value: &str, index: i32, encoding: LegacyEncoding) -> (usize, usize) {
    let Ok(index) = usize::try_from(index) else {
        return (0, 0);
    };
    if index == 0 {
        return (0, 0);
    }
    let mut width = 0;
    let mut units = 0;
    for unit in value.encode_utf16() {
        width += encoding.utf16_unit_len(unit);
        units += 1;
        if width >= index {
            break;
        }
    }
    (units, width)
}

pub(super) fn substring(
    value: &str,
    start: i64,
    length: Option<i64>,
    encoding: LegacyEncoding,
) -> String {
    let start = reference_index(start);
    let length = length.map_or(-1, reference_index);
    let total = encoding.encoded_len_with_policy(value, LegacyStringCounting::Utf16Roundtrip);
    if value.is_empty() || length == 0 || usize::try_from(start).is_ok_and(|start| start >= total) {
        return String::new();
    }
    let length = usize::try_from(length).unwrap_or(total).min(total);
    if start <= 0 && length == total {
        return value.to_owned();
    }
    let (offset, _) = start_boundary(value, start, encoding);
    let mut width = 0;
    let mut result = Vec::new();
    for unit in value.encode_utf16().skip(offset) {
        result.push(unit);
        width += encoding.utf16_unit_len(unit);
        if width >= length {
            break;
        }
    }
    // Public strings remain valid UTF-8. A cut surrogate is deliberately U+FFFD.
    String::from_utf16_lossy(&result)
}

pub(super) fn find(value: &str, needle: &str, start: i64, encoding: LegacyEncoding) -> i64 {
    let (offset, width) = start_boundary(value, reference_index(start), encoding);
    let mut units = 0;
    let mut byte_start = value.len();
    for (byte, character) in value.char_indices() {
        if units >= offset {
            byte_start = byte;
            break;
        }
        units += character.len_utf16();
    }
    if needle.is_empty() {
        // Empty needles can match between surrogate units. The end is rejected.
        return if offset < value.encode_utf16().count() {
            i64::try_from(width).unwrap_or(i64::MAX)
        } else {
            -1
        };
    }
    // A valid UTF-8 needle cannot begin on a low surrogate. Advancing to the next
    // scalar boundary is therefore equivalent to ordinal search in the original
    // UTF-16 units, without allocating or introducing replacement characters.
    value[byte_start..]
        .find(needle)
        .and_then(|relative| {
            i64::try_from(encoding.encoded_len_with_policy(
                &value[..byte_start + relative],
                LegacyStringCounting::Utf16Roundtrip,
            ))
            .ok()
        })
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_cuts_preserve_reference_boundaries_and_replace_lone_surrogates() {
        let encoding = LegacyEncoding::Japanese;
        for (start, length, expected) in [
            (0, 1, "\u{fffd}"),
            (1, 1, "\u{fffd}"),
            (0, 2, "😀"),
            (1, 2, "\u{fffd}X"),
            (2, 1, "X"),
            (-1, 1, "\u{fffd}"),
            (0, 0, ""),
            (4, 1, ""),
            (0, -1, "😀X"),
            (4_294_967_296, 2, "😀"),
            (0, 4_294_967_296, ""),
        ] {
            assert_eq!(substring("😀X", start, Some(length), encoding), expected);
        }
        assert_eq!(substring("漢X", 1, Some(1), encoding), "X");
        assert_eq!(substring("漢X", 0, Some(1), encoding), "漢");
    }

    #[test]
    fn ordinal_find_keeps_partial_surrogates_distinct_from_replacement_characters() {
        let encoding = LegacyEncoding::Japanese;
        for (value, needle, start, expected) in [
            ("😀X", "", 1, 1),
            ("😀X", "X", 1, 2),
            ("😀X", "😀", 1, -1),
            ("😀X", "\u{fffd}", 1, -1),
            ("abc", "a", -1, 0),
            ("abc", "a", 2_147_483_648, 0),
            ("abc", "a", 4_294_967_296, 0),
            ("abc", "a", 4_294_967_297, -1),
            ("", "", 0, -1),
            ("abc", "", 3, -1),
            ("漢X", "X", 1, 2),
        ] {
            assert_eq!(find(value, needle, start, encoding), expected);
        }
    }
}
