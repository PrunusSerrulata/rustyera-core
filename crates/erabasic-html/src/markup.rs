//! Parser for the pinned Emuera HTML-like console dialect.

mod model;

pub use model::{
    HtmlAlignment, HtmlAttribute, HtmlBoxModel, HtmlColorMatrix, HtmlDisplayMode, HtmlDocument,
    HtmlElementKind, HtmlElementSemantic, HtmlError, HtmlErrorKind, HtmlFontEdging,
    HtmlFontHinting, HtmlInteraction, HtmlLength, HtmlNode, HtmlTextRenderIntent, HtmlTextRenderer,
    HtmlVerticalAlignment, HtmlWarning, HtmlWarningKind,
};

mod attributes;
mod normalize;
mod query;
mod serialize;

pub use query::{
    HtmlDecodedSource, HtmlLengthCut, HtmlLengthImageResolution, HtmlLengthMeasuredValue,
    HtmlLengthMeasurement, HtmlLengthProbe, HtmlLengthProbeKind, HtmlLinesPoll, HtmlMappedDocument,
    HtmlMappedText, HtmlOutputOrigin, HtmlOutputPiece, HtmlQueryEntityPolicy, HtmlQueryError,
    HtmlQueryErrorKind, HtmlQueryErrorOrigin, HtmlQueryLimits, HtmlQueryProbe, HtmlQueryProbeKind,
    HtmlScalarBoundary, HtmlSourceEvent, HtmlSourceEventKind, HtmlSourceRange,
    HtmlStringLengthPlan, HtmlStringLengthPoll, HtmlStringLengthResult, HtmlStringLengthSettings,
    HtmlStringLinesPlan, HtmlSubstringPlan, HtmlSubstringPoll, HtmlSubstringResult,
    decode_query_entities, html_string_length_units, parse_document_with_source_map,
};

/// Locate the first AST node that uses a snake-only HTML extension.
///
/// Runtime entry points use this after parsing so the original profile keeps
/// its previous attribute surface while both profiles share one canonical AST.
#[must_use]
pub fn snake_extension_range(document: &HtmlDocument) -> Option<HtmlSourceRange> {
    fn node_extension_range(node: &HtmlNode) -> Option<HtmlSourceRange> {
        let HtmlNode::Element {
            attributes,
            children,
            semantic,
            start,
            end,
            ..
        } = node
        else {
            return None;
        };
        let extension = match semantic {
            HtmlElementSemantic::Font { .. } => attributes.iter().any(|attribute| {
                matches!(
                    attribute.name.as_str(),
                    "size" | "valign" | "render" | "edging" | "hinting"
                )
            }),
            HtmlElementSemantic::Image { .. } => attributes
                .iter()
                .any(|attribute| matches!(attribute.name.as_str(), "xpos" | "display" | "cm")),
            HtmlElementSemantic::Division { height, .. } => {
                height.is_none()
                    || attributes.iter().any(|attribute| {
                        attribute.name == "display"
                            && !matches!(
                                attribute.value.to_ascii_lowercase().as_str(),
                                "relative" | "absolute"
                            )
                    })
            }
            _ => false,
        };
        if extension {
            return Some(HtmlSourceRange {
                start: usize::try_from(*start).unwrap_or(usize::MAX),
                end: usize::try_from(*end).unwrap_or(usize::MAX),
            });
        }
        children.iter().find_map(node_extension_range)
    }

    document.nodes.iter().find_map(node_extension_range)
}

/// Reject snake-only attributes as a source-owned profile error.
///
/// This is shared by print and query entry points so profile routing cannot
/// diverge as new AST consumers are added.
///
/// # Errors
///
/// Returns `InvalidMarkup` with the source range of the first snake-only
/// attribute when the document is not valid for the reference profile.
pub fn reject_snake_extensions(document: &HtmlDocument) -> Result<(), HtmlQueryError> {
    let Some(range) = snake_extension_range(document) else {
        return Ok(());
    };
    Err(HtmlQueryError::input(
        HtmlQueryErrorKind::InvalidMarkup,
        range.start,
        range.end,
        "HTML attribute is unavailable for this compatibility profile",
    ))
}

