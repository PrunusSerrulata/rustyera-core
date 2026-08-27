use super::super::{
    HtmlElementKind, HtmlError, HtmlErrorKind, HtmlNode, attributes, parse_document_inner,
};
use super::{
    HtmlDecodedSource, HtmlMappedDocument, HtmlMappedText, HtmlQueryEntityPolicy, HtmlQueryError,
    HtmlQueryErrorKind, HtmlQueryLimits, HtmlScalarBoundary, HtmlSourceEvent, HtmlSourceEventKind,
    HtmlSourceRange, check_document, check_source,
};

/// Decode entities and retain only exact source/scalar boundaries.
///
/// # Errors
/// Rejects invalid entities, lone surrogates, and exceeded source/scalar limits.
pub fn decode_query_entities(
    source: &str,
    policy: HtmlQueryEntityPolicy,
    limits: HtmlQueryLimits,
) -> Result<HtmlDecodedSource, HtmlQueryError> {
    check_source(source, limits)?;
    let mut text = String::with_capacity(source.len());
    let mut boundaries = vec![HtmlScalarBoundary {
        decoded_utf8: 0,
        decoded_utf16: 0,
        source_byte: 0,
    }];
    let (mut cursor, mut utf16) = (0, 0);
    while cursor < source.len() {
        let (unit, mut end) = decode_unit(source, cursor, policy)?;
        let character = match unit {
            Unit::Scalar(character) => character,
            Unit::Surrogate(high) if (0xd800..=0xdbff).contains(&high) => {
                if end == source.len() {
                    return Err(invalid_unicode(cursor, end));
                }
                let (low, pair_end) = decode_unit(source, end, policy)?;
                let Unit::Surrogate(low) = low else {
                    return Err(invalid_unicode(cursor, pair_end));
                };
                if !(0xdc00..=0xdfff).contains(&low) {
                    return Err(invalid_unicode(cursor, pair_end));
                }
                end = pair_end;
                char::from_u32(
                    0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00),
                )
                .ok_or_else(|| invalid_unicode(cursor, end))?
            }
            Unit::Surrogate(_) => return Err(invalid_unicode(cursor, end)),
        };
        text.push(character);
        utf16 += character.len_utf16();
        cursor = end;
        boundaries.push(HtmlScalarBoundary {
            decoded_utf8: text.len(),
            decoded_utf16: utf16,
            source_byte: cursor,
        });
        if boundaries.len() - 1 > limits.maximum_scalars {
            return Err(HtmlQueryError::new(
                HtmlQueryErrorKind::ResourceLimit,
                0,
                cursor,
                "decoded scalar count exceeds its limit",
            ));
        }
    }
    Ok(HtmlDecodedSource { text, boundaries })
}

enum Unit {
    Scalar(char),
    Surrogate(u16),
}

fn invalid_unicode(start: usize, end: usize) -> HtmlQueryError {
    HtmlQueryError::new(
        HtmlQueryErrorKind::InvalidUnicode,
        start,
        end,
        "entity produces an unpaired UTF-16 surrogate",
    )
}

fn decode_unit(
    source: &str,
    cursor: usize,
    policy: HtmlQueryEntityPolicy,
) -> Result<(Unit, usize), HtmlQueryError> {
    let invalid = |end| {
        HtmlQueryError::new(
            HtmlQueryErrorKind::InvalidEntity,
            cursor,
            end,
            "invalid character entity",
        )
    };
    let Some(character) = source[cursor..].chars().next() else {
        return Err(invalid_unicode(cursor, cursor));
    };
    if character != '&' {
        return Ok((Unit::Scalar(character), cursor + character.len_utf8()));
    }
    let end = source[cursor..]
        .find(';')
        .map(|end| cursor + end + 1)
        .ok_or_else(|| invalid(source.len()))?;
    let raw = &source[cursor..end];
    if policy == HtmlQueryEntityPolicy::Existing {
        let mut decoded = String::new();
        super::super::super::unescape_into(raw, &mut decoded).map_err(|_| invalid(end))?;
        return Ok((
            Unit::Scalar(decoded.chars().next().ok_or_else(|| invalid(end))?),
            end,
        ));
    }
    let name = source[cursor + 1..end - 1].to_ascii_lowercase();
    let named = match name.as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => None,
    };
    if let Some(character) = named {
        return Ok((Unit::Scalar(character), end));
    }
    let value = if let Some(value) = name.strip_prefix("#x") {
        u16::from_str_radix(value, 16).map_err(|_| invalid(end))?
    } else if let Some(value) = name.strip_prefix('#') {
        value.parse::<u16>().map_err(|_| invalid(end))?
    } else {
        return Err(invalid(end));
    };
    if (0xd800..=0xdfff).contains(&value) {
        Ok((Unit::Surrogate(value), end))
    } else {
        Ok((
            Unit::Scalar(char::from_u32(u32::from(value)).ok_or_else(|| invalid(end))?),
            end,
        ))
    }
}

