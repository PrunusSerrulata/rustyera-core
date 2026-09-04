//! Canonical measurement-probe documents and inherited inline style.

use super::super::super::super::{
    HtmlAttribute, HtmlElementKind, HtmlElementSemantic, HtmlFontEdging, HtmlFontHinting, HtmlNode,
    HtmlTextRenderIntent, HtmlTextRenderer, HtmlVerticalAlignment,
};
use super::super::super::{HtmlMappedText, HtmlScalarBoundary};
use super::super::{
    HtmlDocument, HtmlQueryError, HtmlQueryErrorKind, HtmlSourceRange, input_error,
    invalid_measurement,
};

#[derive(Clone, Copy, Default)]
pub(super) struct Style<'a> {
    pub(super) flags: u8,
    pub(super) face: Option<&'a str>,
    pub(super) color: Option<u32>,
    pub(super) button_color: Option<u32>,
    pub(super) size_millipixels: Option<u32>,
    pub(super) vertical_alignment: Option<HtmlVerticalAlignment>,
    pub(super) render_intent: HtmlTextRenderIntent,
    pub(super) font_depth: usize,
}

pub(super) fn inline_style<'a>(
    kind: HtmlElementKind,
    semantic: &'a HtmlElementSemantic,
    style: Style<'a>,
) -> Result<Style<'a>, HtmlQueryError> {
    match semantic {
        HtmlElementSemantic::Style => {
            let bit = match kind {
                HtmlElementKind::Bold => 1,
                HtmlElementKind::Italic => 2,
                HtmlElementKind::Underline => 4,
                HtmlElementKind::Strike => 8,
                _ => return Err(invalid_measurement()),
            };
            if style.flags & bit != 0 {
                return Err(input_error(
                    HtmlQueryErrorKind::InvalidMarkup,
                    "duplicate inline style",
                ));
            }
            Ok(Style {
                flags: style.flags | bit,
                ..style
            })
        }
        HtmlElementSemantic::Font {
            face,
            color,
            button_color,
            size_millipixels,
            vertical_alignment,
            render_intent,
        } => Ok(Style {
            face: face.as_deref().or(style.face),
            color: color.or(style.color),
            button_color: button_color.or(style.button_color),
            size_millipixels: size_millipixels.or(style.size_millipixels),
            vertical_alignment: vertical_alignment.or(style.vertical_alignment),
            render_intent: HtmlTextRenderIntent {
                renderer: render_intent.renderer.or(style.render_intent.renderer),
                edging: render_intent.edging.or(style.render_intent.edging),
                hinting: render_intent.hinting.or(style.render_intent.hinting),
            },
            font_depth: style.font_depth + 1,
            ..style
        }),
        _ => Err(invalid_measurement()),
    }
}

pub(super) fn canonical_mapping(
    text: &str,
    path: &[usize],
    range: HtmlSourceRange,
) -> HtmlMappedText {
    let mut boundaries = vec![HtmlScalarBoundary {
        decoded_utf8: 0,
        decoded_utf16: 0,
        source_byte: 0,
    }];
    let mut utf16 = 0;
    for (offset, character) in text.char_indices() {
        utf16 += character.len_utf16();
        boundaries.push(HtmlScalarBoundary {
            decoded_utf8: offset + character.len_utf8(),
            decoded_utf16: utf16,
            source_byte: 0,
        });
    }
    HtmlMappedText {
        event_id: 0,
        node_path: path.into(),
        range,
        boundaries,
    }
}

pub(super) fn shallow_node(node: &HtmlNode) -> HtmlNode {
    match node {
        HtmlNode::Element {
            kind,
            attributes,
            start,
            end,
            semantic,
            ..
        } => {
            let mut attributes = attributes.clone();
            let mut semantic = semantic.clone();
            if let HtmlElementSemantic::Image { color_matrix, .. } = &mut semantic {
                // Color transforms do not affect geometry. Never expose a VM
                // variable address to the presentation measurement provider.
                *color_matrix = None;
                attributes.retain(|attribute| attribute.name != "cm");
            }
            HtmlNode::Element {
                kind: *kind,
                attributes,
                children: Vec::new(),
                interaction: None,
                start: *start,
                end: *end,
                semantic,
            }
        }
        HtmlNode::Text { .. } => node.clone(),
    }
}