use attributes::{error, find_tag_end, parse_attributes};
use normalize::{decode_entities, normalize_element};
pub use serialize::serialize_document;

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
pub fn parse_document(source: &str) -> Result<HtmlDocument, HtmlError> {
    parse_document_with_warnings(source).map(|(document, _)| document)
}

/// Parse and normalize the fixed markup dialect while retaining recoverable
/// compatibility warnings.
///
/// # Errors
///
/// Returns an error with a UTF-8 byte range for markup that cannot be
/// normalized without changing its meaning.
pub fn parse_document_with_warnings(
    source: &str,
) -> Result<(HtmlDocument, Vec<HtmlWarning>), HtmlError> {
    parse_document_inner(source, false)
}

#[allow(clippy::too_many_lines)]
fn parse_document_inner(
    source: &str,
    query_entities: bool,
) -> Result<(HtmlDocument, Vec<HtmlWarning>), HtmlError> {
    let mut roots = Vec::new();
    let mut stack: Vec<OpenElement> = Vec::new();
    let mut warnings = Vec::new();
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
            let text = if query_entities {
                query::decode_for_parser(&source[cursor..end], cursor)?
            } else {
                decode_entities(&source[cursor..end], cursor)?
            };
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
            let Some(position) = stack.iter().rposition(|open| open.kind == kind) else {
                return Err(error(HtmlErrorKind::UnexpectedClosingTag, cursor, end));
            };
            if kind == HtmlElementKind::Paragraph
                && position + 2 == stack.len()
                && stack[position + 1].kind == HtmlElementKind::NoBreak
            {
                let Some(no_break) = stack.pop() else {
                    return Err(error(HtmlErrorKind::MismatchedClosingTag, cursor, end));
                };
                let node = HtmlNode::Element {
                    kind: no_break.kind,
                    attributes: no_break.attributes,
                    children: no_break.children,
                    interaction: None,
                    start: no_break.start as u64,
                    end: cursor as u64,
                    semantic: no_break.semantic,
                };
                push_node(&mut roots, &mut stack, node);
                warnings.push(HtmlWarning {
                    kind: HtmlWarningKind::CrossedClosingTag,
                    start: cursor,
                    end,
                    closing: kind,
                    crossed: vec![HtmlElementKind::NoBreak],
                });
            }
            if position + 1 != stack.len() && can_reparent_crossed_scope(&stack, position) {
                let crossed = stack[position + 1..]
                    .iter()
                    .map(|open| open.kind)
                    .collect::<Vec<_>>();
                let mut open = stack.remove(position);
                let Some(active) = stack.last_mut() else {
                    return Err(error(HtmlErrorKind::MismatchedClosingTag, cursor, end));
                };
                open.children = std::mem::take(&mut active.children);
                // Emuera tracks paragraph, no-break, inline style, and button scopes
                // independently. The public model is a tree, so reparent a scope that
                // began immediately before its crossed
                // inner scopes. This preserves both style and one logical button without
                // accepting crossings that would require duplicating observable content.
                let node = HtmlNode::Element {
                    kind: open.kind,
                    attributes: open.attributes,
                    children: open.children,
                    interaction: None,
                    start: open.start as u64,
                    end: end as u64,
                    semantic: open.semantic,
                };
                active.children.push(node);
                warnings.push(HtmlWarning {
                    kind: HtmlWarningKind::CrossedClosingTag,
                    start: cursor,
                    end,
                    closing: kind,
                    crossed,
                });
                cursor = end;
                continue;
            }
            let Some(open) = stack.pop() else {
                return Err(error(HtmlErrorKind::MismatchedClosingTag, cursor, end));
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
            let attributes = if query_entities {
                attributes::parse_query_attributes(&raw[name_end..], cursor + 1 + name_end)?
            } else {
                parse_attributes(&raw[name_end..], cursor + 1 + name_end)?
            };
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
    Ok((HtmlDocument { nodes: roots }, warnings))
}

fn can_reparent_crossed_scope(stack: &[OpenElement], position: usize) -> bool {
    fn is_independent_inline(kind: HtmlElementKind) -> bool {
        matches!(
            kind,
            HtmlElementKind::Bold
                | HtmlElementKind::Italic
                | HtmlElementKind::Underline
                | HtmlElementKind::Strike
                | HtmlElementKind::Font
                | HtmlElementKind::Button
                | HtmlElementKind::NonButton
        )
    }

    is_independent_inline(stack[position].kind)
        && stack[position..]
            .iter()
            .all(|open| is_independent_inline(open.kind))
        && stack[position..stack.len() - 1]
            .iter()
            .all(|open| open.children.is_empty())
}

fn push_node(roots: &mut Vec<HtmlNode>, stack: &mut [OpenElement], node: HtmlNode) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
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
        let (paragraph_closed_before_nobr, paragraph_warnings) = parse_document_with_warnings(
            "<p align='right'><nobr><img src='clock' height='500' ypos='4'></p>",
        )
        .unwrap();
        assert_eq!(
            serialize_document(&paragraph_closed_before_nobr),
            "<p align='right'><nobr><img src='clock' height='500' ypos='4'></nobr></p>"
        );
        assert_eq!(paragraph_warnings.len(), 1);
        assert_eq!(paragraph_warnings[0].crossed, [HtmlElementKind::NoBreak]);
        let quoted = parse_document("<button value='a>b'>x</button>").unwrap();
        assert_eq!(
            serialize_document(&quoted),
            "<button value='a&gt;b'>x</button>"
        );
    }

    #[test]
    fn normalizes_erafl_title_tabs_with_crossed_font_and_button_closers() {
        // SHOW_INFO_SHOW_CHARA_TITLE.ERB:67-68 emits these two fragments after
        // CONV_TAG2H. Emuera closes font and button state independently.
        let source = "<font color='#EE7800'><button value='[MODE:TITLE_POINT]'>[称号点]　</font></button><font color='#C0C0C0'><button value='[MODE:TITLE_BONUS]'>[称号加成]　</font></button>";
        let (document, warnings) = parse_document_with_warnings(source).unwrap();

        assert_eq!(
            serialize_document(&document),
            "<button value='[MODE:TITLE_POINT]'><font color='#EE7800'>[称号点]　</font></button><button value='[MODE:TITLE_BONUS]'><font color='#C0C0C0'>[称号加成]　</font></button>"
        );
        assert_eq!(warnings.len(), 2);
        let first_close = source.find("</font>").unwrap();
        let second_close = source.rfind("</font>").unwrap();
        assert_eq!(
            warnings[0],
            HtmlWarning {
                kind: HtmlWarningKind::CrossedClosingTag,
                start: first_close,
                end: first_close + "</font>".len(),
                closing: HtmlElementKind::Font,
                crossed: vec![HtmlElementKind::Button],
            }
        );
        assert_eq!(warnings[1].closing, HtmlElementKind::Font);
        assert_eq!(warnings[1].crossed, [HtmlElementKind::Button]);
        assert_eq!(warnings[1].start, second_close);
        assert_eq!(warnings[1].end, second_close + "</font>".len());
        assert!(second_close > source[..second_close].chars().count());

        let HtmlNode::Element {
            kind: HtmlElementKind::Button,
            children,
            ..
        } = &document.nodes[0]
        else {
            panic!("expected normalized button root");
        };
        let HtmlNode::Element {
            kind: HtmlElementKind::Font,
            start,
            end,
            ..
        } = &children[0]
        else {
            panic!("expected normalized font child");
        };
        assert_eq!((*start, *end), (0, (first_close + "</font>".len()) as u64));
        let HtmlNode::Element {
            kind: HtmlElementKind::Button,
            children,
            ..
        } = &document.nodes[1]
        else {
            panic!("expected second normalized button root");
        };
        let HtmlNode::Element {
            kind: HtmlElementKind::Font,
            end,
            ..
        } = &children[0]
        else {
            panic!("expected second normalized font child");
        };
        assert_eq!(*end, (second_close + "</font>".len()) as u64);

        let nested = serialize_document(&document);
        let (_, nested_warnings) = parse_document_with_warnings(&nested).unwrap();
        assert!(nested_warnings.is_empty());
        assert!(matches!(
            parse_document("<font color='#EE7800'>prefix<button value='1'>x</font>y</button>"),
            Err(HtmlError {
                kind: HtmlErrorKind::MismatchedClosingTag,
                ..
            })
        ));
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
                height: Some(HtmlLength::FontHeightHundredths(40)),
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
    fn normalizes_snake_font_image_and_positioned_division_intents() {
        let document = parse_document(
            "<font size='12.5px' valign='middle' render='skia' edging='subpixel' hinting='full'>x</font><img src='face' xpos='3px' ypos='4' width='5px' height='6' display='absolute-leftbottom' cm='MATRIX:1'><div width='80px' display='absolute-lefttop' padding='1px,2px' radius='3px' />",
        )
        .unwrap();
        let HtmlNode::Element {
            semantic:
                HtmlElementSemantic::Font {
                    size_millipixels,
                    vertical_alignment,
                    render_intent,
                    ..
                },
            ..
        } = &document.nodes[0]
        else {
            panic!("expected font");
        };
        assert_eq!(*size_millipixels, Some(12_500));
        assert_eq!(*vertical_alignment, Some(HtmlVerticalAlignment::Middle));
        assert_eq!(render_intent.renderer, Some(HtmlTextRenderer::Skia));
        assert_eq!(
            render_intent.edging,
            Some(HtmlFontEdging::SubpixelAntiAlias)
        );
        assert_eq!(render_intent.hinting, Some(HtmlFontHinting::Full));

        assert!(matches!(
            &document.nodes[1],
            HtmlNode::Element {
                semantic: HtmlElementSemantic::Image {
                    x: Some(HtmlLength::Pixels(3)),
                    y: Some(HtmlLength::FontHeightHundredths(4)),
                    display: HtmlDisplayMode::AbsoluteLeftBottom,
                    color_matrix: Some(HtmlColorMatrix::Variable { name, indices }),
                    ..
                },
                ..
            } if name == "MATRIX" && *indices == [1, 0, 0]
        ));
        assert!(matches!(
            &document.nodes[2],
            HtmlNode::Element {
                semantic: HtmlElementSemantic::Division {
                    height: None,
                    display: HtmlDisplayMode::AbsoluteLeftTop,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn normalizes_prefixed_hex_colors() {
        let source = "<font color='#0x90EE90'>a</font><div width='1px' height='1px' bcolor='#0X010203,red'>b</div>";
        let document = parse_document(source).unwrap();

        assert!(matches!(
            &document.nodes[0],
            HtmlNode::Element {
                semantic: HtmlElementSemantic::Font {
                    color: Some(0x0090_ee90),
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            &document.nodes[1],
            HtmlNode::Element {
                semantic: HtmlElementSemantic::Division { box_model, .. },
                ..
            } if box_model.border_colors
                == Some([0x0001_0203, 0x00ff_0000, 0x0001_0203, 0x00ff_0000])
        ));
        assert_eq!(snake_extension_range(&document), None);
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
        assert!(matches!(
            parse_document("<img src='x' arbitrary='raw'>"),
            Err(HtmlError {
                kind: HtmlErrorKind::InvalidAttribute,
                ..
            })
        ));
    }
}
