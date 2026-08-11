pub(in super::super) fn format_era_integer(
    value: i64,
    format: &str,
) -> Result<String, &'static str> {
    if format.is_empty() {
        return Ok(value.to_string());
    }
    let mut chars = format.chars();
    let first = chars.next().expect("non-empty format");
    let precision = chars.as_str().parse::<usize>().ok();
    match first.to_ascii_uppercase() {
        'D' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            let magnitude = value.unsigned_abs().to_string();
            let digits = format!("{magnitude:0>width$}");
            Ok(if value < 0 {
                format!("-{digits}")
            } else {
                digits
            })
        }
        'X' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            if first.is_ascii_lowercase() {
                Ok(format!("{value:0>width$x}"))
            } else {
                Ok(format!("{value:0>width$X}"))
            }
        }
        'N' if chars.as_str().is_empty() || precision.is_some() => {
            let decimals = precision.unwrap_or(2);
            let grouped = group_decimal(value);
            Ok(if decimals == 0 {
                grouped
            } else {
                format!("{grouped}.{}", "0".repeat(decimals))
            })
        }
        _ => {
            let Some((literal_prefix, numeric_format, literal_suffix)) =
                custom_decimal_format(format)
            else {
                return Err("unsupported integer format");
            };
            let minimum = numeric_format
                .chars()
                .filter(|character| *character == '0')
                .count();
            let mut digits = if value == 0 && minimum == 0 {
                String::new()
            } else {
                let magnitude = value.unsigned_abs().to_string();
                format!("{magnitude:0>minimum$}")
            };
            if numeric_format.contains(',') {
                digits = group_unsigned_decimal(&digits);
            }
            let formatted = format!("{literal_prefix}{digits}{literal_suffix}");
            Ok(if value < 0 {
                format!("-{formatted}")
            } else {
                formatted
            })
        }
    }
}

fn custom_decimal_format(format: &str) -> Option<(&str, &str, &str)> {
    let numeric_start = format
        .char_indices()
        .find_map(|(index, character)| matches!(character, '#' | '0').then_some(index))?;
    let numeric_end = format[numeric_start..]
        .char_indices()
        .find_map(|(index, character)| {
            (!matches!(character, '#' | '0' | ',')).then_some(numeric_start + index)
        })
        .unwrap_or(format.len());
    let literal_prefix = &format[..numeric_start];
    let numeric_format = &format[numeric_start..numeric_end];
    let literal_suffix = &format[numeric_end..];
    let invalid_literal = |character| {
        matches!(
            character,
            '#' | '0' | ',' | '.' | '%' | '‰' | 'E' | 'e' | '\\' | '\'' | '"' | ';'
        )
    };
    if !numeric_format
        .chars()
        .any(|character| matches!(character, '#' | '0'))
        || literal_prefix.chars().any(invalid_literal)
        || literal_suffix.chars().any(invalid_literal)
    {
        return None;
    }
    Some((literal_prefix, numeric_format, literal_suffix))
}

pub(in super::super) fn group_decimal(value: i64) -> String {
    let digits = group_unsigned_decimal(&value.unsigned_abs().to_string());
    if value < 0 {
        format!("-{digits}")
    } else {
        digits
    }
}

pub(in super::super) fn group_unsigned_decimal(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + value.len() / 3);
    for (index, character) in value.chars().enumerate() {
        if index != 0 && (value.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

// Version 1 of the deterministic width table covers the ASCII block and the
// half-width katakana block used by Emuera projects.  It deliberately avoids
// the platform-dependent VisualBasic StrConv implementation.
const HALF_KANA: &str = "｡｢｣､･ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
const FULL_KANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

pub(in super::super) fn to_full_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut input = value.chars().peekable();
    while let Some(character) = input.next() {
        if let Some(mark) = input.peek().copied()
            && matches!(mark, 'ﾞ' | 'ﾟ')
            && let Some(composed) = compose_half_kana(character, mark)
        {
            output.push(composed);
            input.next();
            continue;
        }
        match character {
            ' ' => output.push('　'),
            '!'..='~' => output.push(char::from_u32(u32::from(character) + 0xfee0).unwrap()),
            _ => output.push(map_width_char(character, HALF_KANA, FULL_KANA).unwrap_or(character)),
        }
    }
    output
}

pub(in super::super) fn to_half_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if let Some(pair) = decompose_full_kana(character) {
            output.extend(pair);
            continue;
        }
        match character {
            '　' => output.push(' '),
            '\u{ff01}'..='\u{ff5e}' => {
                output.push(char::from_u32(u32::from(character) - 0xfee0).unwrap());
            }
            _ => output.push(map_width_char(character, FULL_KANA, HALF_KANA).unwrap_or(character)),
        }
    }
    output
}

/// Apply the pinned Japanese LCID 0x0411 subset used by FORCEKANA. The table is
/// embedded so execution never depends on the host locale or platform APIs.
pub(in super::super) fn convert_kana_mode(value: &str, mode: u8) -> String {
    let value = if mode == 3 {
        to_full_width(value)
    } else {
        value.to_owned()
    };
    value
        .chars()
        .map(|character| match mode {
            1 => hiragana_to_katakana(character),
            2 | 3 => katakana_to_hiragana(character),
            _ => character,
        })
        .collect()
}

pub(in super::super) fn hiragana_to_katakana(character: char) -> char {
    match character {
        '\u{3041}'..='\u{3096}' => char::from_u32(u32::from(character) + 0x60).unwrap_or(character),
        'ゝ' => 'ヽ',
        'ゞ' => 'ヾ',
        _ => character,
    }
}

pub(in super::super) fn katakana_to_hiragana(character: char) -> char {
    match character {
        '\u{30a1}'..='\u{30f6}' => char::from_u32(u32::from(character) - 0x60).unwrap_or(character),
        'ヽ' => 'ゝ',
        'ヾ' => 'ゞ',
        _ => character,
    }
}

pub(in super::super) fn map_width_char(
    character: char,
    source: &str,
    target: &str,
) -> Option<char> {
    source
        .chars()
        .position(|candidate| candidate == character)
        .and_then(|index| target.chars().nth(index))
}

pub(in super::super) fn compose_half_kana(base: char, mark: char) -> Option<char> {
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    bases
        .chars()
        .zip(marks.chars())
        .position(|(candidate, candidate_mark)| candidate == base && candidate_mark == mark)
        .and_then(|index| full.chars().nth(index))
}

pub(in super::super) fn decompose_full_kana(character: char) -> Option<[char; 2]> {
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    full.chars()
        .position(|candidate| candidate == character)
        .and_then(|index| Some([bases.chars().nth(index)?, marks.chars().nth(index)?]))
}
