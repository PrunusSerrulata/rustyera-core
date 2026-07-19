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
        matches!(self, Self::Break | Self::Image | Self::Shape)
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
    #[n(4)]
    pub generation: u64,
    #[n(5)]
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", content = "value", rename_all = "snake_case")]
pub enum HtmlLength {
    #[n(0)]
    Pixels(#[n(0)] i32),
    #[n(1)]
    FontHeightHundredths(#[n(0)] i32),
}

#[derive(Clone, Copy, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(index_only)]
#[serde(rename_all = "snake_case")]
pub enum HtmlAlignment {
    #[n(0)]
    Left,
    #[n(1)]
    Center,
    #[n(2)]
    Right,
}

#[derive(Clone, Debug, Default, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[cbor(map)]
pub struct HtmlBoxModel {
    #[n(0)]
    pub border: Option<[HtmlLength; 4]>,
    #[n(1)]
    pub radius: Option<[HtmlLength; 4]>,
    #[n(2)]
    pub margin: Option<[HtmlLength; 4]>,
    #[n(3)]
    pub padding: Option<[HtmlLength; 4]>,
    #[n(4)]
    pub border_colors: Option<[u32; 4]>,
}

/// Typed, renderer-independent meaning of every accepted Emuera tag.
#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HtmlElementSemantic {
    #[n(0)]
    Style,
    #[n(1)]
    Font {
        #[n(0)]
        face: Option<String>,
        #[n(1)]
        color: Option<u32>,
        #[n(2)]
        button_color: Option<u32>,
    },
    #[n(2)]
    Paragraph {
        #[n(0)]
        alignment: HtmlAlignment,
    },
    #[n(3)]
    NoBreak,
    #[n(4)]
    Button {
        #[n(0)]
        value: Option<String>,
        #[n(1)]
        title: Option<String>,
        #[n(2)]
        position: Option<i32>,
    },
    #[n(5)]
    NonButton {
        #[n(0)]
        title: Option<String>,
        #[n(1)]
        position: Option<i32>,
    },
    #[n(6)]
    ClearButton {
        #[n(0)]
        suppress_tooltip: bool,
    },
    #[n(7)]
    Image {
        #[n(0)]
        source: String,
        #[n(1)]
        hover_source: Option<String>,
        #[n(2)]
        mask_source: Option<String>,
        #[n(3)]
        height: Option<HtmlLength>,
        #[n(4)]
        width: Option<HtmlLength>,
        #[n(5)]
        y: Option<HtmlLength>,
    },
    #[n(8)]
    Shape {
        #[n(0)]
        kind: String,
        #[n(1)]
        parameters: Vec<HtmlLength>,
        #[n(2)]
        color: Option<u32>,
        #[n(3)]
        button_color: Option<u32>,
    },
    #[n(9)]
    Division {
        #[n(0)]
        x: Option<HtmlLength>,
        #[n(1)]
        y: Option<HtmlLength>,
        #[n(2)]
        width: HtmlLength,
        #[n(3)]
        height: HtmlLength,
        #[n(4)]
        depth: i32,
        #[n(5)]
        color: Option<u32>,
        #[n(6)]
        relative: bool,
        #[n(7)]
        box_model: HtmlBoxModel,
    },
    #[n(10)]
    Break,
}

#[derive(Clone, Debug, Decode, Encode, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// Keeping the normalized semantic value inline makes the public AST ergonomic
// and avoids exposing an allocation detail in the runtime protocol.
#[allow(clippy::large_enum_variant)]
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
        #[n(6)]
        semantic: HtmlElementSemantic,
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
    MissingAttribute,
    InvalidAttributeValue,
    InvalidNesting,
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
    semantic: HtmlElementSemantic,
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
            let Some(mut open) = stack.pop() else {
                return Err(error(HtmlErrorKind::UnexpectedClosingTag, cursor, end));
            };
            if kind == HtmlElementKind::Paragraph
                && open.kind == HtmlElementKind::NoBreak
                && stack
                    .last()
                    .is_some_and(|parent| parent.kind == HtmlElementKind::Paragraph)
            {
                // Emuera tracks <p> and <nobr> as independent line flags and explicitly
                // permits their trailing closers to be omitted. Real games consequently
                // close </p> while <nobr> is still active; normalize that form by closing
                // the inner no-break region at the paragraph boundary.
                let node = HtmlNode::Element {
                    kind: open.kind,
                    attributes: open.attributes,
                    children: open.children,
                    interaction: None,
                    start: open.start as u64,
                    end: cursor as u64,
                    semantic: open.semantic,
                };
                push_node(&mut roots, &mut stack, node);
                let Some(parent) = stack.pop() else {
                    return Err(error(HtmlErrorKind::MismatchedClosingTag, cursor, end));
                };
                open = parent;
            }
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
                semantic: open.semantic,
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
            let semantic = normalize_element(kind, &attributes, cursor, end)?;
            validate_nesting(kind, &stack, cursor, end)?;
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
                        semantic,
                    },
                );
            } else {
                stack.push(OpenElement {
                    kind,
                    attributes,
                    children: Vec::new(),
                    start: cursor,
                    semantic,
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
            semantic: open.semantic,
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

fn validate_nesting(
    kind: HtmlElementKind,
    stack: &[OpenElement],
    start: usize,
    end: usize,
) -> Result<(), HtmlError> {
    let inside_button = stack.iter().any(|item| {
        matches!(
            item.kind,
            HtmlElementKind::Button | HtmlElementKind::NonButton
        )
    });
    if inside_button && matches!(kind, HtmlElementKind::Button | HtmlElementKind::NonButton)
        || kind == HtmlElementKind::Division
            && stack
                .iter()
                .any(|item| item.kind == HtmlElementKind::Division)
        || kind == HtmlElementKind::ClearButton
            && stack
                .iter()
                .any(|item| item.kind == HtmlElementKind::ClearButton)
    {
        return Err(error(HtmlErrorKind::InvalidNesting, start, end));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn normalize_element(
    kind: HtmlElementKind,
    attributes: &[HtmlAttribute],
    start: usize,
    end: usize,
) -> Result<HtmlElementSemantic, HtmlError> {
    let invalid = || error(HtmlErrorKind::InvalidAttributeValue, start, end);
    let missing = || error(HtmlErrorKind::MissingAttribute, start, end);
    let value = |name: &str| {
        attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    };
    let allowed = |names: &[&str]| {
        attributes
            .iter()
            .all(|attribute| names.contains(&attribute.name.as_str()))
    };
    let no_attributes = || {
        if attributes.is_empty() {
            Ok(())
        } else {
            Err(error(HtmlErrorKind::InvalidAttribute, start, end))
        }
    };

    Ok(match kind {
        HtmlElementKind::Bold
        | HtmlElementKind::Italic
        | HtmlElementKind::Underline
        | HtmlElementKind::Strike => {
            no_attributes()?;
            HtmlElementSemantic::Style
        }
        HtmlElementKind::Break => {
            no_attributes()?;
            HtmlElementSemantic::Break
        }
        HtmlElementKind::NoBreak => {
            no_attributes()?;
            HtmlElementSemantic::NoBreak
        }
        HtmlElementKind::Paragraph => {
            if !allowed(&["align"]) || attributes.len() != 1 {
                return Err(missing());
            }
            let alignment = match value("align")
                .ok_or_else(missing)?
                .to_ascii_lowercase()
                .as_str()
            {
                "left" => HtmlAlignment::Left,
                "center" => HtmlAlignment::Center,
                "right" => HtmlAlignment::Right,
                _ => return Err(invalid()),
            };
            HtmlElementSemantic::Paragraph { alignment }
        }
        HtmlElementKind::Font => {
            if attributes.is_empty() || !allowed(&["face", "color", "bcolor"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Font {
                face: value("face").map(str::to_owned),
                color: value("color")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
                button_color: value("bcolor")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
            }
        }
        HtmlElementKind::Button => {
            if !allowed(&["value", "title", "pos"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Button {
                value: value("value").map(str::to_owned),
                title: value("title").map(str::to_owned),
                position: value("pos")
                    .map(str::parse)
                    .transpose()
                    .map_err(|_| invalid())?,
            }
        }
        HtmlElementKind::NonButton => {
            if !allowed(&["title", "pos"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::NonButton {
                title: value("title").map(str::to_owned),
                position: value("pos")
                    .map(str::parse)
                    .transpose()
                    .map_err(|_| invalid())?,
            }
        }
        HtmlElementKind::ClearButton => {
            if !allowed(&["notooltip"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            let suppress_tooltip = match value("notooltip") {
                None | Some("false" | "FALSE" | "False") => false,
                Some("true" | "TRUE" | "True") => true,
                Some(_) => return Err(invalid()),
            };
            HtmlElementSemantic::ClearButton { suppress_tooltip }
        }
        HtmlElementKind::Image => {
            if !allowed(&["src", "srcb", "srcm", "height", "width", "ypos"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            HtmlElementSemantic::Image {
                source: value("src").ok_or_else(missing)?.to_owned(),
                hover_source: value("srcb").map(str::to_owned),
                mask_source: value("srcm").map(str::to_owned),
                height: value("height")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                width: value("width")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
                y: value("ypos")
                    .map(parse_length)
                    .transpose()
                    .map_err(|()| invalid())?,
            }
        }
        HtmlElementKind::Shape => {
            if !allowed(&["type", "param", "color", "bcolor"]) {
                return Err(error(HtmlErrorKind::InvalidAttribute, start, end));
            }
            let parameters = value("param")
                .ok_or_else(missing)?
                .split(',')
                .map(|item| parse_length(item.trim()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|()| invalid())?;
            HtmlElementSemantic::Shape {
                kind: value("type").ok_or_else(missing)?.to_owned(),
                parameters,
                color: value("color")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
                button_color: value("bcolor")
                    .map(parse_color)
                    .transpose()
                    .map_err(|()| invalid())?,
            }
        }
        HtmlElementKind::Division => normalize_division(attributes, start, end)?,
    })
}

fn normalize_division(
    attributes: &[HtmlAttribute],
    start: usize,
    end: usize,
) -> Result<HtmlElementSemantic, HtmlError> {
    let invalid = || error(HtmlErrorKind::InvalidAttributeValue, start, end);
    let mut x = None;
    let mut y = None;
    let mut width = None;
    let mut height = None;
    let mut depth = 0;
    let mut color = None;
    let mut relative = true;
    let mut box_model = HtmlBoxModel::default();
    for attribute in attributes {
        match attribute.name.as_str() {
            "xpos" => x = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "ypos" => y = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "width" => width = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "height" => height = Some(parse_length(&attribute.value).map_err(|()| invalid())?),
            "depth" => depth = attribute.value.parse().map_err(|_| invalid())?,
            "color" => color = Some(parse_color(&attribute.value).map_err(|()| invalid())?),
            "display" => {
                relative = match attribute.value.to_ascii_lowercase().as_str() {
                    "relative" => true,
                    "absolute" => false,
                    _ => return Err(invalid()),
                };
            }
            "size" => {
                let values = parse_lengths::<2>(&attribute.value).map_err(|()| invalid())?;
                width = Some(values[0]);
                height = Some(values[1]);
            }
            "rect" => {
                let values = parse_lengths::<4>(&attribute.value).map_err(|()| invalid())?;
                x = Some(values[0]);
                y = Some(values[1]);
                width = Some(values[2]);
                height = Some(values[3]);
            }
            "border" => {
                box_model.border =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "radius" => {
                box_model.radius =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "margin" => {
                box_model.margin =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "padding" => {
                box_model.padding =
                    Some(parse_box_lengths(&attribute.value).map_err(|()| invalid())?);
            }
            "bcolor" => {
                box_model.border_colors =
                    Some(parse_box_colors(&attribute.value).map_err(|()| invalid())?);
            }
            _ => return Err(error(HtmlErrorKind::InvalidAttribute, start, end)),
        }
    }
    Ok(HtmlElementSemantic::Division {
        x,
        y,
        width: width.ok_or_else(|| error(HtmlErrorKind::MissingAttribute, start, end))?,
        height: height.ok_or_else(|| error(HtmlErrorKind::MissingAttribute, start, end))?,
        depth,
        color,
        relative,
        box_model,
    })
}

fn parse_length(value: &str) -> Result<HtmlLength, ()> {
    if let Some(value) = value
        .strip_suffix("px")
        .or_else(|| value.strip_suffix("PX"))
        .or_else(|| value.strip_suffix("Px"))
        .or_else(|| value.strip_suffix("pX"))
    {
        value.parse().map(HtmlLength::Pixels).map_err(|_| ())
    } else {
        value
            .parse()
            .map(HtmlLength::FontHeightHundredths)
            .map_err(|_| ())
    }
}

fn parse_lengths<const N: usize>(value: &str) -> Result<[HtmlLength; N], ()> {
    let values = value
        .split(',')
        .map(|item| parse_length(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    values.try_into().map_err(|_| ())
}

fn expand_four<T: Copy>(values: &[T]) -> Result<[T; 4], ()> {
    Ok(match values {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return Err(()),
    })
}

fn parse_box_lengths(value: &str) -> Result<[HtmlLength; 4], ()> {
    let values = value
        .split(',')
        .map(|item| parse_length(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    expand_four(&values)
}

fn parse_box_colors(value: &str) -> Result<[u32; 4], ()> {
    let values = value
        .split(',')
        .map(|item| parse_color(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    expand_four(&values)
}

fn parse_color(value: &str) -> Result<u32, ()> {
    if let Some(hex) = value.strip_prefix('#') {
        let color = u32::from_str_radix(hex, 16).map_err(|_| ())?;
        return (color <= 0x00ff_ffff).then_some(color).ok_or(());
    }
    let color = match value.to_ascii_lowercase().as_str() {
        "black" => 0x0000_0000,
        "white" => 0x00ff_ffff,
        "red" => 0x00ff_0000,
        "green" => 0x0000_8000,
        "blue" => 0x0000_00ff,
        "yellow" => 0x00ff_ff00,
        "gray" | "grey" => 0x0080_8080,
        "silver" => 0x00c0_c0c0,
        "maroon" => 0x0080_0000,
        "purple" => 0x0080_0080,
        "fuchsia" => 0x00ff_00ff,
        "lime" => 0x0000_ff00,
        "olive" => 0x0080_8000,
        "navy" => 0x0000_0080,
        "teal" => 0x0000_8080,
        "aqua" => 0x0000_ffff,
        _ => return Err(()),
    };
    Ok(color)
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
        let paragraph_closed_before_nobr =
            parse_document("<p align='right'><nobr><img src='clock' height='500' ypos='4'></p>")
                .unwrap();
        assert_eq!(
            serialize_document(&paragraph_closed_before_nobr),
            "<p align='right'><nobr><img src='clock' height='500' ypos='4'></nobr></p>"
        );
        let quoted = parse_document("<button value='a>b'>x</button>").unwrap();
        assert_eq!(
            serialize_document(&quoted),
            "<button value='a&gt;b'>x</button>"
        );
    }

    #[test]
    fn normalizes_mixed_lengths_box_model_and_button_values() {
        let document = parse_document(
            "<div rect='1px,2,30px,40' margin='1,2px' bcolor='#010203,red'><button value='42' pos='3'>x</button></div>",
        )
        .unwrap();
        let HtmlNode::Element {
            semantic, children, ..
        } = &document.nodes[0]
        else {
            panic!("expected div");
        };
        assert!(matches!(
            semantic,
            HtmlElementSemantic::Division {
                width: HtmlLength::Pixels(30),
                height: HtmlLength::FontHeightHundredths(40),
                ..
            }
        ));
        assert!(matches!(
            &children[0],
            HtmlNode::Element {
                semantic: HtmlElementSemantic::Button {
                    value: Some(value),
                    position: Some(3),
                    ..
                },
                ..
            } if value == "42"
        ));
    }

    #[test]
    fn rejects_reference_invalid_attributes_and_nesting() {
        assert!(matches!(
            parse_document("<img width='1'>"),
            Err(HtmlError {
                kind: HtmlErrorKind::MissingAttribute,
                ..
            })
        ));
        assert!(matches!(
            parse_document("<button><nonbutton>x</nonbutton></button>"),
            Err(HtmlError {
                kind: HtmlErrorKind::InvalidNesting,
                ..
            })
        ));
        assert!(parse_document("<clearbutton>x</clearbutton>").is_ok());
        assert!(parse_document("<clearbutton>").is_err());
    }
}