pub(super) fn text_document(
    text: String,
    style: Style<'_>,
    range: HtmlSourceRange,
) -> (HtmlDocument, Vec<usize>) {
    let mut node = HtmlNode::Text {
        text,
        start: range.start as u64,
        end: range.end as u64,
    };
    let mut depth = 1;
    if style.face.is_some()
        || style.color.is_some()
        || style.button_color.is_some()
        || style.size_millipixels.is_some()
        || style.vertical_alignment.is_some()
        || style.render_intent != HtmlTextRenderIntent::default()
    {
        node = wrap(
            node,
            HtmlElementKind::Font,
            font_attributes(style),
            HtmlElementSemantic::Font {
                face: style.face.map(str::to_owned),
                color: style.color,
                button_color: style.button_color,
                size_millipixels: style.size_millipixels,
                vertical_alignment: style.vertical_alignment,
                render_intent: style.render_intent,
            },
            range,
        );
        depth += 1;
    }
    for (bit, kind) in [
        (1, HtmlElementKind::Bold),
        (2, HtmlElementKind::Italic),
        (4, HtmlElementKind::Underline),
        (8, HtmlElementKind::Strike),
    ] {
        if style.flags & bit != 0 {
            node = wrap(node, kind, Vec::new(), HtmlElementSemantic::Style, range);
            depth += 1;
        }
    }
    (HtmlDocument { nodes: vec![node] }, vec![0; depth])
}

fn font_attributes(style: Style<'_>) -> Vec<HtmlAttribute> {
    let mut attributes = Vec::new();
    let mut push = |name: &str, value: String| {
        attributes.push(HtmlAttribute {
            name: name.into(),
            value,
        });
    };
    if let Some(face) = style.face {
        push("face", face.into());
    }
    if let Some(color) = style.color {
        push("color", format!("#{color:06X}"));
    }
    if let Some(color) = style.button_color {
        push("bcolor", format!("#{color:06X}"));
    }
    if let Some(size) = style.size_millipixels {
        let mut value = format!("{}.{:03}", size / 1_000, size % 1_000);
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
        push("size", value);
    }
    if let Some(alignment) = style.vertical_alignment {
        push(
            "valign",
            match alignment {
                HtmlVerticalAlignment::Top => "top",
                HtmlVerticalAlignment::Middle => "middle",
                HtmlVerticalAlignment::Bottom => "bottom",
            }
            .into(),
        );
    }
    if let Some(renderer) = style.render_intent.renderer {
        push(
            "render",
            match renderer {
                HtmlTextRenderer::Gdi => "gdi",
                HtmlTextRenderer::Skia => "skia",
            }
            .into(),
        );
    }
    if let Some(edging) = style.render_intent.edging {
        push(
            "edging",
            match edging {
                HtmlFontEdging::Alias => "alias",
                HtmlFontEdging::AntiAlias => "antialias",
                HtmlFontEdging::SubpixelAntiAlias => "subpixel",
            }
            .into(),
        );
    }
    if let Some(hinting) = style.render_intent.hinting {
        push(
            "hinting",
            match hinting {
                HtmlFontHinting::None => "none",
                HtmlFontHinting::Slight => "slight",
                HtmlFontHinting::Normal => "normal",
                HtmlFontHinting::Full => "full",
            }
            .into(),
        );
    }
    attributes
}

fn wrap(
    child: HtmlNode,
    kind: HtmlElementKind,
    attributes: Vec<HtmlAttribute>,
    semantic: HtmlElementSemantic,
    range: HtmlSourceRange,
) -> HtmlNode {
    HtmlNode::Element {
        kind,
        attributes,
        children: vec![child],
        interaction: None,
        start: range.start as u64,
        end: range.end as u64,
        semantic,
    }
}
