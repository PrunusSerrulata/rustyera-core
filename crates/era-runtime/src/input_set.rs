const MAX_EXPANDED_BYTES: usize = 1024 * 1024;
const MAX_NESTING: usize = 256;
const MAX_SEGMENTS: usize = 65_536;
const EXPANSION_LIMIT_ERROR: &str = "input macro expansion exceeds its limit";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InputSegment {
    pub(crate) text: String,
    pub(crate) message_skip: bool,
}

/// Expand Emuera's keyboard-input mini language before wait-specific validation.
pub(crate) fn preprocess_input(text: &str) -> Result<Vec<InputSegment>, &'static str> {
    ensure_size(text.len())?;
    let expanded = if text.contains('(') {
        let characters = text.chars().collect::<Vec<_>>();
        let mut position = 0;
        parse_repetitions(&characters, &mut position, false, 0)?
    } else {
        text.to_owned()
    };
    split_input(&expanded)
}

fn parse_repetitions(
    characters: &[char],
    position: &mut usize,
    nested: bool,
    depth: usize,
) -> Result<String, &'static str> {
    if depth > MAX_NESTING {
        return Err(EXPANSION_LIMIT_ERROR);
    }
    let mut output = String::new();
    let mut has_return = false;
    while *position < characters.len() && (!nested || characters[*position] != ')') {
        match characters[*position] {
            '(' => {
                *position += 1;
                let group = parse_repetitions(characters, position, true, depth + 1)?;
                if *position >= characters.len() {
                    append_checked(&mut output, &group)?;
                    break;
                }
                *position += 1;
                if characters.get(*position) == Some(&'*') {
                    *position += 1;
                    let start = *position;
                    while characters
                        .get(*position)
                        .is_some_and(|value| value.is_numeric())
                    {
                        *position += 1;
                    }
                    let count = characters[start..*position]
                        .iter()
                        .collect::<String>()
                        .parse::<i32>()
                        .unwrap_or(0);
                    if count > 0 {
                        append_repeated(
                            &mut output,
                            &group,
                            usize::try_from(count).expect("positive i32 fits usize"),
                        )?;
                    }
                } else {
                    append_checked(&mut output, &group)?;
                }
            }
            '\\' => {
                *position += 1;
                let escaped = characters.get(*position).copied().unwrap_or('\0');
                match escaped {
                    'n' if has_return => has_return = false,
                    'n' => append_character(&mut output, '\n')?,
                    'r' => append_character(&mut output, '\r')?,
                    'e' => {
                        append_checked(&mut output, "\\e\n")?;
                        has_return = true;
                    }
                    '\n' => {}
                    other => append_character(&mut output, other)?,
                }
                *position += 1;
            }
            character => {
                append_character(&mut output, character)?;
                *position += 1;
            }
        }
    }
    Ok(output)
}

fn split_input(expanded: &str) -> Result<Vec<InputSegment>, &'static str> {
    let mut pieces = vec![String::new()];
    let mut remaining = expanded;
    while !remaining.is_empty() {
        let separator_length = if remaining.starts_with("\\n") || remaining.starts_with("\r\n") {
            2
        } else {
            usize::from(remaining.starts_with('\n') || remaining.starts_with('\r'))
        };
        if separator_length != 0 {
            if pieces.len() >= MAX_SEGMENTS {
                return Err(EXPANSION_LIMIT_ERROR);
            }
            pieces.push(String::new());
            remaining = &remaining[separator_length..];
            continue;
        }
        let character = remaining
            .chars()
            .next()
            .expect("remaining input is nonempty");
        pieces.last_mut().expect("one input piece").push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    Ok(pieces
        .into_iter()
        .map(|piece| InputSegment {
            message_skip: piece.contains("\\e"),
            text: piece.replace("\\e", ""),
        })
        .collect())
}

fn append_repeated(output: &mut String, repeated: &str, count: usize) -> Result<(), &'static str> {
    let added = repeated
        .len()
        .checked_mul(count)
        .ok_or(EXPANSION_LIMIT_ERROR)?;
    ensure_size(
        output
            .len()
            .checked_add(added)
            .ok_or(EXPANSION_LIMIT_ERROR)?,
    )?;
    for _ in 0..count {
        output.push_str(repeated);
    }
    Ok(())
}

