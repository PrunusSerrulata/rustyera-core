//! Parser for the pinned Emuera HTML-like console dialect.

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlElementKind {
    #[n(0)]
    Bold,
    #[n(1)]
    Italic,
    #[n(2)]
    Underline,
    #[n(3)]
    Strike,
    #[n(4)]
    Font,
    #[n(5)]
    Paragraph,
    #[n(6)]
    NoBreak,
    #[n(7)]
    Button,
    #[n(8)]
    NonButton,
    #[n(9)]
    ClearButton,
    #[n(10)]
    Image,
    #[n(11)]
    Shape,
    #[n(12)]
    Division,
    #[n(13)]
    Break,
}

impl HtmlElementKind {
    fn parse(name: &str) -> Option<Self> {
        Some(match name.to_ascii_lowercase().as_str() {
            "b" => Self::Bold,
            "i" => Self::Italic,
            "u" => Self::Underline,
            "s" => Self::Strike,
            "font" => Self::Font,
            "p" => Self::Paragraph,
            "nobr" => Self::NoBreak,
            "button" => Self::Button,
            "nonbutton" => Self::NonButton,
            "clearbutton" => Self::ClearButton,
            "img" => Self::Image,
            "shape" => Self::Shape,
            "div" => Self::Division,
            "br" => Self::Break,
            _ => return None,
        })
    }

