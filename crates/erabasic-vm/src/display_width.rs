use erabasic_data::LegacyEncoding;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Portable logical-column policy shared by the VM and frontend projection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CharacterWidthMode {
    /// Era-compatible CJK width with CP932 and pictographic symbol handling.
    #[default]
    Automatic,
    /// Unicode width with East Asian Ambiguous characters kept narrow.
    AmbiguousNarrow,
    /// Unicode CJK width with East Asian Ambiguous characters made wide.
    AmbiguousWide,
}

impl CharacterWidthMode {
    /// Parse the stable configuration spelling used by `CharacterWidthMode`.
    #[must_use]
    pub fn from_config_code(value: &str) -> Self {
        match value.trim().to_ascii_uppercase().as_str() {
            "AMBIGUOUS_NARROW" => Self::AmbiguousNarrow,
            "AMBIGUOUS_WIDE" => Self::AmbiguousWide,
            _ => Self::Automatic,
        }
    }
}

/// Measure `RustyEra`'s deterministic Era console columns.
///
/// The reference `FormatPercent` uses the selected ANSI code page, while its
/// `GETLINESTR` uses `WinForms` font pixels. `RustyEra` needs one portable advance
/// across browser, native-WebView, and terminal clients. Automatic mode follows
/// the CP932 double-byte repertoire used by the reference font and keeps other
/// ambiguous symbols narrow unless they are pictographic. Legacy `STRLENS`,
/// `STRLENSU`, and substring operations intentionally retain their separate
/// code-page and UTF-16 semantics.
#[must_use]
pub fn emuera_display_width(value: &str) -> usize {
    display_width(value, CharacterWidthMode::Automatic)
}

/// Measure text using an explicit project character-width policy.
#[must_use]
pub fn display_width(value: &str, mode: CharacterWidthMode) -> usize {
    value
        .graphemes(true)
        .map(|grapheme| grapheme_width(grapheme, mode))
        .sum()
}

fn grapheme_width(grapheme: &str, mode: CharacterWidthMode) -> usize {
    let unicode_width = match mode {
        CharacterWidthMode::Automatic | CharacterWidthMode::AmbiguousNarrow => {
            UnicodeWidthStr::width(grapheme)
        }
        CharacterWidthMode::AmbiguousWide => UnicodeWidthStr::width_cjk(grapheme),
    };
    if mode != CharacterWidthMode::Automatic {
        return unicode_width;
    }
    let mut characters = grapheme.chars();
    let Some(character) = characters.next() else {
        return 0;
    };
    // MS Gothic gives the CP932 double-byte repertoire a full console cell even where
    // terminal-oriented Unicode width tables keep Greek and Cyrillic letters narrow.
    // Multi-scalar graphemes stay on Unicode rules so combining marks add no phantom cell.
    let single_scalar_cp932 = characters.next().is_none()
        && unicode_width == 1
        && LegacyEncoding::Japanese.encoded_char_len(character) == 2;
    // Era table layouts use box-drawing characters as full console cells even for
    // variants outside CP932. Block elements are deliberately excluded: games use
    // them as half-width progress-bar segments.
    let box_drawing = ('\u{2500}'..='\u{257f}').contains(&character);
    let pictographic = unicode_width == 1
        && !grapheme.contains('\u{fe0e}')
        && grapheme.chars().any(super::extended_pictographic::contains);
    if single_scalar_cp932 || box_drawing || pictographic {
        2
    } else {
        unicode_width
    }
}

/// Repeat a pattern to a deterministic logical-column limit without splitting graphemes.
///
/// # Errors
///
/// Returns an error when the pattern is empty or has no positive logical width.
pub fn logical_line_string(pattern: &str, columns: usize) -> Result<String, &'static str> {
    logical_line_string_with_mode(pattern, columns, CharacterWidthMode::Automatic)
}

/// Repeat a pattern to a logical-column limit using an explicit width policy.
///
/// # Errors
///
/// Returns an error when the pattern is empty or has no positive logical width.
pub fn logical_line_string_with_mode(
    pattern: &str,
    columns: usize,
    mode: CharacterWidthMode,
) -> Result<String, &'static str> {
    if pattern.is_empty() {
        return Err("GETLINESTR pattern must not be empty");
    }
    let graphemes: Vec<_> = pattern.graphemes(true).collect();
    let widths: Vec<_> = graphemes
        .iter()
        .map(|grapheme| display_width(grapheme, mode))
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
            ("γ", 2),
            ("о", 2),
            ("´", 2),
            ("║", 2),
            ("▅", 1),
            ("ﾄ", 1),
            ("｡", 1),
            ("e\u{301}", 1),
            ("\u{200b}", 0),
            ("😀", 2),
            ("👨‍👩‍👧‍👦", 2),
            ("☀", 2),
            ("❤", 2),
            ("❤❤❤❤", 8),
        ] {
            assert_eq!(emuera_display_width(text), width, "{text:?}");
        }
    }

    #[test]
    fn width_modes_distinguish_ambiguous_and_unqualified_pictographs() {
        assert_eq!(display_width("…γ■", CharacterWidthMode::AmbiguousNarrow), 3);
        assert_eq!(display_width("…γ■", CharacterWidthMode::AmbiguousWide), 5);
        assert_eq!(display_width("…γ■", CharacterWidthMode::Automatic), 6);
        assert_eq!(display_width("▅", CharacterWidthMode::AmbiguousNarrow), 1);
        assert_eq!(display_width("▅", CharacterWidthMode::AmbiguousWide), 2);
        assert_eq!(display_width("▅", CharacterWidthMode::Automatic), 1);
        assert_eq!(display_width("☀❤", CharacterWidthMode::AmbiguousNarrow), 2);
        assert_eq!(display_width("☀❤", CharacterWidthMode::AmbiguousWide), 2);
        assert_eq!(display_width("☀❤", CharacterWidthMode::Automatic), 4);
        assert_eq!(
            display_width("☀\u{fe0e}❤\u{fe0e}", CharacterWidthMode::Automatic),
            2
        );
        assert_eq!(
            display_width("☀\u{fe0f}❤\u{fe0f}", CharacterWidthMode::Automatic),
            4
        );
        assert_eq!(
            logical_line_string_with_mode("…", 6, CharacterWidthMode::AmbiguousNarrow),
            Ok("………………".into())
        );
        assert_eq!(
            logical_line_string_with_mode("…", 6, CharacterWidthMode::AmbiguousWide),
            Ok("………".into())
        );
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
