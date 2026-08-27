//! Portable parsing utilities for Emuera's HTML-like console markup.
//!
//! Layout and rendering deliberately remain outside this crate. The parser
//! preserves a deterministic semantic stream independent of `WinForms` or a DOM.

use serde::{Deserialize, Serialize};

mod button;
mod color;
mod markup;

pub use button::{ButtonSegment, split_auto_buttons};
pub use color::named_color;
pub use markup::{
    HtmlAlignment, HtmlAttribute, HtmlBoxModel, HtmlDecodedSource, HtmlDocument, HtmlElementKind,
    HtmlElementSemantic, HtmlError, HtmlErrorKind, HtmlInteraction, HtmlLength, HtmlLengthCut,
    HtmlLengthImageResolution, HtmlLengthMeasuredValue, HtmlLengthMeasurement, HtmlLengthProbe,
    HtmlLengthProbeKind, HtmlLinesPoll, HtmlMappedDocument, HtmlMappedText, HtmlNode,
    HtmlOutputOrigin, HtmlOutputPiece, HtmlQueryEntityPolicy, HtmlQueryError, HtmlQueryErrorKind,
    HtmlQueryLimits, HtmlQueryProbe, HtmlQueryProbeKind, HtmlScalarBoundary, HtmlSourceEvent,
    HtmlSourceEventKind, HtmlSourceRange, HtmlStringLengthPlan, HtmlStringLengthPoll,
    HtmlStringLengthResult, HtmlStringLengthSettings, HtmlStringLinesPlan, HtmlSubstringPlan,
    HtmlSubstringPoll, HtmlSubstringResult, HtmlWarning, HtmlWarningKind, decode_query_entities,
    html_string_length_units, parse_document, parse_document_with_source_map,
    parse_document_with_warnings, serialize_document,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Token {
    Text(String),
    Tag(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    UnterminatedTag,
    InvalidEntity,
}

/// Split source into text and complete tag fragments.
///
/// # Errors
///
/// Returns [`Error::UnterminatedTag`] when an opening angle bracket has no close.
pub fn split_tags(source: &str) -> Result<Vec<Token>, Error> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        if start > cursor {
            result.push(Token::Text(source[cursor..start].to_owned()));
        }
        let Some(end_relative) = source[start..].find('>') else {
            return Err(Error::UnterminatedTag);
        };
        let end = start + end_relative + 1;
        result.push(Token::Tag(source[start..end].to_owned()));
        cursor = end;
    }
    if cursor < source.len() {
        result.push(Token::Text(source[cursor..].to_owned()));
    }
    Ok(result)
}

#[must_use]
pub fn escape(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    for character in source.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '>' => output.push_str("&gt;"),
            '<' => output.push_str("&lt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    output
}

/// Remove tags and decode the entity subset accepted by the reference runtime.
///
/// # Errors
///
/// Returns an error for an unterminated tag or malformed/unknown entity.
pub fn to_plain_text(source: &str) -> Result<String, Error> {
    let mut output = String::new();
    for token in split_tags(source)? {
        if let Token::Text(text) = token {
            unescape_into(&text, &mut output)?;
        }
    }
    Ok(output)
}

fn unescape_into(source: &str, output: &mut String) -> Result<(), Error> {
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find('&') {
        let start = cursor + relative;
        output.push_str(&source[cursor..start]);
        let Some(end_relative) = source[start..].find(';') else {
            return Err(Error::InvalidEntity);
        };
        let end = start + end_relative;
        let entity = &source[start + 1..end];
        match entity {
            "amp" => output.push('&'),
            "gt" => output.push('>'),
            "lt" => output.push('<'),
            "quot" => output.push('"'),
            "apos" | "#39" => output.push('\''),
            // The pinned reference normalizes nbsp to an ASCII space, not U+00A0.
            "nbsp" => output.push(' '),
            value if value.starts_with("#x") || value.starts_with("#X") => {
                let value =
                    u32::from_str_radix(&value[2..], 16).map_err(|_| Error::InvalidEntity)?;
                output.push(
                    char::from_u32(value)
                        .filter(|_| value <= 0xffff)
                        .ok_or(Error::InvalidEntity)?,
                );
            }
            value if value.starts_with('#') => {
                let value = value[1..]
                    .parse::<u32>()
                    .map_err(|_| Error::InvalidEntity)?;
                output.push(
                    char::from_u32(value)
                        .filter(|_| value <= 0xffff)
                        .ok_or(Error::InvalidEntity)?,
                );
            }
            _ => return Err(Error::InvalidEntity),
        }
        cursor = end + 1;
    }
    output.push_str(&source[cursor..]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_preserves_markup() {
        assert_eq!(
            split_tags("a<b>x</b>").unwrap(),
            vec![
                Token::Text("a".into()),
                Token::Tag("<b>".into()),
                Token::Text("x".into()),
                Token::Tag("</b>".into())
            ]
        );
        assert_eq!(split_tags("a<b"), Err(Error::UnterminatedTag));
    }

    #[test]
    fn escapes_and_flattens_reference_entities() {
        assert_eq!(escape("<&>'\""), "&lt;&amp;&gt;&apos;&quot;");
        assert_eq!(
            to_plain_text("<b>A&amp;B</b><br>&#x3042;").unwrap(),
            "A&Bあ"
        );
        assert_eq!(to_plain_text("a&nbsp;b").unwrap(), "a b");
        assert_eq!(to_plain_text("&#xD800;"), Err(Error::InvalidEntity));
    }
}