pub(in crate::markup) fn decode_for_parser(source: &str, base: usize) -> Result<String, HtmlError> {
    decode_query_entities(
        source,
        HtmlQueryEntityPolicy::ReferenceQuery,
        HtmlQueryLimits::default(),
    )
    .map(|decoded| decoded.text)
    .map_err(|error| HtmlError {
        kind: HtmlErrorKind::InvalidEntity,
        start: base + error.range.start,
        end: base + error.range.end,
    })
}

/// Parse using the existing semantic parser, with optional query entity policy and source events.
///
/// # Errors
/// Returns ordinary parser errors or bounded source-map failures. Unknown tags are never accepted.
pub fn parse_document_with_source_map(
    source: &str,
    policy: HtmlQueryEntityPolicy,
    limits: HtmlQueryLimits,
) -> Result<HtmlMappedDocument, HtmlQueryError> {
    check_source(source, limits)?;
    // Preflight depth before building/dropping a nested public tree.
    let events = source_events(source, limits)?;
    let (document, _) =
        parse_document_inner(source, policy == HtmlQueryEntityPolicy::ReferenceQuery)
            .map_err(|error| HtmlQueryError::markup(&error))?;
    check_document(&document, limits)?;
    let mut texts = Vec::new();
    let mut pending = document
        .nodes
        .iter()
        .enumerate()
        .rev()
        .map(|(index, node)| (node, vec![index]))
        .collect::<Vec<_>>();
    while let Some((node, path)) = pending.pop() {
        match node {
            HtmlNode::Text { text, start, end } => {
                let start = usize::try_from(*start).map_err(|_| {
                    HtmlQueryError::new(
                        HtmlQueryErrorKind::InvalidMarkup,
                        0,
                        0,
                        "text span is not addressable",
                    )
                })?;
                let end = usize::try_from(*end).map_err(|_| {
                    HtmlQueryError::new(
                        HtmlQueryErrorKind::InvalidMarkup,
                        0,
                        0,
                        "text span is not addressable",
                    )
                })?;
                let decoded = decode_query_entities(&source[start..end], policy, limits)?;
                if &decoded.text != text {
                    return Err(HtmlQueryError::new(
                        HtmlQueryErrorKind::InvalidMarkup,
                        start,
                        end,
                        "text mapping differs from the canonical parser",
                    ));
                }
                let event = events
                    .iter()
                    .find(|event| {
                        event.range == HtmlSourceRange { start, end }
                            && event.kind == HtmlSourceEventKind::Text
                    })
                    .ok_or_else(|| {
                        HtmlQueryError::new(
                            HtmlQueryErrorKind::InvalidMarkup,
                            start,
                            end,
                            "text source event is missing",
                        )
                    })?;
                let boundaries = decoded
                    .boundaries
                    .into_iter()
                    .map(|mut boundary| {
                        boundary.source_byte += start;
                        boundary
                    })
                    .collect();
                texts.push(HtmlMappedText {
                    event_id: event.id,
                    node_path: path,
                    range: HtmlSourceRange { start, end },
                    boundaries,
                });
            }
            HtmlNode::Element { children, .. } => {
                for (index, child) in children.iter().enumerate().rev() {
                    let mut path = path.clone();
                    path.push(index);
                    pending.push((child, path));
                }
            }
        }
    }
    Ok(HtmlMappedDocument {
        document,
        events,
        texts,
    })
}