    const fn is_void(self) -> bool {
        matches!(
            self,
            Self::Break | Self::Image | Self::Shape | Self::ClearButton
        )
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strike => "s",
            Self::Font => "font",
            Self::Paragraph => "p",
            Self::NoBreak => "nobr",
            Self::Button => "button",
            Self::NonButton => "nonbutton",
            Self::ClearButton => "clearbutton",
            Self::Image => "img",
            Self::Shape => "shape",
            Self::Division => "div",
            Self::Break => "br",
        }
    }
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlAttribute {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub value: String,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlInteraction {
    #[n(0)]
    pub epoch: u64,
    #[n(1)]
    pub id: u64,
    #[n(2)]
    pub integer_value: Option<i64>,
    #[n(3)]
    pub string_value: Option<String>,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HtmlNode {
    #[n(0)]
    Text {
        #[n(0)]
        text: String,
        #[n(1)]
        start: u64,
        #[n(2)]
        end: u64,
    },
    #[n(1)]
    Element {
        #[n(0)]
        kind: HtmlElementKind,
        #[n(1)]
        attributes: Vec<HtmlAttribute>,
        #[n(2)]
        children: Vec<HtmlNode>,
        #[n(3)]
        interaction: Option<HtmlInteraction>,
        #[n(4)]
        start: u64,
        #[n(5)]
        end: u64,
    },
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlDocument {
    #[n(0)]
    pub nodes: Vec<HtmlNode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlErrorKind {
    UnterminatedTag,
    UnknownTag,
    UnexpectedClosingTag,
    MismatchedClosingTag,
    InvalidAttribute,
    DuplicateAttribute,
    InvalidEntity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HtmlError {
    pub kind: HtmlErrorKind,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
struct OpenElement {
    kind: HtmlElementKind,
    attributes: Vec<HtmlAttribute>,
    children: Vec<HtmlNode>,
    start: usize,
}

/// Parse and normalize the fixed markup dialect. All offsets are UTF-8 bytes.
///
/// # Errors
///
/// Returns an error with a UTF-8 byte range for malformed tags, attributes,
/// nesting, or entities.
#[allow(clippy::too_many_lines)]
pub fn parse_document(source: &str) -> Result<HtmlDocument, HtmlError> {
    let mut roots = Vec::new();
    let mut stack: Vec<OpenElement> = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source[cursor..].starts_with("<!--") {
            let Some(relative) = source[cursor + 4..].find("-->") else {
                return Err(error(HtmlErrorKind::UnterminatedTag, cursor, source.len()));
            };
            cursor += 4 + relative + 3;
            continue;
        }
        if !source[cursor..].starts_with('<') {
            let end = source[cursor..]
                .find('<')
                .map_or(source.len(), |at| cursor + at);
            let text = decode_entities(&source[cursor..end], cursor)?;
            push_node(
                &mut roots,
                &mut stack,
                HtmlNode::Text {
                    text,
                    start: cursor as u64,
                    end: end as u64,
                },
            );
            cursor = end;
            continue;
        }
        let Some(end) = find_tag_end(source, cursor) else {
            return Err(error(HtmlErrorKind::UnterminatedTag, cursor, source.len()));
        };
        let raw = source[cursor + 1..end - 1].trim();
        if let Some(closing) = raw.strip_prefix('/') {
            let name = closing.trim();
            let Some(kind) = HtmlElementKind::parse(name) else {
                return Err(error(HtmlErrorKind::UnknownTag, cursor, end));
            };
            let Some(open) = stack.pop() else {
                return Err(error(HtmlErrorKind::UnexpectedClosingTag, cursor, end));
            };
            if open.kind != kind {
                return Err(error(HtmlErrorKind::MismatchedClosingTag, cursor, end));
            }
            let node = HtmlNode::Element {
                kind,
                attributes: open.attributes,
                children: open.children,
                interaction: None,
                start: open.start as u64,
                end: end as u64,
            };
            push_node(&mut roots, &mut stack, node);
        } else {
            let self_closing = raw.ends_with('/');
            let raw = raw.strip_suffix('/').unwrap_or(raw).trim_end();
            let name_end = raw.find(char::is_whitespace).unwrap_or(raw.len());
            let name = &raw[..name_end];
            let Some(kind) = HtmlElementKind::parse(name) else {
                return Err(error(HtmlErrorKind::UnknownTag, cursor, end));
            };
            let attributes = parse_attributes(&raw[name_end..], cursor + 1 + name_end)?;
            if self_closing || kind.is_void() {
                push_node(
                    &mut roots,
                    &mut stack,
                    HtmlNode::Element {
                        kind,
                        attributes,
                        children: Vec::new(),
                        interaction: None,
                        start: cursor as u64,
                        end: end as u64,
                    },
                );
            } else {
                stack.push(OpenElement {
                    kind,
                    attributes,
                    children: Vec::new(),
                    start: cursor,
                });
            }
        }
        cursor = end;
    }
    while let Some(open) = stack.pop() {
        if !matches!(
            open.kind,
            HtmlElementKind::Paragraph | HtmlElementKind::NoBreak
        ) {
            return Err(error(
                HtmlErrorKind::MismatchedClosingTag,
                open.start,
                source.len(),
            ));
        }
        let node = HtmlNode::Element {
            kind: open.kind,
            attributes: open.attributes,
            children: open.children,
            interaction: None,
            start: open.start as u64,
            end: source.len() as u64,
        };
        push_node(&mut roots, &mut stack, node);
    }
    Ok(HtmlDocument { nodes: roots })
}

fn find_tag_end(source: &str, start: usize) -> Option<usize> {
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

fn push_node(roots: &mut Vec<HtmlNode>, stack: &mut [OpenElement], node: HtmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn parse_attributes(source: &str, base: usize) -> Result<Vec<HtmlAttribute>, HtmlError> {
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

fn decode_entities(source: &str, base: usize) -> Result<String, HtmlError> {
    let mut output = String::new();
    super::unescape_into(source, &mut output)
        .map_err(|_| error(HtmlErrorKind::InvalidEntity, base, base + source.len()))?;
    Ok(output)
}

const fn error(kind: HtmlErrorKind, start: usize, end: usize) -> HtmlError {
    HtmlError { kind, start, end }
}

#[must_use]
pub fn serialize_document(document: &HtmlDocument) -> String {
    fn node(output: &mut String, item: &HtmlNode) {
        match item {
            HtmlNode::Text { text, .. } => output.push_str(&super::escape(text)),
            HtmlNode::Element {
                kind,
                attributes,
                children,
                ..
            } => {
                output.push('<');
                output.push_str(kind.name());
                for attribute in attributes {
                    output.push(' ');
                    output.push_str(&attribute.name);
                    output.push_str("='");
                    output.push_str(&super::escape(&attribute.value));
                    output.push('\'');
                }
                output.push('>');
                for child in children {
                    node(output, child);
                }
                if !kind.is_void() {
                    output.push_str("</");
                    output.push_str(kind.name());
                    output.push('>');
                }
            }
        }
    }
    let mut output = String::new();
    for item in &document.nodes {
        node(&mut output, item);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_fixed_dialect_and_utf8_spans() {
        let document = parse_document("あ<b><button value='42'>x</button></b><br>").unwrap();
        assert_eq!(
            serialize_document(&document),
            "あ<b><button value='42'>x</button></b><br>"
        );
        assert!(matches!(
            parse_document("<unknown>"),
            Err(HtmlError {
                kind: HtmlErrorKind::UnknownTag,
                ..
            })
        ));

        let omitted = parse_document("<p align='center'><nobr>a>b").unwrap();
        assert_eq!(
            serialize_document(&omitted),
            "<p align='center'><nobr>a&gt;b</nobr></p>"
        );
        let quoted = parse_document("<button value='a>b'>x</button>").unwrap();
        assert_eq!(
            serialize_document(&quoted),
            "<button value='a&gt;b'>x</button>"
        );
    }
}
