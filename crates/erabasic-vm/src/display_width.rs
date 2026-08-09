use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Measure `RustyEra`'s deterministic CJK console columns.
///
/// The reference `FormatPercent` uses the selected ANSI code page, while its
/// `GETLINESTR` uses `WinForms` font pixels. `RustyEra` needs one portable advance
/// across browser, native-WebView, and terminal clients, so it treats East Asian
/// Ambiguous characters as wide. Legacy `STRLENS`, `STRLENSU`, and substring
/// operations intentionally retain their separate code-page and UTF-16 semantics.
#[must_use]
pub fn emuera_display_width(value: &str) -> usize {
    UnicodeWidthStr::width_cjk(value)
}

/// Repeat a pattern to a deterministic logical-column limit without splitting graphemes.
///
/// # Errors
///
/// Returns an error when the pattern is empty or has no positive logical width.
pub fn logical_line_string(pattern: &str, columns: usize) -> Result<String, &'static str> {
    if pattern.is_empty() {
        return Err("GETLINESTR pattern must not be empty");
    }
    let graphemes: Vec<_> = pattern.graphemes(true).collect();
    let widths: Vec<_> = graphemes
        .iter()
        .map(|grapheme| emuera_display_width(grapheme))
        .collect();
    if widths.iter().all(|width| *width == 0) {
        return Err("GETLINESTR pattern must have positive logical width");
    }
    let mut result = String::new();
    let mut used: usize = 0;
    let mut committed_length = 0;
    loop {
        let before = used;
        for (grapheme, width) in graphemes.iter().zip(&widths) {
            if used.saturating_add(*width) > columns {
                result.truncate(committed_length);
                return Ok(result);
            }
            result.push_str(grapheme);
            used = used.saturating_add(*width);
            if *width != 0 {
                committed_length = result.len();
                if used == columns {
                    return Ok(result);
                }
            }
        }
        if used == before {
            return Ok(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_data::LegacyEncoding;

    #[test]
    fn display_width_covers_cjk_ambiguous_combining_and_emoji_sequences() {
        for (text, width) in [
            ("", 0),
            ("abc", 3),
            ("界", 2),
            ("■", 2),
            ("…", 2),
            ("e\u{301}", 1),
            ("\u{200b}", 0),
            ("😀", 2),
            ("👨‍👩‍👧‍👦", 2),
        ] {
            assert_eq!(emuera_display_width(text), width, "{text:?}");
        }
    }

    #[test]
    fn logical_lines_handle_mixed_partial_and_zero_width_patterns() {
        assert_eq!(logical_line_string("■", 8), Ok("■■■■".into()));
        assert_eq!(logical_line_string("A■", 7), Ok("A■A■A".into()));
        assert_eq!(
            logical_line_string("\u{200b}■", 5),
            Ok("\u{200b}■\u{200b}■".into())
        );
        assert_eq!(logical_line_string("■", 1), Ok(String::new()));
        assert!(logical_line_string("\u{200b}", 8).is_err());
    }

    #[test]
    fn portable_columns_do_not_depend_on_the_selected_legacy_encoding() {
        let text = "■……■";
        assert_eq!(emuera_display_width(text), 8);
        for encoding in [
            LegacyEncoding::Japanese,
            LegacyEncoding::Korean,
            LegacyEncoding::ChineseHans,
            LegacyEncoding::ChineseHant,
        ] {
            assert_eq!(encoding.encoded_len(text), 8, "{encoding:?}");
        }

        // Legacy code pages intentionally retain their own replacement behavior.
        let not_portably_encodable = "😀";
        assert_eq!(emuera_display_width(not_portably_encodable), 2);
        assert!(
            [
                LegacyEncoding::Japanese,
                LegacyEncoding::Korean,
                LegacyEncoding::ChineseHans,
                LegacyEncoding::ChineseHant,
            ]
            .into_iter()
            .all(|encoding| encoding.encoded_len(not_portably_encodable) == 1)
        );
    }
}