fn append_checked(output: &mut String, value: &str) -> Result<(), &'static str> {
    ensure_size(
        output
            .len()
            .checked_add(value.len())
            .ok_or(EXPANSION_LIMIT_ERROR)?,
    )?;
    output.push_str(value);
    Ok(())
}

fn append_character(output: &mut String, value: char) -> Result<(), &'static str> {
    ensure_size(
        output
            .len()
            .checked_add(value.len_utf8())
            .ok_or(EXPANSION_LIMIT_ERROR)?,
    )?;
    output.push(value);
    Ok(())
}

pub(crate) fn ensure_size(size: usize) -> Result<(), &'static str> {
    (size <= MAX_EXPANDED_BYTES)
        .then_some(())
        .ok_or(EXPANSION_LIMIT_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_splitting_matches_emuera_phase_order() {
        assert_segments("abc", &[("abc", false)]);
        assert_segments("a\\nb", &[("a", false), ("b", false)]);
        assert_segments("a\r\nb", &[("a", false), ("b", false)]);
        assert_segments("a\\rb", &[("a\\rb", false)]);
        assert_segments("\\\\nb", &[("\\", false), ("b", false)]);
        assert_segments("\\\\eb", &[("\\b", true)]);
        assert_segments("\\q", &[("\\q", false)]);
        assert_segments("a\\nb\\n", &[("a", false), ("b", false), ("", false)]);
    }

    #[test]
    fn input_repetitions_match_tolerant_reference_parser() {
        assert_segments("(ab)*2", &[("abab", false)]);
        assert_segments("(a(b)*2)*2", &[("abbabb", false)]);
        assert_segments("(ab", &[("ab", false)]);
        assert_segments("(ab)*", &[("", false)]);
        assert_segments("(ab)*0", &[("", false)]);
        assert_segments("(ab)*999999999999999999", &[("", false)]);
        assert_segments("\\(ab", &[("(ab", false)]);
        assert_segments("(a\\)b)*2", &[("a)ba)b", false)]);
        assert_segments("(a\\qb)", &[("aqb", false)]);
        assert_segments("(a\\", &[("a\0", false)]);
    }

    #[test]
    fn input_skip_markers_create_reference_boundaries() {
        assert_segments(
            "(412\\n\\e\\n)*2",
            &[
                ("412", false),
                ("", true),
                ("412", false),
                ("", true),
                ("", false),
            ],
        );
        assert_segments(
            "abc\\n(def)*2\\e",
            &[("abc", false), ("defdef", true), ("", false)],
        );
        assert_segments("(a\\eZ)", &[("a", true), ("Z", false)]);
        assert_segments("a\\eZ", &[("aZ", true)]);
        assert_segments("(\\r\\n)", &[("", false), ("", false)]);
    }

    #[test]
    fn input_expansion_enforces_portable_resource_limits() {
        let exact = "a".repeat(MAX_EXPANDED_BYTES);
        assert_eq!(
            preprocess_input(&exact).unwrap()[0].text.len(),
            MAX_EXPANDED_BYTES
        );
        assert_eq!(preprocess_input(&(exact + "a")), Err(EXPANSION_LIMIT_ERROR));
        assert_eq!(
            preprocess_input(&format!("(a)*{MAX_EXPANDED_BYTES}")).unwrap()[0]
                .text
                .len(),
            MAX_EXPANDED_BYTES
        );
        assert_eq!(
            preprocess_input(&format!("(a)*{}", MAX_EXPANDED_BYTES + 1)),
            Err(EXPANSION_LIMIT_ERROR)
        );
        let deep = format!(
            "{}x{}",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert_eq!(preprocess_input(&deep), Err(EXPANSION_LIMIT_ERROR));
        assert_eq!(
            preprocess_input(&"\\n".repeat(MAX_SEGMENTS)),
            Err(EXPANSION_LIMIT_ERROR)
        );
    }

    fn assert_segments(input: &str, expected: &[(&str, bool)]) {
        let expected = expected
            .iter()
            .map(|(text, message_skip)| InputSegment {
                text: (*text).to_owned(),
                message_skip: *message_skip,
            })
            .collect::<Vec<_>>();
        assert_eq!(preprocess_input(input), Ok(expected), "{input:?}");
    }
}
