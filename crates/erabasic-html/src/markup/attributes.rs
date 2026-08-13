//! Tag-boundary and attribute parsing with UTF-8 byte-offset diagnostics.

use super::{HtmlAttribute, HtmlError, HtmlErrorKind, decode_entities};

pub(super) fn find_tag_end(source: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (relative, character) in source[start + 1..].char_indices() {
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '>' => return Some(start + 1 + relative + 1),
            _ => {}
        }
    }
    None
}

pub(super) fn parse_attributes(source: &str, base: usize) -> Result<Vec<HtmlAttribute>, HtmlError> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        while cursor < source.len()
            && source[cursor..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        if cursor == source.len() {
            break;
        }
        let start = cursor;
        while cursor < source.len() {
            let c = source[cursor..].chars().next().unwrap();
            if c.is_whitespace() || c == '=' {
                break;
            }
            cursor += c.len_utf8();
        }
        let name = source[start..cursor].to_ascii_lowercase();
        while cursor < source.len()
            && source[cursor..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        if name.is_empty() || !source[cursor..].starts_with('=') {
            return Err(error(
                HtmlErrorKind::InvalidAttribute,
                base + start,
                base + cursor,
            ));
        }
        cursor += 1;
        while cursor < source.len()
            && source[cursor..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            cursor += source[cursor..].chars().next().unwrap().len_utf8();
        }
        let quote = source[cursor..]
            .chars()
            .next()
            .ok_or_else(|| error(HtmlErrorKind::InvalidAttribute, base + start, base + cursor))?;
        if quote != '\'' && quote != '"' {
            return Err(error(
                HtmlErrorKind::InvalidAttribute,
                base + cursor,
                base + cursor + quote.len_utf8(),
            ));
        }
        cursor += quote.len_utf8();
        let value_start = cursor;
        let Some(relative) = source[cursor..].find(quote) else {
            return Err(error(
                HtmlErrorKind::InvalidAttribute,
                base + value_start,
                base + source.len(),
            ));
        };
        cursor += relative;
        let value = decode_entities(&source[value_start..cursor], base + value_start)?;
        cursor += quote.len_utf8();
        if result
            .iter()
            .any(|attribute: &HtmlAttribute| attribute.name == name)
        {
            return Err(error(
                HtmlErrorKind::DuplicateAttribute,
                base + start,
                base + cursor,
            ));
        }
        result.push(HtmlAttribute { name, value });
    }
    Ok(result)
}

pub(super) const fn error(kind: HtmlErrorKind, start: usize, end: usize) -> HtmlError {
    HtmlError { kind, start, end }
}