fn source_events(
    source: &str,
    limits: HtmlQueryLimits,
) -> Result<Vec<HtmlSourceEvent>, HtmlQueryError> {
    let mut events = Vec::new();
    let mut stack: Vec<(HtmlElementKind, usize)> = Vec::new();
    let (mut cursor, mut nodes) = (0, 0);
    while cursor < source.len() {
        let start = cursor;
        let kind = if source[cursor..].starts_with("<!--") {
            cursor = source[cursor + 4..]
                .find("-->")
                .map(|end| cursor + 4 + end + 3)
                .ok_or_else(|| {
                    HtmlQueryError::new(
                        HtmlQueryErrorKind::InvalidMarkup,
                        cursor,
                        source.len(),
                        "unterminated comment",
                    )
                })?;
            HtmlSourceEventKind::Comment
        } else if !source[cursor..].starts_with('<') {
            cursor = source[cursor..]
                .find('<')
                .map_or(source.len(), |end| cursor + end);
            nodes += 1;
            HtmlSourceEventKind::Text
        } else {
            cursor = attributes::find_tag_end(source, cursor).ok_or_else(|| {
                HtmlQueryError::new(
                    HtmlQueryErrorKind::InvalidMarkup,
                    cursor,
                    source.len(),
                    "unterminated tag",
                )
            })?;
            let raw = source[start + 1..cursor - 1].trim();
            if let Some(name) = raw.strip_prefix('/') {
                let kind = HtmlElementKind::parse(name.trim())
                    .ok_or_else(|| unsupported(start, cursor))?;
                close_source_element(kind, start, &mut stack, &mut events);
                HtmlSourceEventKind::Close { kind }
            } else {
                let self_closing = raw.ends_with('/');
                let name = raw
                    .strip_suffix('/')
                    .unwrap_or(raw)
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                let kind =
                    HtmlElementKind::parse(name).ok_or_else(|| unsupported(start, cursor))?;
                nodes += 1;
                if self_closing || kind.is_void() {
                    HtmlSourceEventKind::Void { kind }
                } else {
                    stack.push((kind, events.len()));
                    if stack.len() > limits.maximum_depth {
                        return Err(limit(start, cursor));
                    }
                    let name_start = start
                        + 1
                        + source[start + 1..cursor - 1]
                            .find(name)
                            .expect("tag name belongs to raw tag");
                    HtmlSourceEventKind::Open {
                        kind,
                        raw_name: HtmlSourceRange {
                            start: name_start,
                            end: name_start + name.len(),
                        },
                    }
                }
            }
        };
        if nodes > limits.maximum_nodes || events.len() >= limits.maximum_nodes.saturating_mul(3) {
            return Err(limit(start, cursor));
        }
        events.push(HtmlSourceEvent {
            id: events.len(),
            range: HtmlSourceRange { start, end: cursor },
            kind,
        });
    }
    for (_, opening_event) in stack.into_iter().rev() {
        events.push(HtmlSourceEvent {
            id: events.len(),
            range: HtmlSourceRange {
                start: source.len(),
                end: source.len(),
            },
            kind: HtmlSourceEventKind::ImplicitClose { opening_event },
        });
    }
    Ok(events)
}

fn close_source_element(
    kind: HtmlElementKind,
    start: usize,
    stack: &mut Vec<(HtmlElementKind, usize)>,
    events: &mut Vec<HtmlSourceEvent>,
) {
    if let Some(position) = stack.iter().rposition(|(open, _)| *open == kind) {
        if kind == HtmlElementKind::Paragraph
            && position + 2 == stack.len()
            && stack[position + 1].0 == HtmlElementKind::NoBreak
        {
            let (_, opening_event) = stack.pop().expect("known open nobr");
            events.push(HtmlSourceEvent {
                id: events.len(),
                range: HtmlSourceRange { start, end: start },
                kind: HtmlSourceEventKind::ImplicitClose { opening_event },
            });
        }
        stack.remove(position);
    }
}

fn unsupported(start: usize, end: usize) -> HtmlQueryError {
    HtmlQueryError::new(
        HtmlQueryErrorKind::UnsupportedTag,
        start,
        end,
        "tag is outside the existing HTML dialect",
    )
}

fn limit(start: usize, end: usize) -> HtmlQueryError {
    HtmlQueryError::new(
        HtmlQueryErrorKind::ResourceLimit,
        start,
        end,
        "HTML source event limit exceeded",
    )
}
