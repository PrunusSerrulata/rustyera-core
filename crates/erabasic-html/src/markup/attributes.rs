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
    parse_attributes_inner(source, base, false)
}

pub(super) fn parse_query_attributes(
    source: &str,
    base: usize,
) -> Result<Vec<HtmlAttribute>, HtmlError> {
    parse_attributes_inner(source, base, true)
}

fn parse_attributes_inner(
    source: &str,
    base: usize,
    query_entities: bool,
) -> Result<Vec<HtmlAttribute>, HtmlError> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        skip_whitespace(source, &mut cursor);
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
        skip_whitespace(source, &mut cursor);
        if name.is_empty() || !source[cursor..].starts_with('=') {
            return Err(error(
                HtmlErrorKind::InvalidAttribute,
                base + start,
                base + cursor,
            ));
        }
        cursor += 1;
        skip_whitespace(source, &mut cursor);
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
        let value = if query_entities {
            super::query::decode_for_parser(&source[value_start..cursor], base + value_start)?
        } else {
            decode_entities(&source[value_start..cursor], base + value_start)?
        };
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

fn skip_whitespace(source: &str, cursor: &mut usize) {
    while let Some(character) = source[*cursor..].chars().next() {
        if !character.is_whitespace() {
            break;
        }
        *cursor += character.len_utf8();
    }
}

pub(super) const fn error(kind: HtmlErrorKind, start: usize, end: usize) -> HtmlError {
    HtmlError {
        kind,
        start,
        end,
        origin: super::query::HtmlQueryErrorOrigin::ScriptInput,
    }
}
