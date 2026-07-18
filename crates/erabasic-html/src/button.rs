//! Reference-compatible recognition of ordinary PRINT button strings.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ButtonSegment {
    pub start: usize,
    pub end: usize,
    pub value: Option<i64>,
}

#[derive(Clone, Debug)]
struct Word {
    start: usize,
    end: usize,
    value: Option<i64>,
    whitespace: bool,
}

/// Split a UTF-8 string using the grouping rules of Emuera's `ButtonStringCreator`.
#[must_use]
pub fn split_auto_buttons(source: &str) -> Vec<ButtonSegment> {
    let Some(words) = lex(source) else {
        return plain(source);
    };
    let button_count = words.iter().filter(|word| word.value.is_some()).count();
    if button_count <= 1 {
        return vec![ButtonSegment {
            start: 0,
            end: source.len(),
            value: words.iter().find_map(|word| word.value),
        }];
    }
    let first = words
        .iter()
        .position(|word| word.value.is_some())
        .unwrap_or(0);
    let last = words
        .iter()
        .rposition(|word| word.value.is_some())
        .unwrap_or(0);
    let before = words[..first]
        .iter()
        .any(|word| !word.whitespace && word.start != word.end);
    let after = words[last + 1..]
        .iter()
        .any(|word| !word.whitespace && word.start != word.end);
    let align_right = !before && after;
    let align_left = before && !after;
    let align_flexible = !align_right && !align_left;

    let mut result = Vec::new();
    let mut start = 0;
    let mut end = 0;
    let mut value = None;
    let mut has_core = false;
    let mut has_description = false;
    let reduce = |result: &mut Vec<ButtonSegment>,
                  start: &mut usize,
                  end: &mut usize,
                  value: &mut Option<i64>| {
        if *start != *end {
            result.push(ButtonSegment {
                start: *start,
                end: *end,
                value: *value,
            });
        }
        *start = *end;
        *value = None;
    };

    for word in words {
        if start == end {
            start = word.start;
        }
        if word.whitespace {
            if has_core
                && has_description
                && align_flexible
                && source[word.start..word.end].chars().count() >= 2
            {
                reduce(&mut result, &mut start, &mut end, &mut value);
                start = word.start;
                has_core = false;
                has_description = false;
            }
            end = word.end;
            continue;
        }
        if let Some(input) = word.value {
            if has_core || align_right {
                reduce(&mut result, &mut start, &mut end, &mut value);
                start = word.start;
                end = word.end;
                value = Some(input);
                has_core = true;
                has_description = false;
            } else if align_left {
                end = word.end;
                value = Some(input);
                reduce(&mut result, &mut start, &mut end, &mut value);
                has_core = false;
                has_description = false;
            } else {
                end = word.end;
                value = Some(input);
                has_core = true;
            }
        } else {
            end = word.end;
            has_description = true;
        }
    }
    reduce(&mut result, &mut start, &mut end, &mut value);
    result
}

fn plain(source: &str) -> Vec<ButtonSegment> {
    vec![ButtonSegment {
        start: 0,
        end: source.len(),
        value: None,
    }]
}

fn lex(source: &str) -> Option<Vec<Word>> {
    if source.is_empty() || !source.contains('[') || !source.contains(']') {
        return Some(vec![Word {
            start: 0,
            end: source.len(),
            value: None,
            whitespace: false,
        }]);
    }
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let character = source[cursor..].chars().next()?;
        if character == '[' {
            let end_relative = source[cursor + 1..].find(']')?;
            let end = cursor + 1 + end_relative + 1;
            if source[cursor + 1..end - 1].contains('[') {
                return None;
            }
            result.push(Word {
                start: cursor,
                end,
                value: parse_core(&source[cursor + 1..end - 1]),
                whitespace: false,
            });
            cursor = end;
        } else if character == ']' {
            return None;
        } else if character.is_whitespace() {
            let start = cursor;
            while cursor < source.len() {
                let next = source[cursor..].chars().next()?;
                if !next.is_whitespace() {
                    break;
                }
                cursor += next.len_utf8();
            }
            result.push(Word {
                start,
                end: cursor,
                value: None,
                whitespace: true,
            });
        } else {
            let start = cursor;
            while cursor < source.len() {
                let next = source[cursor..].chars().next()?;
                if matches!(next, '[' | ']') || next.is_whitespace() {
                    break;
                }
                cursor += next.len_utf8();
            }
            result.push(Word {
                start,
                end: cursor,
                value: None,
                whitespace: false,
            });
        }
    }
    Some(result)
}

fn parse_core(source: &str) -> Option<i64> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let (radix, digits) = if let Some(value) = source
        .strip_prefix("0x")
        .or_else(|| source.strip_prefix("0X"))
    {
        (16, value)
    } else if let Some(value) = source
        .strip_prefix("0b")
        .or_else(|| source.strip_prefix("0B"))
    {
        (2, value)
    } else {
        (10, source)
    };
    if radix != 10 {
        let (negative, digits) = digits
            .strip_prefix('-')
            .map_or((false, digits), |v| (true, v));
        let digits = digits.strip_prefix('+').unwrap_or(digits);
        let value = i64::from_str_radix(digits, radix).ok()?;
        return Some(if negative {
            value.wrapping_neg()
        } else {
            value
        });
    }
    if let Some(index) = source.find(['e', 'E', 'p', 'P']) {
        let base = source[..index].parse::<i64>().ok()?;
        let exponent = source[index + 1..].parse::<u32>().ok()?;
        return base.checked_mul(10_i64.checked_pow(exponent)?);
    }
    source.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_reference_style_descriptions() {
        let parts = split_auto_buttons("[1] one  [2] two");
        assert_eq!(
            parts
                .iter()
                .map(|part| (&"[1] one  [2] two"[part.start..part.end], part.value))
                .collect::<Vec<_>>(),
            vec![("[1] one  ", Some(1)), ("[2] two", Some(2))]
        );
        assert_eq!(split_auto_buttons("prefix [42]")[0].value, Some(42));
        assert_eq!(split_auto_buttons("PRINTPLAIN")[0].value, None);
    }
}
